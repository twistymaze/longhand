//! The session list and each session's state, notes and saved documents,
//! read from the job store and the home folder, which are the truth: a
//! window shown mid-processing rebuilds from these, then follows events
//! (U12, AE4).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pipeline::folder_rename::FolderRename;
use serde::Serialize;
use session_core::folder::{SessionDoc, Summary, SummaryMeta, Transcript, TranscriptLine};
use session_core::note_folder::{Layout, NoteFiles};
use session_core::note_record::NOTE_RECORD_FILE;
use session_core::{Event, SessionRecord, SessionState, Stage};
use specta::Type;
use tauri::{AppHandle, State};

use library::scan::{NoteRead, READ_TIMEOUT, Read, RemoteState, Waiting, read_note, read_timed};
use library::status::LibraryStatus;

use super::blocking;
use crate::managers::capture::CaptureManager;
use crate::managers::live::{LiveManager, LiveTranscript};
use crate::managers::processing::{ProcessingManager, SpeakerProgressView, WaitView, stage_name};
use crate::managers::settings;
use pipeline::runner::Waits;

#[derive(Serialize, Clone, Type)]
pub struct SessionEntry {
    pub id: String,
    /// When the first segment began (RFC 3339), or when the session was made.
    pub started: String,
    /// Recorded time, gaps excluded.
    pub duration_ms: u64,
    /// The lifecycle state's name: capturing, interrupted, queued,
    /// processing, blocked, processing_failed, awaiting_destination, saved.
    pub state: String,
    /// The stage started last: the running one, or the one a failed
    /// session stopped at.
    pub stage: Option<String>,
    /// The proposed or typed title, if the session has one (R28).
    pub title: Option<String>,
    /// The number of speakers that processing found (KTD11). `None` before
    /// the join, or when the transcript has no speaker marks.
    pub found_speakers: Option<u32>,
    /// The note's storage folder is unavailable, or its `session.md` cannot
    /// be read now (R16). Opening it and its actions wait.
    pub storage_unavailable: bool,
    /// Work on the note on another computer, as the latest library scan
    /// found it (KTD5). `None` when no other computer works on the note,
    /// and always for a note that is not Saved on this computer.
    pub remote: Option<RemoteView>,
    /// The note type that the app shows (KTD3): a type change that waits
    /// for its move, else the type folder that holds the home folder.
    /// `None` is General.
    pub note_type: Option<String>,
    /// The tags of the note, as their exact texts (R9, R11).
    pub tags: Vec<String>,
    /// A file of the note folder gives no note (R6). The actions that
    /// write to the note wait. `None` for a note whose files are valid.
    pub problem: Option<NoteProblemView>,
    /// What the note waits for before its next stage can run, from the
    /// runner (KTD10). `None` when it waits for nothing: its stage runs,
    /// or it has no work.
    pub waits_for: Option<WaitView>,
}

/// What the user can do about a file of a note folder that gives no note
/// (R6).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKindView {
    /// `session.md` does not parse. The user can correct it.
    UserFile,
    /// `longhand.json` does not parse or is not there: wait for the sync,
    /// or update Longhand on the other computer.
    Record,
    /// A newer Longhand wrote `longhand.json`: update Longhand.
    NewerRecord,
}

/// A file of a note folder that gives no note (R6).
#[derive(Serialize, Clone, PartialEq, Eq, Debug, Type)]
pub struct NoteProblemView {
    pub kind: ProblemKindView,
    pub file: String,
    /// The line of the error in the file, when the parse tells it.
    pub line: Option<u32>,
    /// Why the actions wait, and what the user can do.
    pub reason: String,
}

impl From<&library::scan::FileProblem> for NoteProblemView {
    fn from(problem: &library::scan::FileProblem) -> Self {
        use library::scan::ProblemKind;
        Self {
            kind: match problem.kind {
                ProblemKind::UserFile => ProblemKindView::UserFile,
                ProblemKind::Record => ProblemKindView::Record,
                ProblemKind::NewerRecord => ProblemKindView::NewerRecord,
            },
            file: problem.file.to_string(),
            line: problem.line.and_then(|line| u32::try_from(line).ok()),
            reason: crate::managers::processing::problem_reason(problem),
        }
    }
}

/// Work on a note on another computer (KTD5). Record, Resume, Transcribe
/// Again, Regenerate Summary, Remove Audio and Delete wait for it (R21),
/// except as R23 allows.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, Type)]
#[serde(rename_all = "snake_case")]
pub enum RemoteView {
    /// Another computer records into or processes the note (R12, R20).
    InUse,
    /// Another computer recorded into the note and stopped before it saved
    /// the recording: "Not finished on another computer" (R23). Notes
    /// edits, and Delete after a confirmation, are available.
    NotFinished,
}

impl From<RemoteState> for RemoteView {
    fn from(state: RemoteState) -> Self {
        match state {
            RemoteState::InUse => Self::InUse,
            RemoteState::NotFinished => Self::NotFinished,
        }
    }
}

/// A part of a note that the sync has not delivered yet (R6).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, Type)]
#[serde(rename_all = "snake_case")]
pub enum WaitingPart {
    Transcript,
    Summary,
    Notes,
    Audio,
}

/// The audio of a note, from its folder (KTD9). The note page shows no
/// mark for it. It gives the reason of an unavailable action only (R14).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, Type)]
#[serde(rename_all = "snake_case")]
pub enum AudioView {
    /// Every segment has its audio file.
    Present,
    /// At least one segment has a removal marker. The audio does not come
    /// back.
    Removed,
    /// At least one segment has no audio file yet: the sync can still
    /// deliver it.
    Waiting,
}

impl From<pipeline::save::AudioState> for AudioView {
    fn from(state: pipeline::save::AudioState) -> Self {
        use pipeline::save::AudioState;
        match state {
            AudioState::Present => Self::Present,
            AudioState::Removed => Self::Removed,
            AudioState::Waiting => Self::Waiting,
        }
    }
}

/// What the one recording button of the note page does (R11).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, Type)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    /// "Record": the note has no audio. The button calls `record_note`.
    Record,
    /// "Resume": the note has audio. The button calls `resume_recording`.
    Resume,
}

/// The recording button of the note page (R11, R12).
#[derive(Serialize, Clone, PartialEq, Eq, Debug, Type)]
pub struct RecordButton {
    pub kind: RecordKind,
    /// Record or Resume can start now.
    pub available: bool,
    /// Why the button is unavailable. `None` while it is available.
    pub reason: Option<String>,
}

/// The reason of a note that records or is processed on another computer
/// (R12, R21).
const IN_USE_ELSEWHERE: &str =
    "This note is recording or being processed on another computer. Try again when it is done.";

/// The reason of a note whose recording another computer did not save
/// (R12, R23).
const NOT_FINISHED_ELSEWHERE: &str =
    "Another computer recorded into this note and did not save the recording.";

/// The reason of a note whose audio was removed (AE3).
const AUDIO_REMOVED: &str = "The audio of this note was removed.";

const CANNOT_RESUME: &str = "This note can’t be resumed.";

const CANNOT_RECORD: &str = "This note can’t record now.";

/// What the recording button reads to find its state.
#[derive(Debug, Clone, Copy)]
struct RecordCheck<'a> {
    kind: RecordKind,
    /// The state machine allows the event now, and no save has begun.
    allowed: bool,
    audio: AudioView,
    storage_unavailable: bool,
    waiting: &'a [WaitingPart],
    remote: Option<RemoteView>,
}

/// Why Record or Resume cannot start now (R12), or `None` when it can.
/// The work of another computer comes first, then the storage folder, the
/// audio and the sync.
fn record_refusal(check: &RecordCheck) -> Option<&'static str> {
    match check.remote {
        Some(RemoteView::InUse) => return Some(IN_USE_ELSEWHERE),
        Some(RemoteView::NotFinished) => return Some(NOT_FINISHED_ELSEWHERE),
        None => {}
    }
    if check.storage_unavailable {
        return Some(FOLDER_UNAVAILABLE);
    }
    if check.kind == RecordKind::Resume && check.audio == AudioView::Removed {
        return Some(AUDIO_REMOVED);
    }
    if !check.waiting.is_empty() {
        return Some(crate::managers::processing::WAIT_FOR_SYNC);
    }
    if !check.allowed {
        return Some(match check.kind {
            RecordKind::Record => CANNOT_RECORD,
            RecordKind::Resume => CANNOT_RESUME,
        });
    }
    None
}

/// The recording button of the note of `record`, or `None` while the note
/// records, is interrupted (the recording bar owns those) or is
/// discarded. `saving`: the save of the note has begun.
fn record_button(
    record: &SessionRecord,
    has_segments: bool,
    saving: bool,
    audio: pipeline::save::AudioState,
    status: &LibraryStatus,
) -> Option<RecordButton> {
    if matches!(
        record.state,
        SessionState::Capturing | SessionState::Interrupted { .. } | SessionState::Discarded
    ) {
        return None;
    }
    let (kind, event) = if has_segments {
        let event = Event::UserResume {
            space_ok: true,
            audio_intact: audio == pipeline::save::AudioState::Present,
        };
        (RecordKind::Resume, event)
    } else {
        (RecordKind::Record, Event::UserRecord { has_segments })
    };
    let note = status.note(&record.id);
    let waiting = waiting_parts(&note.waiting);
    let reason = record_refusal(&RecordCheck {
        kind,
        allowed: !saving && record.state.transition(&event).is_ok(),
        audio: audio.into(),
        storage_unavailable: storage_unavailable(status, record),
        waiting: &waiting,
        remote: status.remote(record).map(RemoteView::from),
    });
    // A file of the folder that gives no note holds the recording too (R6).
    // The work of another computer still comes first.
    let reason = match (reason, &note.problem) {
        (Some(IN_USE_ELSEWHERE | NOT_FINISHED_ELSEWHERE), _) | (_, None) => {
            reason.map(str::to_string)
        }
        (_, Some(problem)) => Some(crate::managers::processing::problem_reason(problem)),
    };
    Some(RecordButton {
        kind,
        available: reason.is_none(),
        reason,
    })
}

/// The checks of Record for the note `id` at `now` (R12, R21, R23): the
/// recording button must be Record and available, and no in-use marker of
/// another computer's run in the home folder is fresh. `own_runs` are
/// this computer's runs ([`ProcessingManager::own_runs`]). Returns the
/// reason of a refusal. Reads the folder with no store lock.
pub(crate) fn check_record(
    store: &pipeline::SharedStore,
    status: &LibraryStatus,
    own_runs: &[String],
    id: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<(), String> {
    let e = |e: pipeline::StageError| e.to_string();
    let (record, button) = {
        let store = store.lock().unwrap_or_else(|e| e.into_inner());
        let record = store
            .session(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no note {id}"))?;
        let has_segments = !store.segments(id).map_err(|e| e.to_string())?.is_empty();
        let saving =
            record.state != SessionState::Saved && pipeline::save::begun(&store, id).map_err(e)?;
        let audio = pipeline::save::audio_state(&store, id).map_err(e)?;
        let button = record_button(&record, has_segments, saving, audio, status);
        (record, button)
    };
    match button {
        Some(RecordButton {
            kind: RecordKind::Record,
            available: true,
            ..
        }) => {}
        Some(RecordButton {
            kind: RecordKind::Resume,
            ..
        }) => return Err("This note has audio. Use Resume to record more.".to_string()),
        Some(RecordButton { reason, .. }) => {
            return Err(reason.unwrap_or_else(|| CANNOT_RECORD.to_string()));
        }
        None => return Err(CANNOT_RECORD.to_string()),
    }
    refuse_other_computer(&record, status, own_runs, now)
}

/// The checks of Resume for the note `id` at `now` (R12, R21, R23): no
/// other computer records into the note, processes it or did not finish
/// its recording. An interrupted or recording note is the work of this
/// computer, so its folder is not read. `own_runs` are this computer's
/// runs. Returns the reason of a refusal. Reads the folder with no store
/// lock.
pub(crate) fn check_resume(
    store: &pipeline::SharedStore,
    status: &LibraryStatus,
    own_runs: &[String],
    id: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<(), String> {
    let record = {
        let store = store.lock().unwrap_or_else(|e| e.into_inner());
        store
            .session(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no note {id}"))?
    };
    if record.state != SessionState::Saved {
        return Ok(());
    }
    refuse_other_computer(&record, status, own_runs, now)
}

/// Why the work of another computer on the note of `record` refuses
/// Record or Resume at `now` (R21, R23): the latest scan shows it, or a
/// fresh in-use marker of a run that is not in `own_runs` shows it. Reads
/// the folder.
fn refuse_other_computer(
    record: &SessionRecord,
    status: &LibraryStatus,
    own_runs: &[String],
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<(), String> {
    match status.remote(record).map(RemoteView::from) {
        Some(RemoteView::InUse) => return Err(IN_USE_ELSEWHERE.to_string()),
        Some(RemoteView::NotFinished) => return Err(NOT_FINISHED_ELSEWHERE.to_string()),
        None => {}
    }
    match folder_in_use(record, own_runs, now) {
        Ok(false) => Ok(()),
        Ok(true) => Err(IN_USE_ELSEWHERE.to_string()),
        Err(()) => Err(FOLDER_UNAVAILABLE.to_string()),
    }
}

/// Whether an in-use marker of a run that is not in `own_runs` is fresh in
/// the home folder of `record` at `now` (KTD5): another computer records
/// into or processes the note. The latest library scan can be older than
/// that work, so the actions of R21 read the folder too. `own_runs` are
/// this computer's runs. A folder that is not there (the early write
/// waits, or the folder was removed) is not in use. `Err` when the folder
/// cannot be read.
fn folder_in_use(
    record: &SessionRecord,
    own_runs: &[String],
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<bool, ()> {
    let Some(home) = record.home_path.as_deref() else {
        return Ok(false);
    };
    match pipeline::in_use::in_use_except(Path::new(home), own_runs, now) {
        Ok(used) => Ok(used),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(()),
    }
}

/// The parts that `waiting` names, in its order, each one time.
fn waiting_parts(waiting: &[Waiting]) -> Vec<WaitingPart> {
    let mut parts = Vec::new();
    for w in waiting {
        let part = match w {
            Waiting::Transcript => WaitingPart::Transcript,
            Waiting::Summary => WaitingPart::Summary,
            Waiting::Notes => WaitingPart::Notes,
            Waiting::Audio(_) => WaitingPart::Audio,
        };
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    parts
}

/// The names of the other versions of a note: conflict copies and other
/// folders with the same note id (R15).
fn file_names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| {
            p.file_name()
                .map_or_else(|| p.to_string_lossy(), |n| n.to_string_lossy())
                .into_owned()
        })
        .collect()
}

#[derive(Serialize, Type)]
pub struct EndpointView {
    pub host: String,
    pub model: String,
    /// False when the transcript would cross the network unencrypted.
    pub encrypted: bool,
}

#[derive(Serialize, Type)]
pub struct SessionDetail {
    pub entry: SessionEntry,
    /// Stages whose work is complete.
    pub done: Vec<String>,
    /// The stage started last.
    pub current: Option<String>,
    pub windows_done: Option<u32>,
    pub windows_total: Option<u32>,
    /// The speaker stage's last saved progress while that stage runs.
    pub speakers: Option<SpeakerProgressView>,
    /// The stage a failed session stopped at.
    pub failed_stage: Option<String>,
    /// Why it failed, is blocked, or is waiting for its destination.
    pub message: Option<String>,
    /// Why the summary is missing, when the session will be saved with a stub.
    pub summary_problem: Option<String>,
    /// The state machine allows Resume now (R6a).
    pub resumable: bool,
    /// The recording button of the note page: Record or Resume, whether it
    /// is available, and why not (R11, R12). `None` while the note records
    /// or is interrupted. The work of another computer is in
    /// `entry.remote`.
    pub record: Option<RecordButton>,
    pub home_path: Option<String>,
    /// Where the summary goes, and what is sent: the transcript and notes (R30).
    pub endpoint: Option<EndpointView>,
    pub audio_kept: bool,
    pub transcript_kept: bool,
    /// The audio of the note (KTD9). Transcribe Again, Resume and Retry
    /// read it for their reason.
    pub audio: AudioView,
    /// The recognition model that the note recorded when it started
    /// (KTD2). `None` for a note that started before the update.
    pub asr_model: Option<String>,
    /// The note records a live transcript (R8, R18). The Transcript tab
    /// reads this, not the model selected now (R9).
    pub live_transcript: bool,
    /// A Transcribe Again run waits or runs (KTD14). It has no live text.
    pub transcribe_again: bool,
    /// A summary regeneration waits or runs. It continues when the user
    /// leaves the note, so the page reads it here when the user comes back.
    pub regenerating: bool,
    /// The same stage failed again after a Retry. The next Retry starts
    /// over from the recorded audio, after the user confirms.
    pub retry_starts_over: bool,
    /// Why the last Transcribe Again run failed. The note kept its
    /// previous transcript and summary (R19).
    pub transcribe_again_error: Option<String>,
    /// Why the last Transcribe Again run kept the previous summary with
    /// the new transcript (R19).
    pub summary_not_updated: Option<String>,
    /// The parts that the sync has not delivered yet (R6). Notes edits,
    /// Regenerate Summary, Transcribe Again and Resume wait for them.
    pub waiting: Vec<WaitingPart>,
    /// The file or folder names of other versions of the note, which the
    /// user resolves in Finder (R15).
    pub conflict_files: Vec<String>,
    /// Why the first write of the note's home folder waits, while it
    /// tries again: the storage folder is not available, for example
    /// (KTD4). The page shows "Not saved yet". `None` when the write is
    /// done or has no problem.
    pub not_saved_yet: Option<String>,
}

#[derive(Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocLine {
    Turn {
        position_ms: u64,
        speaker: Option<u32>,
        text: String,
    },
    /// Between segments; `at_ms` is where the earlier segment ends, and
    /// `wall_start` (RFC 3339) is when the next segment started (KTD12).
    Gap {
        at_ms: u64,
        duration_ms: u64,
        wall_start: Option<String>,
    },
    /// Between segments, where the user paused the note (KTD5 of the pause
    /// plan).
    Pause {
        duration_ms: u64,
    },
    Silence {
        duration_ms: u64,
    },
    /// A line in free text, for example from an agent edit (R7, KTD7). It
    /// has no time.
    Text {
        text: String,
    },
}

#[derive(Serialize, Type)]
pub struct SummaryView {
    /// Markdown.
    pub body: String,
    /// Summarization failed and this stands in for it.
    pub stub: bool,
    pub host: Option<String>,
    pub model: Option<String>,
    /// The name of the prompt that made the summary. A summary made before
    /// prompts had names, and a stub, have none.
    pub prompt: Option<String>,
    /// The id of the prompt that made the summary (KTD4). A summary made
    /// before prompt ids were recorded, and a stub, have none.
    pub prompt_id: Option<String>,
}

#[derive(Serialize, Type)]
pub struct SessionDocument {
    pub has_speakers: bool,
    pub lines: Vec<DocLine>,
    /// The text of `transcript.md`, which the Transcript tab copies (R17).
    pub transcript_markdown: String,
    pub summary: Option<SummaryView>,
}

/// The entry of `record`. The storage flag and the remote state come from
/// the latest library scan in `status`: the entry reads no folder.
fn entry(
    record: &SessionRecord,
    segments: &[session_core::Segment],
    stage: Option<Stage>,
    status: &LibraryStatus,
    waits: &Waits,
) -> SessionEntry {
    let waits_for = waits
        .of(&record.id, &record.state, stage)
        .map(WaitView::from);
    let stage = match &record.state {
        SessionState::ProcessingFailed { stage, .. } => Some(*stage),
        SessionState::Processing { .. } => stage,
        _ => None,
    };
    SessionEntry {
        id: record.id.clone(),
        started: record.start(segments).to_rfc3339(),
        duration_ms: segments.iter().map(|s| s.duration_ms).sum(),
        state: record.state.name().to_string(),
        stage: stage.map(stage_name),
        title: record.title.as_ref().map(|t| t.text.clone()),
        found_speakers: record.found_speakers,
        storage_unavailable: storage_unavailable(status, record),
        remote: status.remote(record).map(RemoteView::from),
        note_type: pipeline::type_move::shown_type(record).0,
        tags: record.tags.clone(),
        problem: status
            .note(&record.id)
            .problem
            .as_ref()
            .map(NoteProblemView::from),
        waits_for,
    }
}

fn storage_unavailable(status: &LibraryStatus, record: &SessionRecord) -> bool {
    status.storage_unavailable(&record.id, record.home_path.as_deref().map(Path::new))
}

/// Every session but the discarded ones: the switcher's list, newest first.
/// The storage flag comes from the latest library scan: the list reads no
/// folder.
#[tauri::command]
#[specta::specta]
pub async fn list_sessions(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
) -> Result<Vec<SessionEntry>, String> {
    let (capture, processing) = (Arc::clone(&capture), Arc::clone(&processing));
    blocking(move || session_entries(&capture, &processing)).await
}

/// The body of [`list_sessions`]. It runs on a blocking thread.
fn session_entries(
    capture: &CaptureManager,
    processing: &ProcessingManager,
) -> Result<Vec<SessionEntry>, String> {
    let status = processing.library_status();
    // The waits read the capture state, so they come before the store lock.
    let waits = processing.waits();
    let store = capture.shared_store();
    let store = store.lock().unwrap_or_else(|e| e.into_inner());
    let records = store.sessions().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for record in records.iter().rev() {
        if record.state != SessionState::Discarded {
            let segments = store.segments(&record.id).map_err(|e| e.to_string())?;
            let stage = store.current_stage(&record.id).map_err(|e| e.to_string())?;
            out.push(entry(record, &segments, stage, status, &waits));
        }
    }
    Ok(out)
}

/// A note folder whose files give no note, in the notice on Home (R6).
#[derive(Serialize, Clone, PartialEq, Eq, Debug, Type)]
pub struct ProblemNote {
    /// The title of the note, or the name of its folder when the note has
    /// no known title.
    pub name: String,
    pub problem: NoteProblemView,
}

/// A note that the last conversion could not convert (R10).
#[derive(Serialize, Clone, PartialEq, Eq, Debug, Type)]
pub struct FailedNote {
    /// The title of the note, or the name of its folder.
    pub name: String,
    pub reason: String,
}

/// The Convert notice of Home (R11, F3). It shows only while the storage
/// folder has a note in the old layout.
#[derive(Serialize, Clone, PartialEq, Eq, Debug, Type)]
pub struct ConversionNotice {
    /// The number of notes in the old layout.
    pub old_notes: u32,
    /// A conversion runs now.
    pub running: bool,
    /// The number of notes that the last conversion skipped, because a
    /// computer worked on them. A later run converts them.
    pub skipped: u32,
    /// The notes that the last conversion could not convert and that are
    /// still old.
    pub failed: Vec<FailedNote>,
}

/// The library notices of Home, from the latest library scans (KTD14).
/// In the normal case each part is empty, and Home then shows nothing.
#[derive(Serialize, Clone, PartialEq, Eq, Debug, Type)]
pub struct LibraryNotices {
    /// `None` when the storage folder has no note in the old layout: then
    /// no conversion control shows (R11).
    pub conversion: Option<ConversionNotice>,
    /// The note folders whose files have given no note for about one
    /// minute, sorted by folder (R6).
    pub problems: Vec<ProblemNote>,
}

/// The state of the conversion on this computer (U5). It stays in memory
/// only.
#[derive(Default)]
pub struct Conversion(std::sync::Mutex<ConversionState>);

#[derive(Default, Clone)]
struct ConversionState {
    running: bool,
    /// The notes that the last run skipped.
    skipped: Vec<String>,
    /// The notes that the last run could not convert, with their folders
    /// and reasons.
    failed: Vec<(String, PathBuf, String)>,
}

impl Conversion {
    fn lock(&self) -> std::sync::MutexGuard<'_, ConversionState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The title of the row `id` in `records`, else the name of `folder`.
fn note_name(records: &[SessionRecord], id: Option<&str>, folder: &Path) -> String {
    id.and_then(|id| records.iter().find(|r| r.id == id))
        .and_then(|r| r.title.as_ref().map(|t| t.text.clone()))
        .unwrap_or_else(|| {
            folder.file_name().map_or_else(
                || folder.to_string_lossy().into_owned(),
                |n| n.to_string_lossy().into_owned(),
            )
        })
}

/// The notices of the library: the old notes and the last conversion
/// (`conversion`), and the reported problems in `status`. Each note is
/// named by the title of its row in `records`, else by its folder name.
fn library_notices_of(
    status: &LibraryStatus,
    conversion: &ConversionState,
    records: &[SessionRecord],
) -> LibraryNotices {
    let old = status.old_layout_notes();
    let still_old = |id: &str| old.iter().any(|(old_id, _)| old_id == id);
    let conversion = (!old.is_empty()).then(|| ConversionNotice {
        old_notes: u32::try_from(old.len()).unwrap_or(u32::MAX),
        running: conversion.running,
        skipped: u32::try_from(conversion.skipped.iter().filter(|id| still_old(id)).count())
            .unwrap_or(u32::MAX),
        failed: conversion
            .failed
            .iter()
            .filter(|(id, _, _)| still_old(id))
            .map(|(id, folder, reason)| FailedNote {
                name: note_name(records, Some(id), folder),
                reason: reason.clone(),
            })
            .collect(),
    });
    let problems = status
        .problems()
        .into_iter()
        .map(|reported| ProblemNote {
            name: note_name(records, reported.known_id.as_deref(), &reported.path),
            problem: NoteProblemView::from(&reported.problem),
        })
        .collect();
    LibraryNotices {
        conversion,
        problems,
    }
}

/// The library notices of Home (KTD14). They come from the latest library
/// scan: the command reads no folder.
#[tauri::command]
#[specta::specta]
pub async fn library_notices(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    conversion: State<'_, Conversion>,
) -> Result<LibraryNotices, String> {
    let (capture, processing) = (Arc::clone(&capture), Arc::clone(&processing));
    let state = conversion.lock().clone();
    blocking(move || {
        let records = {
            let store = capture.shared_store();
            let store = store.lock().unwrap_or_else(|e| e.into_inner());
            store.sessions().map_err(|e| e.to_string())?
        };
        Ok(library_notices_of(
            processing.library_status(),
            &state,
            &records,
        ))
    })
    .await
}

/// Convert the notes in the old layout to the new layout (U5, R10, R12).
/// Only the user starts it, from the Convert notice. A note that a
/// computer works on now is skipped. The notices show the result: the
/// conversion sends a library change when it ends.
#[tauri::command]
#[specta::specta]
pub async fn convert_old_notes(
    processing: State<'_, Arc<ProcessingManager>>,
    conversion: State<'_, Conversion>,
) -> Result<(), String> {
    {
        let mut state = conversion.lock();
        if state.running {
            return Ok(());
        }
        state.running = true;
    }
    let processing = Arc::clone(&processing);
    let report = tauri::async_runtime::spawn_blocking(move || processing.convert_old_notes()).await;
    let mut state = conversion.lock();
    state.running = false;
    let report = report.map_err(|e| e.to_string())?;
    state.skipped = report.skipped;
    state.failed = report.failed;
    Ok(())
}

fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some())
}

#[tauri::command]
#[specta::specta]
pub async fn session_detail(
    app: AppHandle,
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<SessionDetail, String> {
    let (capture, processing) = (Arc::clone(&capture), Arc::clone(&processing));
    blocking(move || detail_of(app, capture, processing, session_id)).await
}

/// The body of [`session_detail`]. It runs on a blocking thread.
fn detail_of(
    app: AppHandle,
    capture: Arc<CaptureManager>,
    processing: Arc<ProcessingManager>,
    session_id: String,
) -> Result<SessionDetail, String> {
    let shared = capture.shared_store();
    let e = |e: pipeline::StageError| e.to_string();
    // The waits read the capture state, so they come before the store lock.
    let waits = processing.waits();
    let (record, segments, saving, audio, again) = {
        let store = shared.lock().unwrap_or_else(|e| e.into_inner());
        let record = store
            .session(&session_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no note {session_id}"))?;
        let segments = store.segments(&session_id).map_err(|e| e.to_string())?;
        // A completed session has finished saving; only one still saving
        // refuses a resume.
        let saving = record.state != SessionState::Saved
            && pipeline::save::begun(&store, &session_id).map_err(e)?;
        let audio = pipeline::save::audio_state(&store, &session_id).map_err(e)?;
        let again = store
            .transcribe_again_marks(&session_id)
            .map_err(|e| e.to_string())?;
        (record, segments, saving, audio, again)
    };
    let audio_intact = audio == pipeline::save::AudioState::Present;
    let snapshot = pipeline::progress::snapshot(&shared, &session_id).map_err(e)?;

    let resumable = !saving
        && record
            .state
            .transition(&Event::UserResume {
                space_ok: true,
                audio_intact,
            })
            .is_ok()
        && !matches!(record.state, SessionState::Capturing);

    let (failed_stage, message) = match &record.state {
        SessionState::ProcessingFailed { stage, .. } => (
            Some(*stage),
            pipeline::progress::error(&shared, &session_id, *stage).map_err(e)?,
        ),
        SessionState::Blocked { .. } => (
            None,
            pipeline::progress::error(&shared, &session_id, Stage::Summarize).map_err(e)?,
        ),
        SessionState::AwaitingDestination { problem, .. } => (None, Some(problem.clone())),
        _ => (None, None),
    };
    let summary_problem: Option<String> = pipeline::checkpoint::get(
        &shared,
        &session_id,
        Stage::Summarize,
        pipeline::save::SUMMARY_FAILED,
    )
    .map_err(e)?;

    let s = settings::get_settings(&app);
    let endpoint = s.endpoint_configured().then(|| {
        let base_url = s.effective_endpoint_base_url();
        EndpointView {
            host: secrets::endpoint_key(base_url).unwrap_or_else(|_| base_url.to_string()),
            model: s.endpoint_model.clone(),
            encrypted: base_url.starts_with("https://"),
        }
    });

    let working = capture.working_root().join(&session_id);
    let (audio_kept, transcript_kept) = match &record.home_path {
        Some(path) => (
            has_files(&Path::new(path).join("audio")),
            Path::new(path)
                .join(pipeline::join::TRANSCRIPT_FILE)
                .exists(),
        ),
        None => (
            has_files(&working.join("audio")) || has_files(&working.join(pipeline::RECOVERY_DIR)),
            working.join(pipeline::join::TRANSCRIPT_FILE).exists(),
        ),
    };

    let status = processing.library_status();
    let note = status.note(&session_id);
    let record_button = record_button(&record, !segments.is_empty(), saving, audio, status);
    Ok(SessionDetail {
        entry: entry(&record, &segments, snapshot.current, status, &waits),
        done: snapshot.done.into_iter().map(stage_name).collect(),
        current: snapshot.current.map(stage_name),
        windows_done: snapshot.windows.map(|(d, _)| d as u32),
        windows_total: snapshot.windows.map(|(_, t)| t as u32),
        speakers: snapshot.speakers.map(SpeakerProgressView::from),
        failed_stage: failed_stage.map(stage_name),
        message,
        summary_problem,
        resumable,
        record: record_button,
        home_path: record.home_path.clone(),
        endpoint,
        audio_kept,
        transcript_kept,
        audio: audio.into(),
        asr_model: record.asr_model.clone(),
        live_transcript: record.live_transcript == Some(true),
        transcribe_again: again.model.is_some(),
        regenerating: processing.is_regenerating(&session_id),
        transcribe_again_error: again.error,
        retry_starts_over: pipeline::runner::retry_starts_over(&shared, &session_id).map_err(e)?,
        summary_not_updated: again.summary_error,
        waiting: waiting_parts(&note.waiting),
        conflict_files: file_names(&note.other_versions),
        not_saved_yet: processing.early_homes().problem(&session_id),
    })
}

/// A session's notes document and the fingerprint of the `notes.md` that
/// holds it. The editor sends the fingerprint back with each save
/// (KTD10).
#[derive(Serialize, Type)]
pub struct LoadedNotes {
    pub markdown: String,
    pub fingerprint: String,
}

/// A session's notes document, editable whatever its state (R20), with the
/// fingerprint that [`save_session_notes`] checks.
#[tauri::command]
#[specta::specta]
pub async fn load_session_notes(
    capture: State<'_, Arc<CaptureManager>>,
    session_id: String,
) -> Result<LoadedNotes, String> {
    let capture = Arc::clone(&capture);
    blocking(move || {
        let markdown = capture
            .session_notes(&session_id)
            .map_err(|e| e.to_string())?;
        Ok(LoadedNotes {
            fingerprint: pipeline::save::notes_fingerprint(&markdown),
            markdown,
        })
    })
    .await
}

/// What a notes save did.
#[derive(Serialize, Type)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotesSaveResult {
    /// `fingerprint` is the fingerprint of the saved document, which the
    /// editor sends with its next save.
    Saved { fingerprint: String },
    /// Another computer changed `notes.md` after the editor loaded it. The
    /// file keeps that text, and the edit is in the draft file `draft`
    /// beside it. The editor loads the notes again.
    Conflict { draft: String },
    /// Another computer removed the folder of this note with no audio
    /// (R24). The text is in the new note `session_id`, with no audio, and
    /// the old note is gone from Longhand. The page tells the user and
    /// opens the new note. `fingerprint` is the fingerprint of the saved
    /// document.
    NewNote {
        session_id: String,
        fingerprint: String,
    },
}

/// Replace a session's notes document. The editor sends the whole
/// document; on a saved session the home folder's `notes.md` follows (R20).
/// When another computer removed the folder of a note with no audio, the
/// text goes into a new note (R24). The write runs off the main thread,
/// since it can reach a synced folder. `loaded_fingerprint` is the
/// fingerprint that [`load_session_notes`] gave for the document that the
/// editor changed; `None` skips the check.
#[tauri::command]
#[specta::specta]
pub async fn save_session_notes(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
    markdown: String,
    loaded_fingerprint: Option<String>,
) -> Result<NotesSaveResult, String> {
    let capture = Arc::clone(&capture);
    let processing = Arc::clone(&processing);
    tauri::async_runtime::spawn_blocking(move || {
        save_notes_keeping_text(
            &capture,
            &processing,
            &session_id,
            &markdown,
            loaded_fingerprint.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The notes save of [`save_session_notes`]. A folder that has another
/// name gets one more save; a folder that is gone makes the new note.
fn save_notes_keeping_text(
    capture: &CaptureManager,
    processing: &Arc<ProcessingManager>,
    session_id: &str,
    markdown: &str,
    loaded: Option<&str>,
) -> Result<NotesSaveResult, String> {
    let claims = processing.claims();
    let store = capture.shared_store();
    let early_homes = processing.early_homes();
    keep_notes_text(
        session_id,
        pipeline::save::notes_fingerprint(markdown),
        || {
            capture
                .save_session_notes(session_id, markdown, loaded, &claims)
                .map_err(|e| e.to_string())
        },
        |new_id| {
            let now = chrono::Local::now().fixed_offset();
            pipeline::save::rescue_notes(&store, session_id, new_id, now).map_err(|e| e.to_string())
        },
        |step| match step {
            EarlyHomeStep::Cancel(id) => early_homes.cancel(id),
            EarlyHomeStep::Start(id) => early_homes.start(id),
        },
    )
}

/// A step of [`keep_notes_text`] on the early write job of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EarlyHomeStep<'a> {
    /// Stop the job of the note.
    Cancel(&'a str),
    /// Start the job of the note again, or for the first time.
    Start(&'a str),
}

/// The steps of [`save_notes_keeping_text`] for the note `session_id`,
/// with the notes save `save`, the rescue `rescue` of the notes into the
/// new note that it gets, and the steps `early_home` on the early write
/// jobs. `fingerprint` is the fingerprint of the document that the editor
/// sends.
fn keep_notes_text(
    session_id: &str,
    fingerprint: String,
    mut save: impl FnMut() -> Result<pipeline::save::NotesSave, String>,
    rescue: impl FnOnce(&str) -> Result<pipeline::save::NotesRescue, String>,
    mut early_home: impl FnMut(EarlyHomeStep<'_>),
) -> Result<NotesSaveResult, String> {
    use pipeline::save::{NotesRescue, NotesSave};
    let mut saved = save()?;
    if saved == NotesSave::FolderGone {
        // The early write job must not write into the folder of the note
        // while the rescue reads the storage folder.
        early_home(EarlyHomeStep::Cancel(session_id));
        let new_id = uuid::Uuid::new_v4().to_string();
        let rescued = rescue(&new_id);
        if !matches!(rescued, Ok(NotesRescue::NewNote)) {
            // The note stays: its job syncs the folder again when it can.
            early_home(EarlyHomeStep::Start(session_id));
        }
        match rescued? {
            NotesRescue::Moved => saved = save()?,
            // The store keeps the text until the folder is available.
            NotesRescue::Unavailable => return Err(FOLDER_UNAVAILABLE.to_string()),
            NotesRescue::NewNote => {
                early_home(EarlyHomeStep::Start(&new_id));
                return Ok(NotesSaveResult::NewNote {
                    session_id: new_id,
                    fingerprint,
                });
            }
        }
    }
    Ok(match saved {
        NotesSave::Saved { .. } => NotesSaveResult::Saved { fingerprint },
        NotesSave::Conflict { draft } => NotesSaveResult::Conflict {
            draft: draft.to_string_lossy().into_owned(),
        },
        NotesSave::FolderGone => return Err(FOLDER_UNAVAILABLE.to_string()),
    })
}

/// What a rename did to the session's folder. The title changes in every
/// case.
#[derive(Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FolderRenameResult {
    /// The folder already has the name the title gives.
    Unchanged,
    /// The session records, waits, processes or regenerates its summary:
    /// the folder takes the new name when that ends.
    Later,
    /// The folder has the new name at `path`.
    Moved { path: String },
    /// The folder could not be renamed and keeps its old name.
    Kept { message: String },
}

impl From<FolderRename> for FolderRenameResult {
    fn from(rename: FolderRename) -> Self {
        match rename {
            FolderRename::Unchanged => Self::Unchanged,
            FolderRename::Later => Self::Later,
            FolderRename::Moved(path) => Self::Moved {
                path: path.to_string_lossy().into_owned(),
            },
            FolderRename::Kept(message) => Self::Kept { message },
        }
    }
}

/// Rename a session (R9, R35): `title` is stored as the user's title, and
/// an empty title clears it. The folder of a saved, idle session takes
/// the new name now; any other session's folder takes it later. The rename
/// runs off the main thread, since it can reach a synced folder.
#[tauri::command]
#[specta::specta]
pub async fn rename_session(
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
    title: String,
) -> Result<FolderRenameResult, String> {
    let processing = Arc::clone(&processing);
    tauri::async_runtime::spawn_blocking(move || {
        processing
            .rename_session(&session_id, &title)
            .map(FolderRenameResult::from)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What a type change did (R5). The note has the new type in every case
/// but `Unchanged`.
#[derive(Serialize, Type, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeChangeResult {
    /// The note has this type already. A waiting change back to the type
    /// of its folder is cleared.
    Unchanged,
    /// The home folder is in the folder of the new type, or is made there
    /// when the note writes its folder.
    Done,
    /// A computer records or processes the note: its folder moves when the
    /// note is free.
    Later,
    /// The folder could not move now. It moves later. The text says why.
    Kept { message: String },
}

impl From<pipeline::type_move::TypeChange> for TypeChangeResult {
    fn from(change: pipeline::type_move::TypeChange) -> Self {
        use pipeline::type_move::TypeChange;
        match change {
            TypeChange::Unchanged => Self::Unchanged,
            TypeChange::NoFolder | TypeChange::Moved(_) => Self::Done,
            TypeChange::Later => Self::Later,
            TypeChange::Kept(message) => Self::Kept { message },
        }
    }
}

/// Change the type of a note to `note_type`, or to General for `None`
/// (R5). The type must be a valid type of the effective list (KTD8,
/// KTD9). The app shows the new type at once. The folder of a note that no
/// computer works on moves now, and any other folder moves later. The
/// change runs off the main thread, since it can reach a synced folder.
#[tauri::command]
#[specta::specta]
pub async fn set_note_type(
    app: AppHandle,
    processing: State<'_, Arc<ProcessingManager>>,
    shared: State<'_, Arc<crate::managers::shared::SharedSettingsManager>>,
    session_id: String,
    note_type: Option<String>,
) -> Result<TypeChangeResult, String> {
    let to = match note_type {
        None => None,
        Some(name) => {
            let folders = crate::commands::settings::storage_folders(&app)?;
            Some(crate::commands::capture::named_type(
                &shared.note_types(&folders),
                &name,
            )?)
        }
    };
    let processing = Arc::clone(&processing);
    tauri::async_runtime::spawn_blocking(move || {
        processing
            .change_note_type(&session_id, to.as_deref())
            .map(TypeChangeResult::from)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Add the tags `add` to a note and remove the tags `remove` (R10, R15).
/// This works during a recording and during processing. A tag of `add`
/// whose key the note has already changes nothing, and `remove` removes the
/// tags with the same key (R30). Returns the tags of the note after the
/// edit. The edit runs off the main thread, since it can reach a synced
/// folder.
#[tauri::command]
#[specta::specta]
pub async fn set_note_tags(
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
    add: Vec<String>,
    remove: Vec<String>,
) -> Result<Vec<String>, String> {
    let add = add
        .iter()
        .map(|tag| pipeline::tags::check_tag(tag))
        .collect::<Result<Vec<_>, _>>()?;
    let processing = Arc::clone(&processing);
    tauri::async_runtime::spawn_blocking(move || {
        processing.set_note_tags(&session_id, &add, &remove)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One tag of the tag list (R12, KTD10).
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type)]
pub struct TagEntry {
    /// The spelling on the note with the earliest start that has the tag
    /// (R30).
    pub text: String,
    /// The key of the tag: the text with no spaces at its ends or around
    /// `::`, in lower case. Two tags with one key are the same tag.
    pub key: String,
    /// The number of notes with the tag (R31).
    pub count: u32,
}

impl From<pipeline::tags::TagCount> for TagEntry {
    fn from(tag: pipeline::tags::TagCount) -> Self {
        Self {
            text: tag.text,
            key: tag.key,
            count: u32::try_from(tag.count).unwrap_or(u32::MAX),
        }
    }
}

/// The tag list (R12): each tag that is on at least one note, sorted by its
/// text. This reads the store only.
#[tauri::command]
#[specta::specta]
pub async fn list_tags(capture: State<'_, Arc<CaptureManager>>) -> Result<Vec<TagEntry>, String> {
    let store = capture.shared_store();
    blocking(move || tag_entries(&store)).await
}

/// The tag list of the store `store`.
pub fn tag_entries(store: &pipeline::SharedStore) -> Result<Vec<TagEntry>, String> {
    let store = pipeline::lock(store);
    pipeline::tags::tag_list(&store)
        .map(|list| list.into_iter().map(TagEntry::from).collect())
        .map_err(|e| e.to_string())
}

/// The live transcript of a note so far: its saved parts and the text of
/// the open part (KTD10). The Transcript tab reads it when it opens during
/// recording, and then follows the live events.
#[tauri::command]
#[specta::specta]
pub async fn live_transcript(
    capture: State<'_, Arc<CaptureManager>>,
    live: State<'_, Arc<LiveManager>>,
    session_id: String,
) -> Result<LiveTranscript, String> {
    let (store, live) = (capture.shared_store(), Arc::clone(&live));
    blocking(move || live.transcript(&store, &session_id)).await
}

/// The saved transcript and summary, read from the session's folder.
#[tauri::command]
#[specta::specta]
pub async fn session_document(
    capture: State<'_, Arc<CaptureManager>>,
    session_id: String,
) -> Result<SessionDocument, String> {
    let store = capture.shared_store();
    blocking(move || {
        let home = {
            let store = store.lock().unwrap_or_else(|e| e.into_inner());
            store
                .session(&session_id)
                .map_err(|e| e.to_string())?
                .and_then(|r| r.home_path)
                .ok_or_else(|| "this note has not finished processing yet".to_string())?
        };
        read_document(Path::new(&home))
    })
    .await
}

/// The documents of the note folder `folder`, as they are on disk. While
/// the sync delivers the files, a document can differ from its fingerprint
/// in `session.md`. The note then shows the file that is there, with no
/// error (R6). Each read stops after [`READ_TIMEOUT`].
fn read_document(folder: &Path) -> Result<SessionDocument, String> {
    let read = |name: &str| match read_timed(folder.join(name), READ_TIMEOUT) {
        Read::Bytes(bytes) => Ok(bytes),
        Read::Missing => Err(format!("{name} is not in {}", folder.display())),
        Read::Failed(why) => Err(format!(
            "could not read {name} in {}: {why}",
            folder.display()
        )),
    };
    let text = |name: &str| {
        read(name).and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()))
    };
    let files = match read_note(folder, READ_TIMEOUT) {
        NoteRead::Note(files) => *files,
        NoteRead::Unreadable(why) => {
            return Err(format!(
                "could not read the note in {}: {why}",
                folder.display()
            ));
        }
        NoteRead::Invalid(invalid) => {
            return Err(format!(
                "could not read the note in {}: {invalid}",
                folder.display()
            ));
        }
        NoteRead::NotANote | NoteRead::WaitingForRecord => {
            return Err(format!("{NOTE_RECORD_FILE} is not in {}", folder.display()));
        }
    };
    let transcript_markdown = text(pipeline::join::TRANSCRIPT_FILE)?;
    let transcript = Transcript::parse(&transcript_markdown);
    let summary_markdown = text(pipeline::summarize::SUMMARY_FILE).ok();
    Ok(SessionDocument {
        has_speakers: transcript.has_speakers,
        lines: doc_lines(&files.doc, transcript),
        transcript_markdown,
        summary: summary_of(&files, summary_markdown.as_deref()).map(summary_view),
    })
}

/// The summary of a note, from its `summary.md` text. In the new layout,
/// the summary data comes from `longhand.json` and the file is the body
/// (R1). In the old layout, the data is the frontmatter of `summary.md`
/// (KTD6). `None` when the note has no summary or no summary data.
fn summary_of(files: &NoteFiles, text: Option<&str>) -> Option<Summary> {
    let text = text?;
    match files.layout {
        Layout::New => Some(Summary {
            meta: files.record.summary.clone()?,
            // A frontmatter that an agent added is not shown (KTD7).
            body: Summary::body_of(text).to_string(),
        }),
        Layout::Old => Summary::parse(text).ok(),
    }
}

/// Transcript lines, each gap placed at the end of the segment before it
/// and given the wall-clock start of the segment after it. Gap and pause
/// lines pair with the segment pairs in order.
fn doc_lines(doc: &SessionDoc, transcript: Transcript) -> Vec<DocLine> {
    let mut ordered: Vec<_> = doc.segments.iter().collect();
    ordered.sort_by_key(|s| s.offset_ms);
    let mut gaps = ordered.windows(2).map(|pair| {
        (
            pair[0].offset_ms + pair[0].duration_ms,
            pair[1].wall_start.to_rfc3339(),
        )
    });
    transcript
        .lines
        .into_iter()
        .map(|line| match line {
            TranscriptLine::Turn {
                position_ms,
                speaker,
                text,
            } => DocLine::Turn {
                position_ms,
                speaker,
                text,
            },
            TranscriptLine::Gap { duration_ms } => {
                let (at_ms, wall_start) =
                    gaps.next().map_or((0, None), |(at, wall)| (at, Some(wall)));
                DocLine::Gap {
                    at_ms,
                    duration_ms,
                    wall_start,
                }
            }
            TranscriptLine::Pause { duration_ms } => {
                // A pause line is also between two segments: it takes its
                // pair, so that a later gap gets the pair after it.
                gaps.next();
                DocLine::Pause { duration_ms }
            }
            TranscriptLine::Silence { duration_ms } => DocLine::Silence { duration_ms },
            TranscriptLine::Text { text } => DocLine::Text { text },
        })
        .collect()
}

fn summary_view(summary: Summary) -> SummaryView {
    let body = summary.body;
    match summary.meta {
        SummaryMeta::Complete {
            endpoint_host,
            model,
            prompt_name,
            prompt_id,
        } => SummaryView {
            body,
            stub: false,
            host: Some(endpoint_host),
            model: Some(model),
            prompt: prompt_name,
            prompt_id,
        },
        SummaryMeta::Stub { .. } => SummaryView {
            body,
            stub: true,
            host: None,
            model: None,
            prompt: None,
            prompt_id: None,
        },
    }
}

/// Why a delete did not complete (R8, R33). Nothing after the failed step
/// changed.
#[derive(Debug, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeleteSessionError {
    /// The session records, is processed, or has its summary regenerated.
    /// Nothing changed.
    Busy { message: String },
    /// `path` cannot go to the Trash (R33). It and the session remain, and
    /// `delete_session_permanently` can delete them. When `path` is the home
    /// folder, which goes first, nothing changed.
    CannotMoveToTrash { path: String, reason: String },
    /// A permanent delete or the store failed.
    Failed { message: String },
}

fn busy_message(state: &str) -> String {
    match state {
        "capturing" => "This note is recording. Stop the recording before you delete it.",
        "interrupted" => {
            "This note's recording was interrupted. Stop it before you delete the note."
        }
        "queued" => {
            "This note is waiting to be processed. You can delete it when processing is done."
        }
        "regenerating" => {
            "This note's summary is being made again. You can delete it when that is done."
        }
        "moving" => {
            "This note is being moved to the new storage folder. You can delete it when that is done."
        }
        IN_USE_REASON => {
            "This note is recording or being processed on another computer. You can delete it when that is done."
        }
        _ => "This note is being processed. You can delete it when processing is done.",
    }
    .to_string()
}

/// How a delete removes a directory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Removal {
    Trash,
    Permanent,
}

fn delete_error(error: session_core::DeleteError, removal: Removal) -> DeleteSessionError {
    use session_core::DeleteError;
    match error {
        DeleteError::Busy { state } => DeleteSessionError::Busy {
            message: busy_message(state),
        },
        DeleteError::Remove { path, message } if removal == Removal::Trash => {
            DeleteSessionError::CannotMoveToTrash {
                path: path.to_string_lossy().into_owned(),
                reason: message,
            }
        }
        DeleteError::Remove { path, message } => DeleteSessionError::Failed {
            message: format!("Could not delete {}: {message}", path.display()),
        },
        DeleteError::InvalidId => DeleteSessionError::Failed {
            message: "This note cannot be deleted in Longhand.".to_string(),
        },
        DeleteError::Store(e) => DeleteSessionError::Failed {
            message: e.to_string(),
        },
    }
}

/// Move a directory to the Trash (KTD9). On macOS, NSFileManager does it:
/// the default Finder method runs AppleScript and asks for Automation
/// permission.
fn move_to_trash(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let context = {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut context = trash::TrashContext::default();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context
    };
    #[cfg(not(target_os = "macos"))]
    let context = trash::TrashContext::default();
    context.delete(path).map_err(|e| match e {
        trash::Error::Unknown { description } | trash::Error::Os { description, .. } => description,
        other => other.to_string(),
    })
}

fn remove_permanently(path: &Path) -> Result<(), String> {
    std::fs::remove_dir_all(path).map_err(|e| e.to_string())
}

/// The work on the note `id` on another computer, as the latest library
/// scan found it (KTD5).
fn remote_of(
    store: &pipeline::SharedStore,
    status: &LibraryStatus,
    id: &str,
) -> Option<RemoteView> {
    let record = store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .session(id)
        .ok()
        .flatten()?;
    status.remote(&record).map(RemoteView::from)
}

/// Whether a saved note `id` is in use on another computer at `now`, from
/// its folder: Delete waits for it (R21). A note that records or is
/// processed on this computer has the marker of one of `own_runs`, which
/// does not count.
/// A folder that cannot be read does not refuse the delete: the delete
/// then reports what it cannot do. Delete of a note that another computer
/// did not finish is allowed (R23): it has no fresh marker.
fn in_use_elsewhere(
    store: &pipeline::SharedStore,
    own_runs: &[String],
    id: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> bool {
    let record = store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .session(id)
        .ok()
        .flatten();
    record.is_some_and(|r| {
        r.state == SessionState::Saved && folder_in_use(&r, own_runs, now) == Ok(true)
    })
}

/// Remove a session on a blocking thread: the Trash call on Windows uses
/// COM, and a folder on a network volume can be slow.
async fn delete(
    capture: &CaptureManager,
    processing: &ProcessingManager,
    session_id: String,
    removal: Removal,
) -> Result<(), DeleteSessionError> {
    let store = capture.shared_store();
    let working = capture.working_root().join(&session_id);
    let claims = processing.claims();
    let early_homes = Arc::clone(processing.early_homes());
    let in_use_by_scan =
        remote_of(&store, processing.library_status(), &session_id) == Some(RemoteView::InUse);
    let own_runs = processing.own_runs().to_vec();
    let remove = match removal {
        Removal::Trash => move_to_trash,
        Removal::Permanent => remove_permanently,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let now = chrono::Local::now().fixed_offset();
        if in_use_by_scan || in_use_elsewhere(&store, &own_runs, &session_id, now) {
            return Err(session_core::DeleteError::Busy {
                state: IN_USE_REASON,
            });
        }
        // The early write job writes nothing more into the folder of the
        // note after this (KTD4).
        early_homes.cancel(&session_id);
        session_core::jobs::delete_session(&store, &session_id, &working, &claims, remove)
            .inspect(|()| log::info!("deleted session {session_id} ({removal:?})"))
    })
    .await
    .map_err(|e| DeleteSessionError::Failed {
        message: e.to_string(),
    })?
    .map_err(|e| delete_error(e, removal))
}

/// Move the session's folder to the Trash and remove the session from
/// Longhand (R8). Refused while it records or is processed.
#[tauri::command]
#[specta::specta]
pub async fn delete_session(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<(), DeleteSessionError> {
    delete(&capture, &processing, session_id, Removal::Trash).await
}

/// Delete the session's folder permanently, for a folder that cannot go
/// to the Trash (R33).
#[tauri::command]
#[specta::specta]
pub async fn delete_session_permanently(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<(), DeleteSessionError> {
    delete(&capture, &processing, session_id, Removal::Permanent).await
}

/// What leaving a note did (R18).
#[derive(Debug, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LeaveOutcome {
    /// The note was empty: the note and its home folder are gone.
    Removed,
    /// The note stays: it is not empty, or another computer can have used
    /// it, or its storage folder is not available.
    Kept,
}

/// The user left the note `session_id` (R18): the main window shows
/// another view or closes, or the user starts a new note. An empty note
/// that this computer started goes, with its home folder, when every check
/// passes. The frontend calls it when the shown note changes and when the
/// main window hides or closes. The note stays when [`ShownNote`] names it:
/// the user opened the note again before the removal ended.
#[tauri::command]
#[specta::specta]
pub async fn leave_note(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    shown: State<'_, ShownNote>,
    session_id: String,
) -> Result<LeaveOutcome, String> {
    let capture = Arc::clone(&capture);
    let processing = Arc::clone(&processing);
    let shown = shown.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        leave(&capture, &processing, &session_id, &|id| shown.shows(id))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// [`leave_note`] on the calling thread, for a note that is left while the
/// main window still shows it. The recording shortcut calls it for the
/// shown note before it starts a new note. It reads the storage folder:
/// call it off the main thread.
pub fn leave_note_now(
    capture: &CaptureManager,
    processing: &ProcessingManager,
    session_id: &str,
) -> Result<LeaveOutcome, String> {
    leave(capture, processing, session_id, &|_| false)
}

/// Leave the note `session_id`. The note stays while `shown(session_id)` is
/// true.
fn leave(
    capture: &CaptureManager,
    processing: &ProcessingManager,
    session_id: &str,
    shown: &dyn Fn(&str) -> bool,
) -> Result<LeaveOutcome, String> {
    Ok(
        match processing.leave_note_unless_shown(session_id, shown)? {
            pipeline::empty_note::Leave::Removed => {
                // The working folder of a note with no audio is normally gone
                // already.
                let _ = std::fs::remove_dir_all(capture.session_dir(session_id));
                LeaveOutcome::Removed
            }
            pipeline::empty_note::Leave::Kept(why) => {
                log::debug!("the note {session_id} stays after the user left it: {why:?}");
                LeaveOutcome::Kept
            }
        },
    )
}

/// The note that the main window shows (KTD7). The main window keeps its
/// view in the frontend, so the frontend reports each change with
/// [`set_shown_note`]. The recording shortcut reads it (R10) and uses it
/// only while the main window is visible and not minimized. The removal
/// of empty notes reads it too: a note that the window shows stays. A
/// clone reads the same state.
#[derive(Default, Clone)]
pub struct ShownNote(Arc<std::sync::Mutex<Option<String>>>);

impl ShownNote {
    pub fn get(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// True when the frontend reports that the main window shows `id`.
    pub fn shows(&self, id: &str) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).as_deref() == Some(id)
    }

    fn set(&self, id: Option<String>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = id;
    }
}

/// The main window shows the note `session_id`, or no note (KTD7). The
/// frontend calls it on each view change, and with `None` when the main
/// window hides or closes.
#[tauri::command]
#[specta::specta]
pub fn set_shown_note(shown: State<'_, ShownNote>, session_id: Option<String>) {
    shown.set(session_id);
}

/// Why Remove Audio did not complete (R13).
#[derive(Debug, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoveAudioFailure {
    /// The removal did not start. Nothing changed.
    Refused { message: String },
    /// The removal started and did not complete for each segment. A
    /// segment that has its marker counts as removed, and the next pass
    /// deletes its file (KTD2).
    Failed { message: String },
}

/// The reason that the guard gives while a Move runs or is paused.
const MOVE_OPEN_REASON: &str = "move open";

/// The reason that the guard gives while another computer uses the note.
const IN_USE_REASON: &str = "in use";

/// The text of a refusal of Remove Audio. `reason` is a state name, a
/// claim name or a guard reason.
fn removal_refusal(reason: &str) -> String {
    match reason {
        "capturing" => "This note is recording. Stop the recording before you remove its audio.",
        "interrupted" => {
            "This note’s recording was interrupted. Stop it before you remove its audio."
        }
        "queued" | "processing" | "blocked" => {
            "This note is being processed. You can remove its audio when processing is done."
        }
        "regenerating" => {
            "This note’s summary is being made again. You can remove its audio when that is done."
        }
        "moving" | MOVE_OPEN_REASON => {
            "Longhand is moving your notes. You can remove the audio when the move is done."
        }
        "deleting" => "This note is being deleted.",
        IN_USE_REASON => "This note is in use on another computer. Try again later.",
        _ => "You can remove the audio of this note only after it is saved.",
    }
    .to_string()
}

/// The text of a note whose storage folder is unavailable (R16).
const FOLDER_UNAVAILABLE: &str = "The storage folder for this note is unavailable.";

/// The checks of Remove Audio that the removal step does after its claim
/// and its check that the note is saved. The folder check runs with no
/// store lock, and the store check runs under the store lock.
fn removal_guard() -> pipeline::remove_audio::Guard<
    pipeline::remove_audio::FolderCheck,
    pipeline::remove_audio::StoreCheck,
> {
    pipeline::remove_audio::Guard {
        folder: |record| removal_folder_checks(record, chrono::Local::now().fixed_offset()),
        store: |store, _| removal_store_checks(store),
    }
}

/// The store checks of Remove Audio. The removal pass uses them too (KTD5).
/// The caller holds the store lock.
pub(crate) fn removal_store_checks(store: &session_core::Store) -> Result<(), &'static str> {
    // A Move changes the home folders of the notes (KTD11). When the
    // journal cannot be read, the guard also refuses.
    if !matches!(store.move_journal(), Ok(None)) {
        return Err(MOVE_OPEN_REASON);
    }
    Ok(())
}

/// The folder checks of Remove Audio at the time `now`. The removal pass
/// uses them too (KTD5). They read the home folder, so the caller holds no
/// store lock.
pub(crate) fn removal_folder_checks(
    record: &SessionRecord,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<(), &'static str> {
    // A fresh in-use marker shows that a computer records into or processes
    // the note (KTD4). A folder that cannot be read also refuses. A missing
    // folder is not in use: the step then refuses it as unavailable.
    if let Some(home) = record.home_path.as_deref() {
        match pipeline::in_use::in_use(std::path::Path::new(home), now) {
            Ok(false) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Ok(true) | Err(_) => return Err(IN_USE_REASON),
        }
    }
    Ok(())
}

/// Remove the audio of the note `session_id` with the checks of Remove
/// Audio. The setting of audio removal has no effect here (R13). `sync`
/// controls only the delete of `recognition.jsonl` (KTD8).
fn run_removal(
    store: &pipeline::SharedStore,
    claims: &session_core::jobs::Claims,
    session_id: &str,
    sync: pipeline::remove_audio::NoteSync,
) -> Result<(), RemoveAudioFailure> {
    use pipeline::remove_audio::RemoveAudioError;
    let removal =
        pipeline::remove_audio::remove_audio(store, claims, session_id, removal_guard(), sync)
            .map_err(|e| match e {
                RemoveAudioError::Busy(reason) => RemoveAudioFailure::Refused {
                    message: removal_refusal(reason),
                },
                RemoveAudioError::NoFolder => RemoveAudioFailure::Refused {
                    message: FOLDER_UNAVAILABLE.to_string(),
                },
                RemoveAudioError::Failed(message) => RemoveAudioFailure::Failed {
                    message: format!("Could not remove the audio of this note: {message}"),
                },
            })?;
    if removal.complete() {
        log::info!("removed the audio of session {session_id}");
        return Ok(());
    }
    for segment in &removal.segments {
        log::warn!(
            "the audio {} of session {session_id} was not removed: {:?}",
            segment.audio_file,
            segment.outcome
        );
    }
    Err(RemoveAudioFailure::Failed {
        message: "Some of the audio of this note could not be removed.".to_string(),
    })
}

/// Remove the audio of a saved note now, whether the setting of audio
/// removal is on or off (R13). The transcript, summary and notes stay.
/// Refused while the note is busy, while a Move runs or is paused, or while
/// its storage folder is unavailable.
#[tauri::command]
#[specta::specta]
pub async fn remove_audio(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<(), RemoveAudioFailure> {
    let store = capture.shared_store();
    let home = {
        let store = store.lock().unwrap_or_else(|e| e.into_inner());
        store
            .session(&session_id)
            .map_err(|e| RemoveAudioFailure::Failed {
                message: e.to_string(),
            })?
            .and_then(|r| r.home_path)
    };
    if processing
        .library_status()
        .storage_unavailable(&session_id, home.as_deref().map(Path::new))
    {
        return Err(RemoveAudioFailure::Refused {
            message: FOLDER_UNAVAILABLE.to_string(),
        });
    }
    let sync = crate::managers::audio_removal::note_sync(processing.library_status(), &session_id);
    let claims = processing.claims();
    // A folder on a network volume can be slow.
    tauri::async_runtime::spawn_blocking(move || run_removal(&store, &claims, &session_id, sync))
        .await
        .map_err(|e| RemoveAudioFailure::Failed {
            message: e.to_string(),
        })?
}

#[cfg(test)]
mod remove_audio_tests {
    use super::*;
    use pipeline::remove_audio::NoteSync;
    use session_core::folder::{SessionDoc, fingerprint};
    use session_core::jobs::{Adoption, Claims};
    use session_core::recognition::ARTIFACT_FILE;

    const ID: &str = "20260901-0900-abcd";

    struct Note {
        _root: tempfile::TempDir,
        home: PathBuf,
        store: pipeline::SharedStore,
    }

    /// A saved note folder with the audio file of its one segment, and its
    /// row in the store.
    fn saved_note() -> Note {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap().join("Planning");
        std::fs::create_dir_all(home.join("audio")).unwrap();
        let end = chrono::DateTime::parse_from_rfc3339("2026-09-01T09:01:00-05:00").unwrap();
        let segments = [session_core::Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: end - chrono::TimeDelta::minutes(1),
            wall_end: end,
            audio_file: "audio/segment-01.opus".into(),
            channels: 1,
            ends_in_pause: false,
        }];
        let transcript = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
        let summary = "---\nstatus: stub\nfailed_stage: summarize\n---\n\nNo summary.\n";
        let mut doc = SessionDoc::new(ID, None, &segments).unwrap();
        doc.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
        doc.summary_fingerprint = Some(fingerprint(summary.as_bytes()));
        session_save::note_files::write_doc(&home, &doc).unwrap();
        std::fs::write(home.join("transcript.md"), transcript).unwrap();
        std::fs::write(home.join("summary.md"), summary).unwrap();
        std::fs::write(home.join("notes.md"), "My notes\n").unwrap();
        std::fs::write(home.join("audio/segment-01.opus"), b"opus").unwrap();
        let mut store = session_core::Store::open_in_memory().unwrap();
        let adoption = Adoption::from_doc(
            &doc,
            "My notes\n".into(),
            home.to_string_lossy().into_owned(),
            Some(fingerprint(transcript.as_bytes())),
        );
        assert!(store.adopt(&adoption).unwrap());
        Note {
            _root: root,
            home,
            store: Arc::new(std::sync::Mutex::new(store)),
        }
    }

    fn refused(result: Result<(), RemoveAudioFailure>) -> String {
        match result {
            Err(RemoveAudioFailure::Refused { message }) => message,
            other => panic!("{other:?}"),
        }
    }

    // Covers AE8: the command reads no setting, so it removes the audio
    // whether the setting is on or off.
    #[test]
    fn remove_audio_removes_the_audio_and_keeps_the_documents() {
        let note = saved_note();
        run_removal(&note.store, &Claims::default(), ID, NoteSync::Synced).unwrap();
        assert!(!note.home.join("audio/segment-01.opus").exists());
        for name in [
            "session.md",
            "longhand.json",
            "transcript.md",
            "summary.md",
            "notes.md",
        ] {
            assert!(note.home.join(name).is_file(), "{name}");
        }
        let store = note.store.lock().unwrap();
        assert_eq!(
            pipeline::save::audio_state(&store, ID).unwrap(),
            pipeline::save::AudioState::Removed
        );
    }

    // Covers R11, KTD8: Remove Audio deletes `recognition.jsonl` with the
    // audio, except while the note waits for a sync.
    #[test]
    fn remove_audio_deletes_recognition_jsonl_unless_the_note_waits_for_a_sync() {
        for (sync, stays) in [(NoteSync::Synced, false), (NoteSync::Waiting, true)] {
            let note = saved_note();
            std::fs::write(note.home.join(ARTIFACT_FILE), b"{\"version\":1}\n").unwrap();
            let written = std::fs::read(note.home.join("session.md")).unwrap();
            run_removal(&note.store, &Claims::default(), ID, sync).unwrap();
            assert!(!note.home.join("audio/segment-01.opus").exists());
            assert_eq!(note.home.join(ARTIFACT_FILE).exists(), stays, "{sync:?}");
            assert_eq!(
                std::fs::read(note.home.join("session.md")).unwrap(),
                written
            );
        }
    }

    #[test]
    fn an_open_move_refuses_with_a_reason_and_keeps_the_audio() {
        let note = saved_note();
        note.store
            .lock()
            .unwrap()
            .open_move(Path::new("/old"), Path::new("/new"))
            .unwrap();
        let message = refused(run_removal(
            &note.store,
            &Claims::default(),
            ID,
            NoteSync::Synced,
        ));
        assert!(message.contains("moving your notes"), "{message}");
        assert!(note.home.join("audio/segment-01.opus").is_file());
    }

    #[test]
    fn a_fresh_in_use_marker_of_another_computer_refuses_and_keeps_the_audio() {
        let note = saved_note();
        let now = chrono::Local::now().fixed_offset();
        assert!(pipeline::in_use::write_marker(&note.home, "other-computer", now).unwrap());
        let message = refused(run_removal(
            &note.store,
            &Claims::default(),
            ID,
            NoteSync::Synced,
        ));
        assert!(message.contains("in use on another computer"), "{message}");
        assert!(note.home.join("audio/segment-01.opus").is_file());
    }

    #[test]
    fn a_busy_note_refuses_with_a_reason_and_keeps_the_audio() {
        let note = saved_note();
        let claims = Claims::default();
        let _claim = claims.regenerate(ID).unwrap();
        let message = refused(run_removal(&note.store, &claims, ID, NoteSync::Synced));
        assert!(message.contains("summary"), "{message}");
        assert!(note.home.join("audio/segment-01.opus").is_file());
    }

    #[test]
    fn a_missing_folder_refuses_as_an_unavailable_storage_folder() {
        let note = saved_note();
        std::fs::remove_dir_all(&note.home).unwrap();
        let message = refused(run_removal(
            &note.store,
            &Claims::default(),
            ID,
            NoteSync::Synced,
        ));
        assert_eq!(message, FOLDER_UNAVAILABLE);
        assert!(!note.home.exists(), "the step makes no folder");
    }

    #[test]
    fn each_refusal_says_why_in_plain_words() {
        for (reason, word) in [
            ("capturing", "recording"),
            ("interrupted", "interrupted"),
            ("queued", "processed"),
            ("processing", "processed"),
            ("regenerating", "summary"),
            ("moving", "moving"),
            (MOVE_OPEN_REASON, "moving"),
            ("deleting", "deleted"),
            (IN_USE_REASON, "another computer"),
            ("processing_failed", "saved"),
            ("awaiting_destination", "saved"),
        ] {
            let message = removal_refusal(reason);
            assert!(message.contains(word), "{reason}: {message}");
        }
    }
}

#[cfg(test)]
mod delete_tests {
    use super::*;
    use session_core::DeleteError;

    #[test]
    fn a_trash_failure_is_a_typed_error_naming_the_folder() {
        let e = delete_error(
            DeleteError::Remove {
                path: "/d/Budget review".into(),
                message: "no Trash on this volume".into(),
            },
            Removal::Trash,
        );
        assert!(
            matches!(&e, DeleteSessionError::CannotMoveToTrash { path, reason }
                if path == "/d/Budget review" && reason == "no Trash on this volume"),
            "{e:?}"
        );
        let e = delete_error(
            DeleteError::Remove {
                path: "/d/x".into(),
                message: "denied".into(),
            },
            Removal::Permanent,
        );
        assert!(matches!(e, DeleteSessionError::Failed { .. }), "{e:?}");
    }

    #[test]
    fn a_refusal_says_why_in_plain_words() {
        for (state, word) in [
            ("capturing", "recording"),
            ("interrupted", "interrupted"),
            ("queued", "waiting"),
            ("processing", "processed"),
            ("regenerating", "summary"),
            ("moving", "moved"),
        ] {
            match delete_error(DeleteError::Busy { state }, Removal::Trash) {
                DeleteSessionError::Busy { message } => {
                    assert!(message.contains(word), "{state}: {message}")
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_permanent_delete_removes_the_folder_and_the_rows() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let working = dir.path().join("working");
        std::fs::create_dir_all(home.join("audio")).unwrap();
        std::fs::write(home.join("audio").join("segment-01.opus"), "x").unwrap();
        std::fs::create_dir_all(&working).unwrap();
        let mut store = session_core::Store::open_in_memory().unwrap();
        let created = chrono::DateTime::parse_from_rfc3339("2026-09-22T14:00:00+00:00").unwrap();
        store.create_session("s", created, None).unwrap();
        store
            .apply(
                "s",
                &session_core::Event::UserStop {
                    at: created,
                    retained_audio: false,
                },
            )
            .unwrap();
        store.set_home_path("s", home.to_str().unwrap()).unwrap();
        let store = std::sync::Mutex::new(store);
        let claims = session_core::jobs::Claims::default();
        session_core::jobs::delete_session(&store, "s", &working, &claims, remove_permanently)
            .unwrap();
        assert!(!home.exists() && !working.exists());
        assert!(store.lock().unwrap().session("s").unwrap().is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clone_of_the_shown_note_sees_each_new_report() {
        // The leave check holds a clone while the frontend reports a
        // reopen.
        let shown = ShownNote::default();
        let check = shown.clone();
        assert!(!check.shows("a"));
        shown.set(Some("a".into()));
        assert!(check.shows("a"));
        assert!(!check.shows("b"));
        shown.set(None);
        assert!(!check.shows("a"));
    }

    #[test]
    fn gaps_sit_at_the_end_of_the_segment_before_them() {
        let text = "---\nsession_id: s\nstarted: 2026-09-22T14:00:00+00:00\nsegments:\n- index: 0\n  wall_start: 2026-09-22T14:00:00+00:00\n  wall_end: 2026-09-22T14:10:00+00:00\n  offset_ms: 0\n  duration_ms: 600000\n  audio: audio/segment-01.opus\n- index: 1\n  wall_start: 2026-09-22T14:20:00+00:00\n  wall_end: 2026-09-22T14:30:00+00:00\n  offset_ms: 1200000\n  duration_ms: 600000\n  audio: audio/segment-02.opus\n---\n";
        let doc = SessionDoc::parse(text).unwrap();
        let transcript = Transcript {
            has_speakers: false,
            lines: vec![
                TranscriptLine::Turn {
                    position_ms: 1_000,
                    speaker: None,
                    text: "a".into(),
                },
                TranscriptLine::Gap {
                    duration_ms: 600_000,
                },
                TranscriptLine::Turn {
                    position_ms: 1_201_000,
                    speaker: None,
                    text: "b".into(),
                },
            ],
        };
        let lines = doc_lines(&doc, transcript);
        assert!(matches!(
            &lines[1],
            DocLine::Gap {
                at_ms: 600_000,
                duration_ms: 600_000,
                wall_start: Some(w),
            } if w == "2026-09-22T14:20:00+00:00"
        ));
    }

    #[test]
    fn a_pause_line_takes_its_segment_pair_so_a_later_gap_keeps_its_start() {
        let segment = |index: u32, start: &str, end: &str, offset_ms: u64| {
            format!(
                "- index: {index}\n  wall_start: 2026-09-22T{start}:00+00:00\n  wall_end: 2026-09-22T{end}:00+00:00\n  offset_ms: {offset_ms}\n  duration_ms: 600000\n  audio: audio/segment-0{}.opus\n",
                index + 1
            )
        };
        let text = format!(
            "---\nsession_id: s\nstarted: 2026-09-22T14:00:00+00:00\nsegments:\n{}{}{}---\n",
            segment(0, "14:00", "14:10", 0) + "  ends_in_pause: true\n",
            segment(1, "14:25", "14:35", 1_500_000),
            segment(2, "16:00", "16:10", 6_600_000),
        );
        let doc = SessionDoc::parse(&text).unwrap();
        let turn = |position_ms: u64| TranscriptLine::Turn {
            position_ms,
            speaker: None,
            text: "a".into(),
        };
        let transcript = Transcript {
            has_speakers: false,
            lines: vec![
                turn(1_000),
                TranscriptLine::Pause {
                    duration_ms: 900_000,
                },
                turn(1_501_000),
                TranscriptLine::Gap {
                    duration_ms: 4_500_000,
                },
                turn(6_601_000),
            ],
        };
        let lines = doc_lines(&doc, transcript);
        assert!(matches!(
            &lines[1],
            DocLine::Pause {
                duration_ms: 900_000
            }
        ));
        assert!(matches!(
            &lines[3],
            DocLine::Gap {
                at_ms: 2_100_000,
                duration_ms: 4_500_000,
                wall_start: Some(w),
            } if w == "2026-09-22T16:00:00+00:00"
        ));
    }

    #[test]
    fn a_summary_view_carries_the_recorded_prompt_name_and_id() {
        let complete = |prompt: Option<(&str, &str)>| Summary {
            meta: SummaryMeta::Complete {
                endpoint_host: "api.example.com".into(),
                model: "m".into(),
                prompt_name: prompt.map(|p| p.0.into()),
                prompt_id: prompt.map(|p| p.1.into()),
            },
            body: "We agreed.".into(),
        };
        let named = summary_view(complete(Some(("Team sync", "b1"))));
        assert_eq!(named.prompt.as_deref(), Some("Team sync"));
        assert_eq!(named.prompt_id.as_deref(), Some("b1"));
        assert_eq!(named.host.as_deref(), Some("api.example.com"));
        let older = summary_view(complete(None));
        assert!(older.prompt.is_none());
        assert!(older.prompt_id.is_none());
    }

    // Covers AE1 (R6).
    #[test]
    fn a_document_that_waits_for_the_sync_shows_the_files_as_they_are() {
        use session_core::folder::fingerprint;
        let folder = tempfile::tempdir().unwrap();
        let text = "---\nsession_id: s\nstarted: 2026-09-22T14:00:00+00:00\nsegments:\n- index: 0\n  wall_start: 2026-09-22T14:00:00+00:00\n  wall_end: 2026-09-22T14:10:00+00:00\n  offset_ms: 0\n  duration_ms: 600000\n  audio: audio/segment-01.opus\n---\n";
        let transcript = "# Transcript\n\n- [00:00:01] Speaker 1: We start with the plan.\n";
        let old_summary = "---\nstatus: stub\nfailed_stage: summarize\n---\n\nThe old summary.\n";
        // The new session.md and transcript.md arrived. The new summary.md
        // did not, and an older build named copies that did not arrive.
        let mut doc = SessionDoc::parse(text).unwrap();
        doc.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
        doc.summary_fingerprint = Some(fingerprint(b"# Summary\n\nThe new summary.\n"));
        let session = doc.render().replacen(
            "---\n",
            &format!(
                "---\nresult_generation:\n  id: committed-1\n  files:\n    transcript.md: {}\n    summary.md: {}\n",
                fingerprint(transcript.as_bytes()),
                fingerprint(b"# Summary\n\nThe new summary.\n"),
            ),
            1,
        );
        std::fs::write(folder.path().join("session.md"), session).unwrap();
        std::fs::write(folder.path().join("transcript.md"), transcript).unwrap();
        std::fs::write(folder.path().join("summary.md"), old_summary).unwrap();

        let document = read_document(folder.path()).unwrap();
        assert_eq!(document.transcript_markdown, transcript);
        let summary = document.summary.unwrap();
        assert!(summary.stub);
        assert_eq!(summary.body.trim(), "The old summary.");
    }

    // R1: in the new layout, the summary data comes from `longhand.json`,
    // and `summary.md` is the body only.
    #[test]
    fn a_new_layout_summary_takes_its_data_from_longhand_json() {
        let folder = tempfile::tempdir().unwrap();
        let started = chrono::DateTime::parse_from_rfc3339("2026-09-22T14:00:00+00:00").unwrap();
        let mut record = session_core::note_record::NoteRecord::new("s", started);
        record.summary = Some(SummaryMeta::Complete {
            endpoint_host: "api.example.com".into(),
            model: "m".into(),
            prompt_name: Some("Team sync".into()),
            prompt_id: Some("b1".into()),
        });
        std::fs::write(folder.path().join("session.md"), "---\ntitle: Plan\n---\n").unwrap();
        std::fs::write(folder.path().join("longhand.json"), record.render()).unwrap();
        std::fs::write(folder.path().join("transcript.md"), "# Transcript\n").unwrap();
        std::fs::write(
            folder.path().join("summary.md"),
            "# Summary\n\nWe agreed.\n",
        )
        .unwrap();

        let summary = read_document(folder.path()).unwrap().summary.unwrap();
        assert!(!summary.stub);
        assert_eq!(summary.body, "# Summary\n\nWe agreed.");
        assert_eq!(summary.host.as_deref(), Some("api.example.com"));
        assert_eq!(summary.prompt.as_deref(), Some("Team sync"));
        assert_eq!(summary.prompt_id.as_deref(), Some("b1"));
    }

    // Covers R7, KTD7: an agent added a free-text line to `transcript.md`
    // and a frontmatter to `summary.md`. The line shows as plain text, and
    // the summary shows only its body.
    #[test]
    fn agent_edits_of_the_transcript_and_the_summary_still_show() {
        let folder = tempfile::tempdir().unwrap();
        let started = chrono::DateTime::parse_from_rfc3339("2026-09-22T14:00:00+00:00").unwrap();
        let mut record = session_core::note_record::NoteRecord::new("s", started);
        record.summary = Some(SummaryMeta::Stub {
            failed_stage: "summarize".into(),
        });
        std::fs::write(folder.path().join("session.md"), "---\ntitle: Plan\n---\n").unwrap();
        std::fs::write(folder.path().join("longhand.json"), record.render()).unwrap();
        std::fs::write(
            folder.path().join("transcript.md"),
            "# Transcript\n\n- [00:00:01] Speaker 1: We start.\nA note from the agent.\n",
        )
        .unwrap();
        std::fs::write(
            folder.path().join("summary.md"),
            "---\naliases: [Plan]\n---\n\nWe agreed.\n",
        )
        .unwrap();

        let document = read_document(folder.path()).unwrap();
        assert!(matches!(
            document.lines.last(),
            Some(DocLine::Text { text }) if text == "A note from the agent."
        ));
        assert_eq!(document.summary.unwrap().body, "We agreed.");
    }

    // Covers AE4, R10, R11 (U5): the Convert notice shows only while an old
    // note exists, with the failures of the last run that are still old.
    #[test]
    fn the_convert_notice_shows_only_while_an_old_note_exists() {
        use library::reconcile::{Reconciler, Target};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(library::scan::write_marker(&root));
        let store = std::sync::Mutex::new(session_core::Store::open_in_memory().unwrap());
        let mut reconciler = Reconciler::default();
        let target = Target::Full(vec![root.clone()]);
        let start = std::time::Instant::now();
        reconciler
            .run_at(&store, &target, &|_| false, start)
            .unwrap();
        let status = Arc::clone(reconciler.status());
        let failed = ConversionState {
            running: false,
            skipped: vec!["o".into(), "gone".into()],
            failed: vec![(
                "o".into(),
                root.join("Old note"),
                "the frontmatter cannot change".into(),
            )],
        };
        // No old note: no conversion control.
        assert_eq!(library_notices_of(&status, &failed, &[]).conversion, None);

        let folder = root.join("Old note");
        std::fs::create_dir(&folder).unwrap();
        let started = chrono::DateTime::parse_from_rfc3339("2026-09-22T14:00:00+00:00").unwrap();
        let doc = SessionDoc::without_audio("o", started, None);
        std::fs::write(folder.join("session.md"), doc.render()).unwrap();
        reconciler
            .run_at(&store, &target, &|_| false, start)
            .unwrap();

        let idle = library_notices_of(&status, &ConversionState::default(), &[]);
        let notice = idle.conversion.unwrap();
        assert_eq!(notice.old_notes, 1);
        assert!(!notice.running && notice.skipped == 0 && notice.failed.is_empty());

        let notice = library_notices_of(&status, &failed, &[])
            .conversion
            .unwrap();
        assert_eq!(notice.skipped, 1);
        assert_eq!(
            notice.failed,
            [FailedNote {
                name: "Old note".into(),
                reason: "the frontmatter cannot change".into(),
            }]
        );
    }

    // Covers R6, KTD14: the notice names a note with no known title by its
    // folder, and says what the user can do. Before the wait it is empty.
    #[test]
    fn the_library_notices_name_a_folder_with_no_known_title() {
        use library::reconcile::{Reconciler, Target};
        use library::status::PROBLEM_WAIT;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(library::scan::write_marker(&root));
        let folder = root.join("Planning");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("session.md"), "---\ntitle: Planning\n---\n").unwrap();
        let store = std::sync::Mutex::new(session_core::Store::open_in_memory().unwrap());
        let mut reconciler = Reconciler::default();
        let target = Target::Full(vec![root]);
        let start = std::time::Instant::now();
        reconciler
            .run_at(&store, &target, &|_| false, start)
            .unwrap();
        let status = Arc::clone(reconciler.status());
        assert!(
            library_notices_of(&status, &ConversionState::default(), &[])
                .problems
                .is_empty()
        );

        reconciler
            .run_at(&store, &target, &|_| false, start + PROBLEM_WAIT)
            .unwrap();
        let notices = library_notices_of(&status, &ConversionState::default(), &[]);
        assert_eq!(notices.problems.len(), 1);
        assert_eq!(notices.problems[0].name, "Planning");
        assert_eq!(notices.problems[0].problem.kind, ProblemKindView::Record);
        assert_eq!(
            notices.problems[0].problem.reason,
            "This note waits for longhand.json to sync. If it does not sync, update Longhand on the other computer."
        );
    }
}

#[cfg(test)]
mod entry_tests {
    use super::*;
    use session_core::{Title, TitleSource};

    #[test]
    fn an_entry_carries_the_title_and_the_found_speaker_count() {
        let created = chrono::DateTime::parse_from_rfc3339("2026-09-22T14:00:00+00:00").unwrap();
        let record = SessionRecord {
            id: "s".into(),
            created_at: created,
            state: SessionState::Saved,
            queue_seq: None,
            speaker_count: None,
            asr_model: None,
            recognition_snapshot: None,
            home_path: None,
            title: Some(Title {
                text: "Budget review".into(),
                source: TitleSource::Proposed,
            }),
            found_speakers: Some(3),
            live_transcript: None,
            transcribe_again_model: None,
            started_at: None,
            note_type: None,
            tags: Vec::new(),
            pending_type: None,
        };
        let status: Arc<LibraryStatus> = Arc::default();
        let e = entry(&record, &[], None, &status, &Waits::default());
        assert!(!e.storage_unavailable);
        assert_eq!(e.remote, None);
        assert_eq!(e.title.as_deref(), Some("Budget review"));
        assert_eq!(e.found_speakers, Some(3));
        // A scan finds the storage folder of the note missing.
        let gone = tempfile::tempdir().unwrap().path().join("gone");
        let store = Arc::new(std::sync::Mutex::new(
            session_core::Store::open_in_memory().unwrap(),
        ));
        library::reconcile::Reconciler::default()
            .with_status(Arc::clone(&status))
            .run(
                &store,
                &library::reconcile::Target::Full(vec![gone.clone()]),
                &|_| false,
            )
            .unwrap();
        let e = entry(
            &SessionRecord {
                title: None,
                found_speakers: None,
                home_path: Some(gone.join("note").to_string_lossy().into_owned()),
                ..record
            },
            &[],
            None,
            &status,
            &Waits::default(),
        );
        assert!(e.storage_unavailable);
        assert_eq!(e.title, None);
        assert_eq!(e.found_speakers, None);
    }

    // KTD3: the entry shows a waiting type change while the folder is still
    // in the `from` folder, else the type of the folder.
    #[test]
    fn an_entry_shows_the_waiting_type_while_the_folder_is_in_the_old_type_folder() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-10-07T14:00:00+00:00").unwrap();
        let record = SessionRecord {
            id: "s".into(),
            created_at: at,
            state: SessionState::Saved,
            queue_seq: None,
            speaker_count: None,
            asr_model: None,
            recognition_snapshot: None,
            home_path: None,
            title: None,
            found_speakers: None,
            live_transcript: None,
            transcribe_again_model: None,
            started_at: None,
            note_type: Some("Pathfinder".into()),
            tags: vec!["Person::Nathan".into(), "follow-up".into()],
            pending_type: Some(session_core::folder::PendingType {
                to: Some("D&D".into()),
                from: Some("pathfinder".into()),
                by: "computer-b".into(),
                at,
            }),
        };
        let status: Arc<LibraryStatus> = Arc::default();
        let e = entry(&record, &[], None, &status, &Waits::default());
        assert_eq!(e.note_type.as_deref(), Some("D&D"));
        assert_eq!(
            e.tags,
            vec!["Person::Nathan".to_string(), "follow-up".into()]
        );
        // A person moved the folder: the folder wins.
        let moved = SessionRecord {
            note_type: Some("Board".into()),
            ..record
        };
        let e = entry(&moved, &[], None, &status, &Waits::default());
        assert_eq!(e.note_type.as_deref(), Some("Board"));
    }

    #[test]
    fn a_type_change_result_names_what_the_move_did() {
        use pipeline::type_move::TypeChange;
        assert_eq!(
            TypeChangeResult::from(TypeChange::Moved("/x".into())),
            TypeChangeResult::Done
        );
        assert_eq!(
            TypeChangeResult::from(TypeChange::NoFolder),
            TypeChangeResult::Done
        );
        assert_eq!(
            TypeChangeResult::from(TypeChange::Later),
            TypeChangeResult::Later
        );
        assert_eq!(
            TypeChangeResult::from(TypeChange::Kept("in use".into())),
            TypeChangeResult::Kept {
                message: "in use".into()
            }
        );
    }

    // Covers R12, R20, R23 (KTD5): the entry carries what the latest scan
    // found about work on the note on another computer, for a Saved row
    // only.
    #[test]
    fn an_entry_carries_the_remote_state_of_a_saved_note_only() {
        let root = tempfile::tempdir().unwrap();
        library::scan::write_marker(root.path());
        let home = root.path().join("2026-09-25 0900 Longhand session");
        std::fs::create_dir(&home).unwrap();
        let opened = chrono::DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap();
        let mut doc = SessionDoc::without_audio("s", opened, None);
        doc.result = Some(session_core::folder::ResultFlag::NotSaved);
        session_save::note_files::write_doc(&home, &doc).unwrap();
        std::fs::write(home.join("notes.md"), "").unwrap();
        let store = Arc::new(std::sync::Mutex::new(
            session_core::Store::open_in_memory().unwrap(),
        ));
        let status: Arc<LibraryStatus> = Arc::default();
        library::reconcile::Reconciler::default()
            .with_status(Arc::clone(&status))
            .run(
                &store,
                &library::reconcile::Target::Full(vec![root.path().to_path_buf()]),
                &|_| false,
            )
            .unwrap();
        let record = store.lock().unwrap().session("s").unwrap().unwrap();

        let e = entry(&record, &[], None, &status, &Waits::default());
        assert_eq!(e.remote, Some(RemoteView::InUse));
        assert!(!e.storage_unavailable);
        let here = SessionRecord {
            state: SessionState::Capturing,
            ..record
        };
        assert_eq!(
            entry(&here, &[], None, &status, &Waits::default()).remote,
            None
        );
    }
}

#[cfg(test)]
mod record_tests {
    use super::*;
    use pipeline::save::AudioState;

    const RUN: &str = "this-run";
    const EARLIER_RUN: &str = "earlier-run";

    /// This computer's runs: this run and its earlier run.
    fn own() -> Vec<String> {
        vec![RUN.to_string(), EARLIER_RUN.to_string()]
    }

    fn opened() -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339("2026-09-30T09:00:00-05:00").unwrap()
    }

    fn segment() -> session_core::Segment {
        session_core::Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: opened(),
            wall_end: opened() + chrono::TimeDelta::minutes(1),
            audio_file: "audio/segment-01.opus".into(),
            channels: 1,
            ends_in_pause: false,
        }
    }

    /// A saved note: with a segment when `audio`, else a notes-only note.
    fn saved(audio: bool) -> SessionRecord {
        let mut store = session_core::Store::open_in_memory().unwrap();
        if audio {
            store.create_session("s", opened(), None).unwrap();
            store.put_segment("s", &segment()).unwrap();
            store
                .apply(
                    "s",
                    &Event::UserStop {
                        at: opened(),
                        retained_audio: true,
                    },
                )
                .unwrap();
            store.apply("s", &Event::Dequeued).unwrap();
            store.apply("s", &Event::Saved).unwrap();
        } else {
            store.create_notes_only("s", opened(), None).unwrap();
        }
        store.session("s").unwrap().unwrap()
    }

    fn button(record: &SessionRecord, has_segments: bool, audio: AudioState) -> RecordButton {
        let status: Arc<LibraryStatus> = Arc::default();
        record_button(record, has_segments, false, audio, &status).unwrap()
    }

    // Covers AE3 and R11, R12.
    #[test]
    fn the_button_says_record_or_resume_and_why_it_is_unavailable() {
        let with_audio = button(&saved(true), true, AudioState::Present);
        assert_eq!(
            with_audio,
            RecordButton {
                kind: RecordKind::Resume,
                available: true,
                reason: None,
            }
        );
        let no_audio = button(&saved(false), false, AudioState::Present);
        assert_eq!(
            no_audio,
            RecordButton {
                kind: RecordKind::Record,
                available: true,
                reason: None,
            }
        );
        let removed = button(&saved(true), true, AudioState::Removed);
        assert_eq!(
            removed,
            RecordButton {
                kind: RecordKind::Resume,
                available: false,
                reason: Some(AUDIO_REMOVED.to_string()),
            }
        );
    }

    #[test]
    fn a_note_that_records_or_is_interrupted_has_no_button() {
        let status: Arc<LibraryStatus> = Arc::default();
        let capturing = SessionRecord {
            state: SessionState::Capturing,
            ..saved(false)
        };
        assert_eq!(
            record_button(&capturing, false, false, AudioState::Present, &status),
            None
        );
        // A note whose save has begun cannot resume.
        let queued = SessionRecord {
            state: SessionState::Queued {
                stopped_at: opened(),
            },
            ..saved(true)
        };
        let saving = record_button(&queued, true, true, AudioState::Present, &status).unwrap();
        assert_eq!(saving.reason.as_deref(), Some(CANNOT_RESUME));
        let waiting = record_button(&queued, true, false, AudioState::Present, &status).unwrap();
        assert!(waiting.available);
    }

    // Covers R12, R21, R23: the work of another computer comes first, then
    // the storage folder and the sync.
    #[test]
    fn each_refusal_of_the_button_has_its_reason() {
        let check = |remote, storage_unavailable, waiting: &[WaitingPart]| {
            record_refusal(&RecordCheck {
                kind: RecordKind::Record,
                allowed: true,
                audio: AudioView::Present,
                storage_unavailable,
                waiting,
                remote,
            })
        };
        assert_eq!(check(None, false, &[]), None);
        assert_eq!(
            check(Some(RemoteView::InUse), true, &[]),
            Some(IN_USE_ELSEWHERE)
        );
        assert_eq!(
            check(Some(RemoteView::NotFinished), false, &[]),
            Some(NOT_FINISHED_ELSEWHERE)
        );
        assert_eq!(check(None, true, &[]), Some(FOLDER_UNAVAILABLE));
        assert_eq!(
            check(None, false, &[WaitingPart::Notes]),
            Some(crate::managers::processing::WAIT_FOR_SYNC)
        );
        let not_allowed = record_refusal(&RecordCheck {
            kind: RecordKind::Record,
            allowed: false,
            audio: AudioView::Present,
            storage_unavailable: false,
            waiting: &[],
            remote: None,
        });
        assert_eq!(not_allowed, Some(CANNOT_RECORD));
        for reason in [IN_USE_ELSEWHERE, NOT_FINISHED_ELSEWHERE, CANNOT_RECORD] {
            assert!(!reason.contains("session"), "{reason}");
        }
    }

    /// A store with the notes-only note "n" whose home folder is in a
    /// temporary storage folder.
    fn placed_note() -> (tempfile::TempDir, PathBuf, pipeline::SharedStore) {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("2026-09-30 0900");
        std::fs::create_dir(&home).unwrap();
        let mut store = session_core::Store::open_in_memory().unwrap();
        store
            .create_notes_only("n", opened(), Some(home.to_str().unwrap()))
            .unwrap();
        (root, home, Arc::new(std::sync::Mutex::new(store)))
    }

    // Covers R12, R21: Record waits while another computer records into
    // the note, also before a library scan found it.
    #[test]
    fn record_is_refused_while_another_computer_records_into_the_note() {
        let (_root, home, store) = placed_note();
        let status: Arc<LibraryStatus> = Arc::default();
        let now = chrono::Local::now().fixed_offset();
        assert_eq!(check_record(&store, &status, &own(), "n", now), Ok(()));
        // This run's own marker does not count.
        assert!(pipeline::in_use::write_marker(&home, RUN, now).unwrap());
        assert_eq!(check_record(&store, &status, &own(), "n", now), Ok(()));

        assert!(pipeline::in_use::write_marker(&home, "other-computer", now).unwrap());
        assert_eq!(
            check_record(&store, &status, &own(), "n", now),
            Err(IN_USE_ELSEWHERE.to_string())
        );
    }

    #[test]
    fn record_is_refused_for_a_note_with_audio_and_a_note_that_records() {
        let (_root, _home, store) = placed_note();
        let status: Arc<LibraryStatus> = Arc::default();
        let now = chrono::Local::now().fixed_offset();
        store.lock().unwrap().put_segment("n", &segment()).unwrap();
        let e = check_record(&store, &status, &own(), "n", now).unwrap_err();
        assert!(e.contains("Resume"), "{e}");

        let (_root, _home, store) = placed_note();
        store
            .lock()
            .unwrap()
            .apply(
                "n",
                &Event::UserRecord {
                    has_segments: false,
                },
            )
            .unwrap();
        assert_eq!(
            check_record(&store, &status, &own(), "n", now),
            Err(CANNOT_RECORD.to_string())
        );
    }

    // Covers R21, R23: Delete waits while another computer uses the note,
    // and a note that another computer did not finish can be deleted.
    #[test]
    fn delete_waits_while_another_computer_uses_the_note() {
        let (_root, home, store) = placed_note();
        let now = chrono::Local::now().fixed_offset();
        assert!(!in_use_elsewhere(&store, &own(), "n", now));
        assert!(pipeline::in_use::write_marker(&home, RUN, now).unwrap());
        assert!(!in_use_elsewhere(&store, &own(), "n", now));
        assert!(pipeline::in_use::write_marker(&home, "other-computer", now).unwrap());
        assert!(in_use_elsewhere(&store, &own(), "n", now));
        assert!(busy_message(IN_USE_REASON).contains("another computer"));

        // Not finished on another computer: its marker is old.
        let (_root, home, store) = placed_note();
        let old = now - pipeline::in_use::FRESH_FOR - chrono::TimeDelta::hours(1);
        assert!(pipeline::in_use::write_marker(&home, "other-computer", old).unwrap());
        assert!(!in_use_elsewhere(&store, &own(), "n", now));
    }

    // A relaunch: the earlier run on this computer quit while the note
    // recorded or processed, and its marker is still fresh. This run
    // finished the work and the note is Saved. Record and Delete are not
    // refused. A fresh marker of another computer still refuses them.
    #[test]
    fn a_fresh_marker_of_an_earlier_run_on_this_computer_refuses_nothing() {
        let (_root, home, store) = placed_note();
        let status: Arc<LibraryStatus> = Arc::default();
        let now = chrono::Local::now().fixed_offset();
        assert!(pipeline::in_use::write_marker(&home, EARLIER_RUN, now).unwrap());
        assert_eq!(check_record(&store, &status, &own(), "n", now), Ok(()));
        assert!(!in_use_elsewhere(&store, &own(), "n", now));
        // Without the earlier run in this computer's runs, the marker
        // counts as work on another computer.
        let this_run = [RUN.to_string()];
        assert!(in_use_elsewhere(&store, &this_run, "n", now));

        assert!(pipeline::in_use::write_marker(&home, "other-computer", now).unwrap());
        assert_eq!(
            check_record(&store, &status, &own(), "n", now),
            Err(IN_USE_ELSEWHERE.to_string())
        );
        assert!(in_use_elsewhere(&store, &own(), "n", now));
    }

    // Covers R12, R21: Resume waits while another computer records into
    // or processes the note, also before a library scan found it.
    #[test]
    fn resume_is_refused_while_another_computer_uses_the_note() {
        let (_root, home, store) = placed_note();
        store.lock().unwrap().put_segment("n", &segment()).unwrap();
        let status: Arc<LibraryStatus> = Arc::default();
        let now = chrono::Local::now().fixed_offset();
        assert_eq!(check_resume(&store, &status, &own(), "n", now), Ok(()));
        // This run's own marker does not count.
        assert!(pipeline::in_use::write_marker(&home, RUN, now).unwrap());
        assert_eq!(check_resume(&store, &status, &own(), "n", now), Ok(()));

        assert!(pipeline::in_use::write_marker(&home, "other-computer", now).unwrap());
        assert_eq!(
            check_resume(&store, &status, &own(), "n", now),
            Err(IN_USE_ELSEWHERE.to_string())
        );
    }

    /// The saved note "n" with audio, whose folder in a storage folder
    /// says that its recording is not saved, and the library status after
    /// a scan of that folder. `aged`: the records of the folder are older
    /// than the time that a marker stays fresh.
    fn not_saved_elsewhere(
        aged: bool,
    ) -> (tempfile::TempDir, pipeline::SharedStore, Arc<LibraryStatus>) {
        use session_core::folder::ResultFlag;
        let root = tempfile::tempdir().unwrap();
        assert!(library::scan::write_marker(root.path()));
        let home = root.path().join("2026-09-30 0900 n");
        std::fs::create_dir(&home).unwrap();
        let mut doc = SessionDoc::without_audio("n", opened(), None);
        doc.result = Some(ResultFlag::NotSaved);
        session_save::note_files::write_doc(&home, &doc).unwrap();
        if aged {
            let age =
                pipeline::in_use::FRESH_FOR.to_std().unwrap() + std::time::Duration::from_secs(60);
            std::fs::File::options()
                .write(true)
                .open(home.join(NOTE_RECORD_FILE))
                .unwrap()
                .set_modified(std::time::SystemTime::now() - age)
                .unwrap();
        }
        let mut store = session_core::Store::open_in_memory().unwrap();
        store
            .create_notes_only("n", opened(), Some(home.to_str().unwrap()))
            .unwrap();
        store.put_segment("n", &segment()).unwrap();
        let scan = library::scan::scan(
            &[root.path().to_path_buf()],
            &library::scan::Known::default(),
            &library::scan::ScanOptions::default(),
        );
        let status: Arc<LibraryStatus> = Arc::default();
        status.apply(
            &scan,
            &library::reconcile::Coverage::Full,
            &store.sessions().unwrap(),
            &Default::default(),
            &Default::default(),
            std::time::Instant::now(),
        );
        (root, Arc::new(std::sync::Mutex::new(store)), status)
    }

    // Covers R12, R21, R23: Resume waits while the latest scan shows the
    // note in use on another computer, and stays unavailable while another
    // computer did not finish its recording.
    #[test]
    fn resume_is_refused_for_a_note_that_another_computer_uses_or_did_not_finish() {
        let now = chrono::Local::now().fixed_offset();
        let (_root, store, status) = not_saved_elsewhere(false);
        assert_eq!(
            check_resume(&store, &status, &own(), "n", now),
            Err(IN_USE_ELSEWHERE.to_string())
        );
        let (_root, store, status) = not_saved_elsewhere(true);
        assert_eq!(
            check_resume(&store, &status, &own(), "n", now),
            Err(NOT_FINISHED_ELSEWHERE.to_string())
        );
    }

    #[test]
    fn resume_is_refused_when_the_folder_of_a_saved_note_cannot_be_read() {
        let (_root, home, store) = placed_note();
        store.lock().unwrap().put_segment("n", &segment()).unwrap();
        // A file in the place of the folder: its names cannot be listed.
        std::fs::remove_dir(&home).unwrap();
        std::fs::write(&home, b"").unwrap();
        let status: Arc<LibraryStatus> = Arc::default();
        let now = chrono::Local::now().fixed_offset();
        assert_eq!(
            check_resume(&store, &status, &own(), "n", now),
            Err(FOLDER_UNAVAILABLE.to_string())
        );
    }

    // An earlier run on this computer quit while the note recorded, and
    // its marker is still fresh. The note is interrupted, and the user
    // can resume it, also when its folder cannot be read.
    #[test]
    fn a_note_that_this_computer_left_interrupted_can_be_resumed() {
        let (_root, home, store) = placed_note();
        {
            let mut store = store.lock().unwrap();
            store.put_segment("n", &segment()).unwrap();
            store
                .apply(
                    "n",
                    &Event::UserRecord {
                        has_segments: false,
                    },
                )
                .unwrap();
            store
                .apply(
                    "n",
                    &Event::CaptureLost {
                        reason: session_core::InterruptReason::Crash,
                        at: opened(),
                    },
                )
                .unwrap();
        }
        let status: Arc<LibraryStatus> = Arc::default();
        let now = chrono::Local::now().fixed_offset();
        assert!(pipeline::in_use::write_marker(&home, EARLIER_RUN, now).unwrap());
        assert_eq!(check_resume(&store, &status, &own(), "n", now), Ok(()));

        std::fs::remove_dir_all(&home).unwrap();
        std::fs::write(&home, b"").unwrap();
        assert_eq!(check_resume(&store, &status, &own(), "n", now), Ok(()));
    }
}

#[cfg(test)]
mod notes_save_tests {
    use super::*;
    use pipeline::save::{NotesRescue, NotesSave};
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// What one run of [`keep_notes_text`] did.
    struct Run {
        result: Result<NotesSaveResult, String>,
        /// The steps on the early write jobs: `true` for a start.
        steps: Vec<(bool, String)>,
        saves: usize,
        /// The id that the rescue got for the new note.
        rescued: Option<String>,
    }

    /// Run [`keep_notes_text`] for the note "n". The saves give `saves` in
    /// order, and the rescue gives `rescue`.
    fn run(saves: Vec<NotesSave>, rescue: Result<NotesRescue, String>) -> Run {
        let saves = RefCell::new(VecDeque::from(saves));
        let count = RefCell::new(0);
        let rescued = RefCell::new(None);
        let steps = RefCell::new(Vec::new());
        let result = keep_notes_text(
            "n",
            "fp".to_string(),
            || {
                *count.borrow_mut() += 1;
                Ok(saves.borrow_mut().pop_front().expect("one more save"))
            },
            |new_id| {
                *rescued.borrow_mut() = Some(new_id.to_string());
                rescue
            },
            |step| {
                let step = match step {
                    EarlyHomeStep::Cancel(id) => cancel(id),
                    EarlyHomeStep::Start(id) => start(id),
                };
                steps.borrow_mut().push(step);
            },
        );
        Run {
            result,
            steps: steps.into_inner(),
            saves: count.into_inner(),
            rescued: rescued.into_inner(),
        }
    }

    fn cancel(id: &str) -> (bool, String) {
        (false, id.to_string())
    }

    fn start(id: &str) -> (bool, String) {
        (true, id.to_string())
    }

    fn saved() -> Result<NotesSaveResult, String> {
        Ok(NotesSaveResult::Saved {
            fingerprint: "fp".into(),
        })
    }

    #[test]
    fn a_save_that_finds_its_folder_saves_with_no_rescue() {
        let run = run(
            vec![NotesSave::Saved { written: true }],
            Ok(NotesRescue::Moved),
        );
        assert_eq!(run.result, saved());
        assert_eq!(run.saves, 1);
        assert_eq!(run.rescued, None);
        assert!(run.steps.is_empty());
    }

    // R24: the folder has another name. The job of the note starts again,
    // and the second save writes the notes.
    #[test]
    fn a_moved_folder_gets_one_more_save() {
        let run = run(
            vec![NotesSave::FolderGone, NotesSave::Saved { written: true }],
            Ok(NotesRescue::Moved),
        );
        assert_eq!(run.result, saved());
        assert_eq!(run.saves, 2);
        assert_eq!(run.steps, [cancel("n"), start("n")]);
    }

    // The second save after Moved finds the folder gone again: the store
    // keeps the text, and no second rescue runs.
    #[test]
    fn a_folder_gone_again_after_the_move_is_unavailable() {
        let run = run(
            vec![NotesSave::FolderGone, NotesSave::FolderGone],
            Ok(NotesRescue::Moved),
        );
        assert_eq!(run.result, Err(FOLDER_UNAVAILABLE.to_string()));
        assert_eq!(run.saves, 2);
        assert_eq!(run.steps, [cancel("n"), start("n")]);
    }

    #[test]
    fn an_unavailable_storage_folder_keeps_the_note_and_its_job() {
        let run = run(vec![NotesSave::FolderGone], Ok(NotesRescue::Unavailable));
        assert_eq!(run.result, Err(FOLDER_UNAVAILABLE.to_string()));
        assert_eq!(run.saves, 1);
        assert_eq!(run.steps, [cancel("n"), start("n")]);
    }

    #[test]
    fn a_failed_rescue_keeps_the_note_and_its_job() {
        let run = run(vec![NotesSave::FolderGone], Err("disk".to_string()));
        assert_eq!(run.result, Err("disk".to_string()));
        assert_eq!(run.steps, [cancel("n"), start("n")]);
    }

    // R24: the folder is gone. The text is in the new note, and the job of
    // the new note starts. The job of the old note does not start again.
    #[test]
    fn a_gone_folder_makes_the_new_note_and_starts_its_job() {
        let run = run(vec![NotesSave::FolderGone], Ok(NotesRescue::NewNote));
        let new_id = run.rescued.clone().expect("the rescue ran");
        assert_ne!(new_id, "n");
        assert_eq!(
            run.result,
            Ok(NotesSaveResult::NewNote {
                session_id: new_id.clone(),
                fingerprint: "fp".into(),
            })
        );
        assert_eq!(run.saves, 1);
        assert_eq!(run.steps, [cancel("n"), start(&new_id)]);
    }

    #[test]
    fn a_conflict_gives_the_draft_with_no_rescue() {
        let draft = PathBuf::from("/notes/draft.md");
        let run = run(
            vec![NotesSave::Conflict {
                draft: draft.clone(),
            }],
            Ok(NotesRescue::Moved),
        );
        assert_eq!(
            run.result,
            Ok(NotesSaveResult::Conflict {
                draft: draft.to_string_lossy().into_owned(),
            })
        );
        assert_eq!(run.rescued, None);
        assert!(run.steps.is_empty());
    }
}
