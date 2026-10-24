//! The import worker (U4 of the audio import plan): one audio file becomes
//! a queued note, crash-safe.
//!
//! Each import has an entry in a local journal, a file in the app data
//! folder ([`JOURNAL_FILE`]), written before any other step (KTD6). The
//! journal records each step before the next step starts (step, then
//! record):
//!
//! 1. [`Step::Journaled`]: the entry exists. The step makes the note row
//!    in `Importing` with its title, type, tags and recording time, and
//!    begins the early write of its home folder in the same store lock.
//! 2. [`Step::Created`]: the step checks the free space, then copies the
//!    file into the working folder of the note and hashes it (KTD3).
//! 3. [`Step::Copied`]: the step decodes the copy into the chunks of a
//!    normal recording (KTD4). It pauses while a recording runs (KTD9).
//! 4. [`Step::Converted`]: the step deletes the copy and writes the
//!    `imported` entry of the import record. For a file from the watched
//!    folder, it first reads the import record again: when another
//!    computer imported the file meanwhile, the note goes
//!    ([`Step::Duplicate`]).
//! 5. [`Step::Recorded`]: the step puts the segment row and queues the
//!    note in one store lock. Then the journal entry goes.
//!
//! Each step recognises its own effect when a crash lost its record: a row
//! that exists is not made again, a copy and a conversion are made again
//! from the start, and the `imported` entry of this computer with the time
//! that the journal holds is this import's own entry.
//!
//! A decoder that refuses the complete copy gives [`Step::Failed`]: the
//! `failed` entry is written and the note goes (R12a).
//!
//! A user delete of an importing note marks the journal entry deleted
//! ([`Imports::delete`]). The worker stops the conversion, removes its
//! working folder, hashes the file when the hash is not known, writes the
//! `imported` entry and only then removes the journal entry, so the file
//! does not come back (R12).
//!
//! Every file read, hash and conversion runs on the caller's thread or the
//! worker thread, never under the store lock.

use std::collections::{BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Local};
use import_record::{Identity, Lookup, State};
use recorder::{ChunkedWriter, SegmentArgs};
use serde::{Deserialize, Serialize};
use session_core::jobs::{Claims, NewImport};
use session_core::note_record::ImportSource;
use session_core::{RecordingModel, Segment, SessionState, SpeakerCount, Title, TitleSource};
use session_save::home_write;

use crate::early_home::{self, EarlyHomes};
use crate::{RECOVERY_DIR, SharedStore, StageError, lock};

/// The id of this computer in the import record.
pub use import_record::ComputerId;

/// The name of the journal file in the app data folder, next to
/// `settings.json`.
pub const JOURNAL_FILE: &str = "import-journal.json";

/// The version of the journal file that this build writes.
const JOURNAL_VERSION: u32 = 1;

/// The folder of the copy in the working folder of the note.
const COPY_DIR: &str = "import";

/// How long the worker thread waits before it tries an import that waits
/// again, when no scan or launch asks first.
const RETRY_EVERY: Duration = Duration::from_secs(60);

/// The lowest bit rate that the disk estimate expects of a compressed file,
/// in bytes per second (32 kbit/s).
const COMPRESSED_BYTES_PER_SECOND: u64 = 4_000;

/// The lowest bit rate that the disk estimate expects of a wav file, in
/// bytes per second (8 kHz, 16-bit, mono).
const WAV_BYTES_PER_SECOND: u64 = 16_000;

/// The sample rate that the disk estimate expects of the chunks.
const ESTIMATE_RATE: u32 = 48_000;

/// Where the import came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The user dropped or chose the file (R1). The import record does not
    /// stop it (R3).
    Manual,
    /// The watched folder (R5). The worker reads the import record again
    /// before it writes its entry (R13).
    WatchedFolder,
}

/// What the caller asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The audio file. The worker reads it and never changes it (R7).
    pub source: PathBuf,
    pub origin: Origin,
    /// `None` is General.
    pub note_type: Option<String>,
    pub tags: Vec<String>,
    /// The title that the user set in the import sheet (R2a). `None` gives
    /// the file name as a proposed title (R15).
    pub title: Option<String>,
}

/// The recognition that the note records at its start, as a recording
/// start records it. An import has no live transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recognition {
    pub asr_model: String,
    pub snapshot: Option<serde_json::Value>,
    pub speaker_count: Option<SpeakerCount>,
}

/// The last step that the journal records for an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// The entry exists. The note row can exist.
    Journaled,
    /// The note row is `Importing`, and its early write has begun.
    Created,
    /// The copy of the file is complete in the working folder.
    Copied { identity: String, size: u64 },
    /// The chunks are complete. `entry_time` is the time of the `imported`
    /// entry that the next step writes.
    Converted {
        identity: String,
        size: u64,
        duration_ms: u64,
        entry_time: DateTime<FixedOffset>,
    },
    /// The import record has the `imported` entry.
    Recorded {
        identity: String,
        size: u64,
        duration_ms: u64,
    },
    /// The decoder refused the complete copy (R12a). `at` is the time of
    /// the `failed` entry.
    Failed {
        identity: String,
        size: u64,
        at: DateTime<FixedOffset>,
        error: String,
    },
    /// Another computer imported the file during the conversion.
    Duplicate,
}

impl Step {
    /// The identity and the size, when the copy is complete.
    fn identity(&self) -> Option<(&str, u64)> {
        match self {
            Step::Copied { identity, size }
            | Step::Converted { identity, size, .. }
            | Step::Recorded { identity, size, .. }
            | Step::Failed { identity, size, .. } => Some((identity, *size)),
            Step::Journaled | Step::Created | Step::Duplicate => None,
        }
    }
}

/// One import in the journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub note_id: String,
    pub request: Request,
    /// The recording time of the file (R16): the start of the note.
    pub started: DateTime<FixedOffset>,
    pub recognition: Recognition,
    pub step: Step,
    /// The user deleted the note. The worker records the file and makes no
    /// note.
    #[serde(default)]
    pub deleted: bool,
}

/// What a step gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// The step is done: record this step.
    Step(Step),
    /// The import is done: remove the journal entry.
    Done,
    /// The import waits. A retry, a scan or a launch tries it again.
    Wait(Wait),
}

/// Why an import waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// The free space is too low for the copy and the conversion (R22).
    Disk,
    /// No storage folder is set, so the import record cannot be written.
    StorageFolder,
    /// An entry of the import record cannot be read or written now.
    ImportRecord,
    /// The source file cannot be read now (KTD11).
    Source,
    /// The app stops.
    Stopped,
}

/// A file that the decoder refused (R12a), for the notice of U8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub note_id: String,
    pub identity: String,
    pub file_name: String,
    pub at: DateTime<FixedOffset>,
}

/// The facts that the notices of U8 need.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notices {
    /// The notes whose import waits for free space (R22). Only the user can
    /// free space.
    pub disk: Vec<String>,
    /// The files that the decoder refused since the launch.
    pub failed: Vec<Failure>,
}

/// What the worker tells the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportEvent {
    /// The note row exists: Home shows it (R21).
    Created { note_id: String },
    /// The conversion waits while a recording runs (R18).
    Paused { note_id: String },
    /// The conversion continues after the recording.
    Continued { note_id: String },
    /// The import waits for free space (R22).
    DiskWait { note_id: String },
    /// The import no longer waits for free space.
    DiskClear { note_id: String },
    /// The decoder refused a file (R12a).
    Failed(Failure),
    /// The note is queued: processing can start.
    Queued { note_id: String },
    /// The note is gone: a duplicate or a failure.
    Removed { note_id: String },
    /// The journal entry is gone: the import ended.
    Ended { note_id: String },
}

/// The free space of the volume of a folder, or `None` when unknown.
pub type FreeSpace = Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

/// Remove a folder: to the Trash or permanently. The error is the cause.
pub type RemoveFolder = Box<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

/// What the worker needs from the app.
pub struct ImportEnvironment {
    pub store: SharedStore,
    /// The folder that holds the working folder of each note.
    pub working_root: PathBuf,
    /// The journal file ([`JOURNAL_FILE`] in the app data folder).
    pub journal: PathBuf,
    /// The computer id that names this computer's entry files.
    pub computer_id: ComputerId,
    /// The Longhand version that a `failed` entry records (KTD11).
    pub longhand_version: String,
    /// The storage folder now.
    pub storage_folder: Box<dyn Fn() -> Option<PathBuf> + Send + Sync>,
    /// True while a recording runs or the live worker drains: the
    /// conversion waits (KTD9). The caller must not hold the store lock.
    pub suspended: Box<dyn Fn() -> bool + Send + Sync>,
    /// The free space of the volume of a folder.
    pub free_space: FreeSpace,
    /// Remove a folder of a note that goes: to the Trash or permanently.
    pub remove: RemoveFolder,
    /// The processing claims, which a removal of a note takes.
    pub claims: Claims,
    /// The jobs of the early writes. `None` writes no home folder.
    pub early_homes: Option<Arc<EarlyHomes>>,
    pub events: Box<dyn Fn(ImportEvent) + Send + Sync>,
    /// The wait between two reads of `suspended` while a recording runs.
    pub poll: Duration,
}

/// The journal file. Every change writes the whole file atomically.
#[derive(Debug, Default, Serialize, Deserialize)]
struct JournalFile {
    version: u32,
    entries: Vec<Entry>,
}

/// The work of the worker thread.
#[derive(Default)]
struct Work {
    queued: VecDeque<String>,
    /// The imports that wait for a retry.
    waiting: BTreeSet<String>,
    stopped: bool,
}

/// The imports of this computer.
pub struct Imports {
    env: ImportEnvironment,
    /// The entries of the journal file. The lock is held across each write
    /// of the file, and across the queue step of an entry, so a delete and
    /// the queue step exclude each other. Lock order: this lock, then the
    /// store.
    journal: Mutex<Vec<Entry>>,
    work: Mutex<Work>,
    signal: Condvar,
    /// Held while entries run, so only one thread runs them.
    running: Mutex<()>,
    notices: Mutex<Notices>,
    stopped: AtomicBool,
}

fn held<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Imports {
    /// Read the journal. A missing file is an empty journal. A file that
    /// does not parse gives an error and is never overwritten.
    pub fn open(env: ImportEnvironment) -> io::Result<Arc<Self>> {
        let entries = match std::fs::read(&env.journal) {
            Ok(bytes) => {
                serde_json::from_slice::<JournalFile>(&bytes)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
                    .entries
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        Ok(Arc::new(Self {
            env,
            journal: Mutex::new(entries),
            work: Mutex::default(),
            signal: Condvar::new(),
            running: Mutex::new(()),
            notices: Mutex::default(),
            stopped: AtomicBool::new(false),
        }))
    }

    /// Every entry of the journal.
    pub fn entries(&self) -> Vec<Entry> {
        held(&self.journal).clone()
    }

    /// The source files of the open imports. The watched folder does not
    /// import such a file again, also while its note is deleted (KTD6).
    pub fn open_sources(&self) -> Vec<PathBuf> {
        held(&self.journal)
            .iter()
            .map(|e| e.request.source.clone())
            .collect()
    }

    /// The facts that the notices of U8 need.
    pub fn notices(&self) -> Notices {
        held(&self.notices).clone()
    }

    /// Step 1: write the journal entry of a new import. The caller runs it
    /// off the main thread: it reads the recording time of the file (R16).
    pub fn begin(&self, request: Request, recognition: Recognition) -> Result<Entry, StageError> {
        let started = recording_start(&request.source);
        let entry = Entry {
            note_id: uuid::Uuid::new_v4().to_string(),
            request,
            started,
            recognition,
            step: Step::Journaled,
            deleted: false,
        };
        let mut journal = held(&self.journal);
        journal.push(entry.clone());
        if let Err(e) = self.save(&journal) {
            journal.pop();
            return Err(e.into());
        }
        Ok(entry)
    }

    /// Import `request`: write the journal entry, make the note row so the
    /// note shows on Home at once (R21), and give the rest to the worker.
    /// Returns the note id. The caller runs it off the main thread.
    pub fn import(&self, request: Request, recognition: Recognition) -> Result<String, StageError> {
        let entry = self.begin(request, recognition)?;
        let id = entry.note_id.clone();
        let created = self.step(&entry).and_then(|next| match next {
            Next::Step(step) => Ok(self.record(&id, step)?),
            _ => Ok(()),
        });
        // The journal holds the import, so the worker makes the row later
        // when this fails.
        if let Err(e) = created {
            log::error!("could not make the row of the import {id}: {e}");
        }
        self.queue(&id);
        Ok(id)
    }

    /// Mark the import of the note `id` deleted, before the caller deletes
    /// the note. Returns false when the note has no open import, or when
    /// the import has queued the note: the caller then deletes as for any
    /// note.
    pub fn delete(&self, id: &str) -> bool {
        let mut journal = held(&self.journal);
        let Some(i) = journal.iter().position(|e| e.note_id == id) else {
            return false;
        };
        if matches!(journal[i].step, Step::Recorded { .. }) && !journal[i].deleted {
            // The queue step can have run: the note is a normal note now.
            let queued = lock(&self.env.store)
                .session(id)
                .ok()
                .flatten()
                .is_some_and(|r| r.state != SessionState::Importing);
            if queued {
                return false;
            }
        }
        if !journal[i].deleted {
            journal[i].deleted = true;
            if let Err(e) = self.save(&journal) {
                log::error!("could not mark the import of {id} deleted: {e}");
                journal[i].deleted = false;
                return false;
            }
        }
        drop(journal);
        self.queue(id);
        true
    }

    /// At launch: remove each importing note that has no journal entry
    /// (an import that the journal does not know cannot continue), then
    /// queue every entry.
    pub fn launch(&self) {
        let known: Vec<String> = self.entries().into_iter().map(|e| e.note_id).collect();
        let orphans = match lock(&self.env.store).sessions_in(SessionState::Importing.name()) {
            Ok(records) => records,
            Err(e) => {
                log::error!("could not list the importing notes: {e}");
                Vec::new()
            }
        };
        for record in orphans.into_iter().filter(|r| !known.contains(&r.id)) {
            log::warn!(
                "the importing note {} has no journal entry; it is removed",
                record.id
            );
            if let Err(e) = self.remove_note(&record.id) {
                log::error!("could not remove the importing note {}: {e}", record.id);
            }
        }
        for id in known {
            self.queue(&id);
        }
    }

    /// Try each import that waits again: free space can be back, or the
    /// storage folder. A scan of the watched folder calls it too (KTD9).
    pub fn retry(&self) {
        let mut work = held(&self.work);
        let waiting = std::mem::take(&mut work.waiting);
        for id in waiting {
            if !work.queued.contains(&id) {
                work.queued.push_back(id);
            }
        }
        self.signal.notify_all();
    }

    /// Run the queued imports on this thread until none is queued.
    pub fn run_pending(&self) {
        let _running = held(&self.running);
        loop {
            let next = {
                let mut work = held(&self.work);
                if work.stopped {
                    return;
                }
                work.queued.pop_front()
            };
            let Some(id) = next else {
                return;
            };
            self.run_entry(&id);
        }
    }

    /// Run the imports on a thread of their own until [`Imports::stop`].
    pub fn start(self: &Arc<Self>) {
        let imports = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("import".into())
            .spawn(move || {
                loop {
                    imports.run_pending();
                    let mut work = held(&imports.work);
                    if work.stopped {
                        return;
                    }
                    if work.queued.is_empty() {
                        let (next, wait) = imports
                            .signal
                            .wait_timeout(work, RETRY_EVERY)
                            .unwrap_or_else(|e| e.into_inner());
                        work = next;
                        if wait.timed_out() && !work.waiting.is_empty() {
                            drop(work);
                            imports.retry();
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            log::error!("could not start the import worker: {e}");
        }
    }

    /// Before the process exits: the conversion in progress stops, and the
    /// worker thread ends. The imports continue at the next launch.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        held(&self.work).stopped = true;
        self.signal.notify_all();
    }

    /// Record `step` as the last step of the import of `id`. A delete mark
    /// stays.
    pub fn record(&self, id: &str, step: Step) -> io::Result<()> {
        let mut journal = held(&self.journal);
        let Some(entry) = journal.iter_mut().find(|e| e.note_id == id) else {
            return Ok(());
        };
        let previous = std::mem::replace(&mut entry.step, step);
        if let Err(e) = self.save(&journal) {
            if let Some(entry) = journal.iter_mut().find(|e| e.note_id == id) {
                entry.step = previous;
            }
            return Err(e);
        }
        Ok(())
    }

    /// Take the next step of `entry` (see the module documentation). The
    /// caller records the step that it gives before the next step. The
    /// worker and the tests call it.
    pub fn step(&self, entry: &Entry) -> Result<Next, StageError> {
        if entry.deleted {
            return self.finish_deleted(entry);
        }
        let id = entry.note_id.as_str();
        // A row that is gone with no delete (a rebuilt store) starts the
        // import again from its row.
        let needs_row = matches!(
            entry.step,
            Step::Created | Step::Copied { .. } | Step::Converted { .. }
        );
        if needs_row && !self.importing(id)? {
            return Ok(Next::Step(Step::Journaled));
        }
        match &entry.step {
            Step::Journaled => self.create(entry),
            Step::Created => self.copy(entry),
            Step::Copied { identity, size } => self.convert(entry, identity, *size),
            Step::Converted {
                identity,
                size,
                duration_ms,
                entry_time,
            } => self.record_import(entry, identity, *size, *duration_ms, *entry_time),
            Step::Recorded { duration_ms, .. } => self.queue_note(entry, *duration_ms),
            Step::Failed {
                identity,
                size,
                at,
                error,
            } => self.fail(entry, identity, *size, *at, error),
            Step::Duplicate => {
                self.remove_note(id)?;
                Ok(Next::Done)
            }
        }
    }

    // ------------------------------------------------------------ the steps

    /// Step 1 to 2: the row in `Importing`, and the begin of the early
    /// write in the same store lock. A row that exists is this import's
    /// row: a crash lost the record.
    fn create(&self, entry: &Entry) -> Result<Next, StageError> {
        let id = entry.note_id.as_str();
        if !self.importing(id)? {
            let mut store = lock(&self.env.store);
            {
                let request = &entry.request;
                let title = match &request.title {
                    Some(text) => Title {
                        text: text.clone(),
                        source: TitleSource::User,
                    },
                    None => Title {
                        text: file_stem(&request.source),
                        source: TitleSource::Proposed,
                    },
                };
                store.create_import(&NewImport {
                    id,
                    opened_at: entry.started,
                    title: &title,
                    note_type: request.note_type.as_deref(),
                    tags: &request.tags,
                    model: &RecordingModel {
                        asr_model: entry.recognition.asr_model.clone(),
                        live_transcript: false,
                    },
                    snapshot: entry.recognition.snapshot.as_ref(),
                    speaker_count: entry.recognition.speaker_count,
                    source: &ImportSource {
                        source_name: file_name(&request.source),
                        sha256: None,
                    },
                })?;
                // The early write begins with the row, so a crash before
                // its first step still gets one folder (KTD4).
                early_home::begin(&store, id)?;
            }
        }
        if let Some(homes) = &self.env.early_homes {
            homes.start(id);
        }
        self.tell(ImportEvent::Created {
            note_id: id.to_string(),
        });
        Ok(Next::Step(Step::Created))
    }

    /// Step 2 to 3: the disk check, then the copy with its hash.
    fn copy(&self, entry: &Entry) -> Result<Next, StageError> {
        let id = entry.note_id.as_str();
        let source = &entry.request.source;
        let size = match std::fs::metadata(source) {
            Ok(meta) => meta.len(),
            Err(e) => {
                log::warn!("cannot read the file of the import {id}: {e}");
                return Ok(Next::Wait(Wait::Source));
            }
        };
        if !self.space_for(id, source, size) {
            return Ok(Next::Wait(Wait::Disk));
        }
        let folder = self.working(id).join(COPY_DIR);
        std::fs::create_dir_all(&folder)?;
        match audio_import::copy_with_hash(source, &folder.join(copy_name(source))) {
            Ok(copied) => {
                lock(&self.env.store).set_import_identity(id, &copied.sha256)?;
                Ok(Next::Step(Step::Copied {
                    identity: copied.sha256,
                    size: copied.bytes,
                }))
            }
            Err(e) => {
                log::warn!("cannot copy the file of the import {id}: {e}");
                Ok(Next::Wait(Wait::Source))
            }
        }
    }

    /// Step 3 to 4: decode the copy into the chunks of segment 0. A partial
    /// conversion from before a crash is removed first.
    fn convert(&self, entry: &Entry, identity: &str, size: u64) -> Result<Next, StageError> {
        let id = entry.note_id.as_str();
        let recovery = self.working(id).join(RECOVERY_DIR);
        recorder::remove_segment(&recovery, 0)?;
        let copy = self
            .working(id)
            .join(COPY_DIR)
            .join(copy_name(&entry.request.source));
        let probe = Arc::clone(&self.env.free_space);
        let mut writer: Option<ChunkedWriter> = None;
        let mut write_error: Option<recorder::RecorderError> = None;
        let mut paused = false;
        let decoded = audio_import::decode_mono_until(&copy, |rate, block| {
            if !self.wait_for_recording(id, &mut paused) {
                return false;
            }
            if writer.is_none() {
                let probe = Arc::clone(&probe);
                let created = ChunkedWriter::with_probe(
                    SegmentArgs {
                        dir: recovery.clone(),
                        segment: 0,
                        capture_started_at: u64::try_from(entry.started.timestamp_millis())
                            .unwrap_or(0),
                        offset_ms: 0,
                        sample_rate: rate,
                        channels: 1,
                        floor_bytes: recorder::storage::DISK_RESERVE_BYTES,
                    },
                    Box::new(move |path| probe(path)),
                );
                match created {
                    Ok(created) => writer = Some(created),
                    Err(e) => {
                        write_error = Some(e);
                        return false;
                    }
                }
            }
            match writer.as_mut().map(|w| w.write(block)) {
                Some(Err(e)) => {
                    write_error = Some(e);
                    false
                }
                _ => true,
            }
        });
        if let Some(e) = write_error {
            drop(writer);
            recorder::remove_segment(&recovery, 0)?;
            return match e {
                recorder::RecorderError::LowDisk { .. } => {
                    self.disk_wait(id);
                    Ok(Next::Wait(Wait::Disk))
                }
                recorder::RecorderError::Io(e) => Err(e.into()),
            };
        }
        match decoded {
            Ok(_) => {}
            Err(audio_import::DecodeError::Stopped) => {
                drop(writer);
                recorder::remove_segment(&recovery, 0)?;
                // A delete or the stop of the app: the next run reads the
                // journal entry again.
                return Ok(Next::Wait(Wait::Stopped));
            }
            Err(audio_import::DecodeError::Format(error)) => {
                drop(writer);
                recorder::remove_segment(&recovery, 0)?;
                log::warn!("the decoder refused the file of the import {id}: {error}");
                return Ok(Next::Step(Step::Failed {
                    identity: identity.to_string(),
                    size,
                    at: Local::now().fixed_offset(),
                    error,
                }));
            }
            Err(audio_import::DecodeError::Io(e)) => {
                drop(writer);
                recorder::remove_segment(&recovery, 0)?;
                return Err(e.into());
            }
        }
        let Some(writer) = writer else {
            return Err(StageError::Failed("the decode gave no audio".into()));
        };
        let span = writer
            .finish()
            .map_err(|e| StageError::Failed(e.to_string()))?;
        Ok(Next::Step(Step::Converted {
            identity: identity.to_string(),
            size,
            duration_ms: span.duration_ms(),
            entry_time: Local::now().fixed_offset(),
        }))
    }

    /// Wait while a recording runs (KTD9). Returns false when the import
    /// must stop: the user deleted the note, or the app stops. `paused`
    /// tells whether the conversion paused; the caller keeps it between
    /// blocks.
    fn wait_for_recording(&self, id: &str, paused: &mut bool) -> bool {
        loop {
            if self.stopped.load(Ordering::SeqCst) || self.is_deleted(id) {
                return false;
            }
            if !(self.env.suspended)() {
                break;
            }
            if !*paused {
                *paused = true;
                self.tell(ImportEvent::Paused {
                    note_id: id.to_string(),
                });
            }
            std::thread::sleep(self.env.poll);
        }
        if *paused {
            *paused = false;
            self.tell(ImportEvent::Continued {
                note_id: id.to_string(),
            });
        }
        true
    }

    /// Step 4 to 5: delete the copy, then write the `imported` entry. The
    /// entry of this computer with the time of `entry_time` is this
    /// import's own entry: a crash lost the record of its write.
    fn record_import(
        &self,
        entry: &Entry,
        identity: &str,
        size: u64,
        duration_ms: u64,
        entry_time: DateTime<FixedOffset>,
    ) -> Result<Next, StageError> {
        let id = entry.note_id.as_str();
        remove_folder(&self.working(id).join(COPY_DIR))?;
        let Some(root) = (self.env.storage_folder)() else {
            return Ok(Next::Wait(Wait::StorageFolder));
        };
        let identity_value = parse_identity(identity)?;
        let recorded = Next::Step(Step::Recorded {
            identity: identity.to_string(),
            size,
            duration_ms,
        });
        let own = import_record::entry_of(&root, &identity_value, &self.env.computer_id);
        if own.is_some_and(|own| own.state == State::Imported && own.time == entry_time) {
            return Ok(recorded);
        }
        if entry.request.origin == Origin::WatchedFolder {
            match import_record::lookup(&root, &identity_value) {
                Lookup::Imported(other) if other.computer_id != self.env.computer_id.as_str() => {
                    log::info!(
                        "another computer imported the file of {id} during the conversion; the note is removed"
                    );
                    return Ok(Next::Step(Step::Duplicate));
                }
                Lookup::Wait => return Ok(Next::Wait(Wait::ImportRecord)),
                _ => {}
            }
        }
        let written = import_record::Entry::new(
            State::Imported,
            &file_name(&entry.request.source),
            size,
            self.env.computer_id.clone(),
            entry_time,
        );
        match import_record::write(&root, &identity_value, &written) {
            Ok(_) => Ok(recorded),
            Err(e) => {
                log::warn!("cannot write the import record entry of {id}: {e}");
                Ok(Next::Wait(Wait::ImportRecord))
            }
        }
    }

    /// Step 5: the segment row and the queue in one store lock (KTD6). The
    /// journal lock is held, so a delete cannot start between its check
    /// and the queue.
    fn queue_note(&self, entry: &Entry, duration_ms: u64) -> Result<Next, StageError> {
        let id = entry.note_id.as_str();
        let journal = held(&self.journal);
        if journal.iter().any(|e| e.note_id == id && e.deleted) {
            // The next run takes the delete path.
            return Ok(Next::Wait(Wait::Stopped));
        }
        let queued = {
            let mut store = lock(&self.env.store);
            match store.session(id)? {
                Some(record) if record.state == SessionState::Importing => {
                    let segment = Segment {
                        index: 0,
                        offset_ms: 0,
                        duration_ms,
                        wall_start: entry.started,
                        wall_end: entry.started
                            + chrono::TimeDelta::milliseconds(
                                i64::try_from(duration_ms).unwrap_or(i64::MAX),
                            ),
                        audio_file: crate::recover::segment_audio_file(0),
                        channels: 1,
                        ends_in_pause: false,
                    };
                    store.finish_import(id, &segment, Local::now().fixed_offset())?;
                    true
                }
                // The queue happened, and a crash lost the record; or the
                // note went.
                _ => false,
            }
        };
        drop(journal);
        if queued {
            self.tell(ImportEvent::Queued {
                note_id: id.to_string(),
            });
        }
        Ok(Next::Done)
    }

    /// The decoder refused the file: write the `failed` entry with the
    /// versions (KTD11), then remove the note.
    fn fail(
        &self,
        entry: &Entry,
        identity: &str,
        size: u64,
        at: DateTime<FixedOffset>,
        error: &str,
    ) -> Result<Next, StageError> {
        let id = entry.note_id.as_str();
        let Some(root) = (self.env.storage_folder)() else {
            return Ok(Next::Wait(Wait::StorageFolder));
        };
        let name = file_name(&entry.request.source);
        let failed = import_record::Entry::failed(
            &name,
            size,
            self.env.computer_id.clone(),
            at,
            &self.env.longhand_version,
            audio_import::DECODER_NAME,
            audio_import::DECODER_VERSION,
        );
        let identity_value = parse_identity(identity)?;
        // A file that another computer imported keeps that entry: a
        // lookup resolves `imported` over `failed`.
        if let Err(e) = import_record::write(&root, &identity_value, &failed) {
            log::warn!("cannot write the failed entry of {id}: {e}");
            return Ok(Next::Wait(Wait::ImportRecord));
        }
        self.remove_note(id)?;
        let failure = Failure {
            note_id: id.to_string(),
            identity: identity.to_string(),
            file_name: name,
            at,
        };
        {
            let mut notices = held(&self.notices);
            if !notices.failed.contains(&failure) {
                notices.failed.push(failure.clone());
            }
        }
        log::warn!("the import of {id} failed: {error}");
        self.tell(ImportEvent::Failed(failure));
        Ok(Next::Done)
    }

    /// The user deleted the note (KTD6): remove the working folder, hash
    /// the file when the hash is not known, and write the `imported` entry.
    /// The caller deleted the row and the home folder.
    fn finish_deleted(&self, entry: &Entry) -> Result<Next, StageError> {
        let id = entry.note_id.as_str();
        remove_folder(&self.working(id))?;
        self.clear_disk(id);
        if matches!(entry.step, Step::Recorded { .. } | Step::Duplicate) {
            return Ok(Next::Done);
        }
        let (identity, size) = match entry.step.identity() {
            Some((identity, size)) => (identity.to_string(), size),
            None => {
                let source = &entry.request.source;
                let hashed = std::fs::metadata(source)
                    .map(|meta| meta.len())
                    .and_then(|size| audio_import::hash_file(source).map(|hash| (hash, size)));
                match hashed {
                    Ok(hashed) => hashed,
                    // Nothing can import a file that is gone.
                    Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Next::Done),
                    Err(e) => {
                        log::warn!("cannot hash the file of the deleted import {id}: {e}");
                        return Ok(Next::Wait(Wait::Source));
                    }
                }
            }
        };
        let Some(root) = (self.env.storage_folder)() else {
            return Ok(Next::Wait(Wait::StorageFolder));
        };
        let identity_value = parse_identity(&identity)?;
        if matches!(
            import_record::entry_of(&root, &identity_value, &self.env.computer_id),
            Some(own) if own.state == State::Imported
        ) {
            return Ok(Next::Done);
        }
        let written = import_record::Entry::new(
            State::Imported,
            &file_name(&entry.request.source),
            size,
            self.env.computer_id.clone(),
            Local::now().fixed_offset(),
        );
        match import_record::write(&root, &identity_value, &written) {
            Ok(_) => Ok(Next::Done),
            Err(e) => {
                log::warn!("cannot write the import record entry of {id}: {e}");
                Ok(Next::Wait(Wait::ImportRecord))
            }
        }
    }

    // ------------------------------------------------------------ helpers

    /// Run the import of `id` until it ends or waits.
    fn run_entry(&self, id: &str) {
        loop {
            let Some(entry) = held(&self.journal)
                .iter()
                .find(|e| e.note_id == id)
                .cloned()
            else {
                return;
            };
            let next = match self.step(&entry) {
                Ok(next) => next,
                Err(e) => {
                    log::error!("the import of {id} stopped at {:?}: {e}", entry.step);
                    held(&self.work).waiting.insert(id.to_string());
                    return;
                }
            };
            match next {
                Next::Step(step) => {
                    if let Err(e) = self.record(id, step) {
                        log::error!("could not record the import of {id}: {e}");
                        held(&self.work).waiting.insert(id.to_string());
                        return;
                    }
                }
                Next::Done => {
                    if let Err(e) = self.end(id) {
                        log::error!("could not remove the journal entry of {id}: {e}");
                        held(&self.work).waiting.insert(id.to_string());
                    }
                    return;
                }
                // A delete during the conversion: run the delete path now.
                Next::Wait(Wait::Stopped) if self.is_deleted(id) => {}
                Next::Wait(Wait::Stopped) => return,
                Next::Wait(_) => {
                    held(&self.work).waiting.insert(id.to_string());
                    return;
                }
            }
        }
    }

    /// Remove the journal entry of `id`: the import ended.
    fn end(&self, id: &str) -> io::Result<()> {
        let mut journal = held(&self.journal);
        let Some(i) = journal.iter().position(|e| e.note_id == id) else {
            return Ok(());
        };
        let removed = journal.remove(i);
        if let Err(e) = self.save(&journal) {
            journal.insert(i, removed);
            return Err(e);
        }
        drop(journal);
        self.clear_disk(id);
        self.tell(ImportEvent::Ended {
            note_id: id.to_string(),
        });
        Ok(())
    }

    fn queue(&self, id: &str) {
        let mut work = held(&self.work);
        work.waiting.remove(id);
        if !work.queued.iter().any(|queued| queued == id) {
            work.queued.push_back(id.to_string());
        }
        self.signal.notify_all();
    }

    fn save(&self, entries: &[Entry]) -> io::Result<()> {
        let file = JournalFile {
            version: JOURNAL_VERSION,
            entries: entries.to_vec(),
        };
        let bytes = serde_json::to_vec_pretty(&file)?;
        let dir = self
            .env
            .journal
            .parent()
            .ok_or_else(|| io::Error::other("the journal path has no folder"))?;
        let name = self
            .env
            .journal
            .file_name()
            .ok_or_else(|| io::Error::other("the journal path has no file name"))?;
        home_write::write_atomic(dir, &name.to_string_lossy(), &bytes)
    }

    fn is_deleted(&self, id: &str) -> bool {
        held(&self.journal)
            .iter()
            .any(|e| e.note_id == id && e.deleted)
    }

    /// Whether the note `id` is `Importing`. False when the row is gone.
    /// A row in another state is an error: only the queue step and a
    /// delete take a note out of `Importing`.
    fn importing(&self, id: &str) -> Result<bool, StageError> {
        match lock(&self.env.store).session(id)? {
            None => Ok(false),
            Some(record) if record.state == SessionState::Importing => Ok(true),
            Some(record) => Err(StageError::Failed(format!(
                "the imported note {id} is {}, not importing",
                record.state.name()
            ))),
        }
    }

    fn working(&self, id: &str) -> PathBuf {
        self.env.working_root.join(id)
    }

    /// Whether the volume of the working folder can hold the copy, the
    /// chunks and the processing files of `source` of `size` bytes, and
    /// still keep the recording reserve (KTD9, R22). Sets or clears the
    /// disk notice of `id`.
    fn space_for(&self, id: &str, source: &Path, size: u64) -> bool {
        let available = (self.env.free_space)(&self.env.working_root);
        let needed = disk_need(source, size).saturating_add(recorder::storage::DISK_RESERVE_BYTES);
        if available.is_some_and(|free| free < needed) {
            self.disk_wait(id);
            return false;
        }
        self.clear_disk(id);
        true
    }

    fn disk_wait(&self, id: &str) {
        let added = {
            let mut notices = held(&self.notices);
            let added = !notices.disk.iter().any(|d| d == id);
            if added {
                notices.disk.push(id.to_string());
            }
            added
        };
        if added {
            log::warn!("the import of {id} waits for free space");
            self.tell(ImportEvent::DiskWait {
                note_id: id.to_string(),
            });
        }
    }

    fn clear_disk(&self, id: &str) {
        let removed = {
            let mut notices = held(&self.notices);
            let before = notices.disk.len();
            notices.disk.retain(|d| d != id);
            notices.disk.len() != before
        };
        if removed {
            self.tell(ImportEvent::DiskClear {
                note_id: id.to_string(),
            });
        }
    }

    /// Remove the note `id`: its home folder, its working folder and its
    /// rows. A note with no row loses only its working folder.
    fn remove_note(&self, id: &str) -> Result<(), StageError> {
        if lock(&self.env.store).session(id)?.is_none() {
            return Ok(remove_folder(&self.working(id))?);
        }
        // The early write writes nothing more after this (KTD4).
        if let Some(homes) = &self.env.early_homes {
            homes.cancel(id);
        }
        session_core::jobs::delete_session(
            &self.env.store,
            id,
            &self.working(id),
            &self.env.claims,
            |path: &Path| (self.env.remove)(path),
        )
        .map_err(|e| StageError::Failed(e.to_string()))?;
        self.tell(ImportEvent::Removed {
            note_id: id.to_string(),
        });
        Ok(())
    }

    fn tell(&self, event: ImportEvent) {
        (self.env.events)(event);
    }
}

/// The start of the note of `source` (R16): the recording time in the
/// file, else its modification time, else now.
fn recording_start(source: &Path) -> DateTime<FixedOffset> {
    let time = audio_import::recording_time(source).or_else(|| {
        std::fs::metadata(source)
            .and_then(|meta| meta.modified())
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from)
    });
    match time {
        Some(time) => time.with_timezone(&Local).fixed_offset(),
        None => Local::now().fixed_offset(),
    }
}

/// The bytes that the import of `source` of `size` bytes needs in the
/// working location: the copy, the chunks and the processing files of the
/// longest audio that a file of that size can hold.
fn disk_need(source: &Path, size: u64) -> u64 {
    let wav = source
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
    let per_second = if wav {
        WAV_BYTES_PER_SECOND
    } else {
        COMPRESSED_BYTES_PER_SECOND
    };
    let seconds = size.div_ceil(per_second).max(1);
    let chunks =
        recorder::storage::chunk_bytes_per_second(ESTIMATE_RATE, 1).saturating_mul(seconds);
    size.saturating_add(chunks)
        .saturating_add(recorder::storage::processing_demand(seconds))
}

/// The name of the copy: `source` with the extension of the file, so the
/// decoder gets the format hint.
fn copy_name(source: &Path) -> String {
    match source.extension().and_then(|e| e.to_str()) {
        Some(extension) => format!("source.{}", extension.to_ascii_lowercase()),
        None => "source".to_string(),
    }
}

fn file_name(source: &Path) -> String {
    source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The file name with no extension: the first title (R15).
fn file_stem(source: &Path) -> String {
    source
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn parse_identity(identity: &str) -> Result<Identity, StageError> {
    Identity::parse(identity)
        .ok_or_else(|| StageError::Failed(format!("{identity} is not a content identity")))
}

/// Remove `folder` and all it holds. A folder that is not there is removed.
fn remove_folder(folder: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(folder) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
