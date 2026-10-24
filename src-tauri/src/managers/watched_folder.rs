//! The watched folder: while Longhand runs,
//! each new audio file in a folder that the user chose on this computer
//! becomes a note with no user action (R5).
//!
//! The watcher is modelled on the library watcher (`library.rs`, KTD7):
//! one thread, a `notify` debouncer, a command channel, a scan at the start
//! and a full scan each minute. An event only asks for a scan. No file is
//! imported from an event alone, because FSEvents and File Provider
//! folders drop and delay events.
//!
//! Each scan walks the folder and its subfolders (R5). It skips names that
//! start with a dot, temporary files and the extensions that Longhand does
//! not import (R19), with no message. A file is a candidate only when its
//! size and modification time have not changed for [`Timing::stable_for`]
//! (KTD8). Then the watcher reads the whole file for its identity: for an
//! online-only file, that read is the download. It reads no file while a
//! recording runs, and it reads no online-only file while the free space
//! minus the file's size is below the recording reserve. A read with no
//! progress for [`Timing::stall_after`] is a read failure: the file stays,
//! and the next scan tries it again, with no message (KTD11).
//!
//! The watcher checks the import record before it imports a file (R12,
//! R13). A file with an `imported` or `skipped` entry, or a `failed` entry
//! of this or a newer Longhand version, makes no note (R12a). A file whose
//! import is open in the import journal is not imported again (KTD6).
//! Longhand never changes a file in the watched folder (R7).
//!
//! At the answer "Only New Files" to the first-pick question (R6), the
//! files then present go into a snapshot file in the app data folder
//! ([`SNAPSHOT_FILE`]). The watcher holds them from import, reads each one
//! and writes a `skipped` entry for it, then drops it from the snapshot
//! (KTD12, R12b). Settling files and read failures stay in memory only: the
//! next scan finds them again.
//!
//! While the storage folder changes, the watcher pauses: a read in
//! progress stops, and no scan, read or import starts until the resume.
//!
//! A folder that cannot be read for [`UNAVAILABLE_SCANS`] scans in a row
//! has the status [`WatchedFolderStatus::Unavailable`]. The next scan that
//! reads it clears the status.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use chrono::Local;
use import_record::{ComputerId, Entry, Identity, Lookup, State};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::managers::library::should_skip_path;

/// The name of the snapshot file in the app data folder, next to
/// `settings.json` (KTD12).
pub const SNAPSHOT_FILE: &str = "watched-folder-snapshot.json";

/// The version of the snapshot file that this build writes.
const SNAPSHOT_VERSION: u32 = 1;

/// The number of scans in a row that cannot read the folder before its
/// status is unavailable (Home shows the notice).
pub const UNAVAILABLE_SCANS: u32 = 3;

/// How long [`WatchedFolder::stop`] waits for a scan in progress.
const STOP_WAIT: Duration = Duration::from_secs(2);

/// How long [`WatchedFolder::pause`] waits for a scan in progress. A read
/// stops within [`PROGRESS_CHECK`] of the pause.
const PAUSE_WAIT: Duration = Duration::from_secs(10);

/// The longest time between two checks of the progress of a read.
const PROGRESS_CHECK: Duration = Duration::from_secs(1);

/// The state of the watched folder, as Settings shows it and the notices
/// on Home read it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WatchedFolderStatus {
    /// No watched folder is set on this computer.
    Off,
    /// The folder is watched.
    Watching { path: String },
    /// The folder could not be read for [`UNAVAILABLE_SCANS`] scans in a
    /// row. The watcher tries again at each scan.
    Unavailable { path: String },
}

impl WatchedFolderStatus {
    /// The status from the watched folder setting `folder` alone, when no
    /// watcher runs.
    pub fn of_setting(folder: Option<String>) -> Self {
        match folder {
            Some(path) => Self::Watching { path },
            None => Self::Off,
        }
    }
}

/// The status of the watched folder changed.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type, tauri_specta::Event)]
pub struct WatchedFolderChanged {
    pub status: WatchedFolderStatus,
}

/// The times that the watcher uses.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// The debounce of the watch, as the library watcher uses.
    pub debounce: Duration,
    /// The time between two full scans.
    pub full_scan_every: Duration,
    /// How long the size and the modification time of a file must stay
    /// the same before the watcher reads it (KTD8).
    pub stable_for: Duration,
    /// A read with no progress for this time is a read failure (KTD8).
    pub stall_after: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(900),
            full_scan_every: Duration::from_secs(60),
            stable_for: Duration::from_secs(30),
            stall_after: Duration::from_secs(10 * 60),
        }
    }
}

/// Read the whole file for its SHA-256 and count the bytes read.
pub type HashFn = Arc<dyn Fn(&Path, &AtomicU64) -> io::Result<String> + Send + Sync>;
/// Import a file into a new note. Returns the note id.
pub type ImportFn = Box<dyn Fn(&Path) -> Result<String, String> + Send>;
/// The free space of the volume of a folder, or `None` when unknown.
pub type FreeSpaceFn = Box<dyn Fn(&Path) -> Option<u64> + Send>;
/// Whether a file is online only.
pub type DatalessFn = Box<dyn Fn(&Path, &std::fs::Metadata) -> bool + Send>;

/// What the watcher needs from the app.
pub struct Environment {
    /// The storage folder now, whose import record the watcher reads.
    pub storage_folder: Box<dyn Fn() -> Option<PathBuf> + Send>,
    /// The id that names this computer's entry files.
    pub computer_id: ComputerId,
    /// The Longhand version that runs: a `failed` entry of an older version
    /// gives the file another try (R12a).
    pub longhand_version: String,
    /// Import the file into a new note with the default type and no tags
    /// (R5). Returns the note id.
    pub import: ImportFn,
    /// The source files of the open imports (KTD6).
    pub open_sources: Box<dyn Fn() -> Vec<PathBuf> + Send>,
    /// Try the imports that wait for free space or the storage folder
    /// again (KTD9).
    pub retry: Box<dyn Fn() + Send>,
    /// True while a recording runs or the live worker drains: no read
    /// starts (KTD8).
    pub suspended: Box<dyn Fn() -> bool + Send>,
    /// The free space of the volume of a folder, or `None` when unknown.
    pub free_space: FreeSpaceFn,
    /// Whether a file is online only: its read is a download.
    pub dataless: DatalessFn,
    /// The read of a file for its identity.
    pub hash: HashFn,
    /// The snapshot file ([`SNAPSHOT_FILE`] in the app data folder).
    pub snapshot: PathBuf,
    /// Tell the app that the status changed.
    pub status: Box<dyn Fn(&WatchedFolderStatus) + Send>,
}

/// One audio file in the watched folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

/// List the audio files in `folder` and its subfolders that the watcher
/// imports (R5, R19): no name that starts with a dot, no temporary file,
/// no other extension, and no link. A subfolder that cannot be read is
/// left out. The error is for `folder` itself.
pub fn list(folder: &Path) -> io::Result<Vec<Listed>> {
    let mut found = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    let mut first = true;
    while let Some(dir) = folders.pop() {
        let items = match std::fs::read_dir(&dir) {
            Ok(items) => items,
            Err(e) if first => return Err(e),
            Err(e) => {
                log::warn!("watched folder: cannot read a subfolder: {e}");
                continue;
            }
        };
        first = false;
        for item in items.flatten() {
            let path = item.path();
            let Ok(relative) = path.strip_prefix(folder) else {
                continue;
            };
            if should_skip_path(relative, &path) {
                continue;
            }
            // `file_type` does not follow a link, so no link is read and
            // no loop of links is walked.
            let Ok(kind) = item.file_type() else {
                continue;
            };
            if kind.is_dir() {
                folders.push(path);
            } else if kind.is_file() && audio_import::is_accepted(&path) {
                let Ok(meta) = item.metadata() else {
                    continue;
                };
                found.push(Listed {
                    path,
                    size: meta.len(),
                    modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                });
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// The count and the total size of the audio files that the first-pick
/// question names (R6).
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type)]
pub struct WatchedFolderPreview {
    pub count: u32,
    /// The total size in bytes.
    pub bytes: u64,
}

/// The audio files in `listed` whose name and size match no entry in the
/// import record of `storage` (R6, KTD12).
pub fn preview(listed: &[Listed], storage: Option<&Path>) -> WatchedFolderPreview {
    let known = storage
        .map(import_record::names_and_sizes)
        .unwrap_or_default();
    let mut counted = WatchedFolderPreview { count: 0, bytes: 0 };
    for file in listed {
        if !known.contains(&(audio_import::file_name(&file.path), file.size)) {
            counted.count += 1;
            counted.bytes = counted.bytes.saturating_add(file.size);
        }
    }
    counted
}

/// A file that the snapshot holds from import (KTD12).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub path: PathBuf,
    pub size: u64,
    /// The modification time, in milliseconds since 1970.
    pub modified_ms: i64,
}

impl From<&Listed> for Held {
    fn from(listed: &Listed) -> Self {
        let modified_ms = listed
            .modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        Self {
            path: listed.path.clone(),
            size: listed.size,
            modified_ms,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct SnapshotFile {
    version: u32,
    folder: PathBuf,
    files: Vec<Held>,
}

/// Write the snapshot of `files` in `folder` to `file` (R6, KTD12). An
/// empty list removes the file.
pub fn write_snapshot(file: &Path, folder: &Path, files: &[Held]) -> io::Result<()> {
    if files.is_empty() {
        return remove_snapshot(file);
    }
    let bytes = serde_json::to_vec_pretty(&SnapshotFile {
        version: SNAPSHOT_VERSION,
        folder: folder.to_path_buf(),
        files: files.to_vec(),
    })?;
    let dir = file
        .parent()
        .ok_or_else(|| io::Error::other("the snapshot path has no folder"))?;
    let name = file
        .file_name()
        .ok_or_else(|| io::Error::other("the snapshot path has no file name"))?;
    std::fs::create_dir_all(dir)?;
    session_save::home_write::write_atomic(dir, &name.to_string_lossy(), &bytes)
}

/// Remove the snapshot file. A file that is not there is removed.
pub fn remove_snapshot(file: &Path) -> io::Result<()> {
    match std::fs::remove_file(file) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The files of the snapshot in `file` for `folder`. A snapshot of
/// another folder, or one that does not parse, holds nothing.
fn read_snapshot(file: &Path, folder: &Path) -> Vec<Held> {
    let Ok(bytes) = std::fs::read(file) else {
        return Vec::new();
    };
    match serde_json::from_slice::<SnapshotFile>(&bytes) {
        Ok(snapshot) if snapshot.folder == folder => snapshot.files,
        Ok(_) => Vec::new(),
        Err(e) => {
            log::warn!("watched folder: the snapshot does not parse: {e}");
            Vec::new()
        }
    }
}

/// Whether the file of `meta` is online only (`SF_DATALESS` on macOS, a
/// placeholder with no local data on Windows): its read is a download.
pub fn is_dataless(_path: &Path, meta: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        /// `SF_DATALESS` of `sys/stat.h`.
        const SF_DATALESS: u32 = 0x4000_0000;
        meta.st_flags() & SF_DATALESS != 0
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        /// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` and `FILE_ATTRIBUTE_OFFLINE`.
        const ONLINE_ONLY: u32 = 0x0040_0000 | 0x0000_1000;
        meta.file_attributes() & ONLINE_ONLY != 0
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = meta;
        false
    }
}

/// The bytes on disk of `path`: an online-only file gets more while it
/// downloads.
fn allocated(path: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path)
            .map(|m| m.blocks().saturating_mul(512))
            .unwrap_or(0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0
    }
}

/// What the watcher knows of one file in this run.
#[derive(Debug, Clone)]
struct Tracked {
    size: u64,
    modified: SystemTime,
    /// When the watcher first saw this size and modification time.
    since: Instant,
    /// The identity, when a read gave it for this size and time.
    identity: Option<Identity>,
    /// The file makes no note: the import record holds it, or this run
    /// imported it.
    known: bool,
}

impl Tracked {
    fn new(listed: &Listed, now: Instant) -> Self {
        Self {
            size: listed.size,
            modified: listed.modified,
            since: now,
            identity: None,
            known: false,
        }
    }

    fn is(&self, listed: &Listed) -> bool {
        self.size == listed.size && self.modified == listed.modified
    }
}

/// What a read of a file gave.
enum Read {
    Identity(Identity),
    /// Not now: the next scan tries again.
    Later,
    /// No read starts in this scan: a recording runs, or the app stops.
    Stop,
}

enum Command {
    Events,
    Folder {
        folder: Option<PathBuf>,
        skip: Vec<Held>,
    },
    /// The thread replies when it does no more work: the pause flag is
    /// set, and the scan in progress stopped.
    Paused(Sender<()>),
    Stop,
}

/// The state of the watcher thread.
struct Worker {
    env: Environment,
    timing: Timing,
    folder: Option<PathBuf>,
    watch: Option<Debouncer<RecommendedWatcher, RecommendedCache>>,
    /// Each watch sends its events here.
    commands: Sender<Command>,
    tracked: HashMap<PathBuf, Tracked>,
    /// The files that the snapshot holds from import (KTD12).
    held: Vec<Held>,
    /// The identities that this run imported. A copy of a file whose
    /// import is not recorded yet makes no second note.
    imported: HashSet<Identity>,
    /// The scans in a row that could not read the folder.
    failed_scans: u32,
    status: WatchedFolderStatus,
    next_full: Instant,
    last_scan: Instant,
    stopped: Arc<AtomicBool>,
    /// True while the storage folder changes: no scan, no read and no
    /// import.
    paused: Arc<AtomicBool>,
}

impl Worker {
    fn new(
        env: Environment,
        timing: Timing,
        commands: Sender<Command>,
        stopped: Arc<AtomicBool>,
        paused: Arc<AtomicBool>,
    ) -> Self {
        let now = Instant::now();
        Self {
            env,
            timing,
            folder: None,
            watch: None,
            commands,
            tracked: HashMap::new(),
            held: Vec::new(),
            imported: HashSet::new(),
            failed_scans: 0,
            status: WatchedFolderStatus::Off,
            next_full: now + timing.full_scan_every,
            last_scan: now,
            stopped,
            paused,
        }
    }

    fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Watch `folder`, or nothing. The memory of the old folder goes.
    /// `skip` is the new snapshot: the files that the user chose not to
    /// import (R6). Only this thread writes the snapshot file, so a skip
    /// pass of the old folder cannot write over the new snapshot. `None`
    /// reads the snapshot file, as at the start.
    fn set_folder(&mut self, folder: Option<PathBuf>, skip: Option<Vec<Held>>) {
        self.folder = folder;
        self.watch = None;
        self.tracked.clear();
        self.failed_scans = 0;
        self.held = match (&self.folder, skip) {
            (Some(folder), None) => read_snapshot(&self.env.snapshot, folder),
            (Some(folder), Some(skip)) => {
                if let Err(e) = write_snapshot(&self.env.snapshot, folder, &skip) {
                    log::error!("watched folder: cannot write the snapshot: {e}");
                }
                skip
            }
            (None, _) => {
                if let Err(e) = remove_snapshot(&self.env.snapshot) {
                    log::error!("watched folder: cannot remove the snapshot: {e}");
                }
                Vec::new()
            }
        };
        let status = match &self.folder {
            Some(folder) => WatchedFolderStatus::Watching {
                path: folder.to_string_lossy().into_owned(),
            },
            None => WatchedFolderStatus::Off,
        };
        self.set_status(status);
        self.sync_watch();
    }

    fn set_status(&mut self, status: WatchedFolderStatus) {
        if self.status != status {
            self.status = status;
            (self.env.status)(&self.status);
        }
    }

    /// Watch the folder when it is present and has no watch.
    fn sync_watch(&mut self) {
        let Some(folder) = self.folder.clone() else {
            return;
        };
        if self.watch.is_some() || !folder.is_dir() {
            return;
        }
        let commands = self.commands.clone();
        let started = new_debouncer(self.timing.debounce, None, move |_: DebounceEventResult| {
            let _ = commands.send(Command::Events);
        })
        .and_then(|mut debouncer| {
            debouncer.watch(&folder, RecursiveMode::Recursive)?;
            Ok(debouncer)
        });
        match started {
            Ok(debouncer) => self.watch = Some(debouncer),
            Err(e) => log::warn!("watched folder: cannot watch the folder: {e}"),
        }
    }

    /// One scan of the folder at `now`: the imports that wait try again,
    /// each stable file that is not known is read and imported, and then
    /// the skip work of the snapshot continues.
    fn scan(&mut self, now: Instant) {
        self.last_scan = now;
        if self.is_paused() {
            return;
        }
        let Some(folder) = self.folder.clone() else {
            return;
        };
        (self.env.retry)();
        let listed = match list(&folder) {
            Ok(listed) => listed,
            Err(e) => {
                self.failed_scan(&folder, &e);
                return;
            }
        };
        self.failed_scans = 0;
        self.set_status(WatchedFolderStatus::Watching {
            path: folder.to_string_lossy().into_owned(),
        });
        if self.watch.is_none() {
            self.sync_watch();
        }
        let present: HashSet<&Path> = listed.iter().map(|l| l.path.as_path()).collect();
        self.tracked
            .retain(|path, _| present.contains(path.as_path()));
        let open = (self.env.open_sources)();
        let held: HashSet<&Path> = self.held.iter().map(|h| h.path.as_path()).collect();
        let mut candidates = Vec::new();
        for file in &listed {
            let tracked = self
                .tracked
                .entry(file.path.clone())
                .or_insert_with(|| Tracked::new(file, now));
            if !tracked.is(file) {
                *tracked = Tracked::new(file, now);
            }
            if tracked.known
                || now.saturating_duration_since(tracked.since) < self.timing.stable_for
            {
                continue;
            }
            if held.contains(file.path.as_path()) || open.contains(&file.path) {
                continue;
            }
            candidates.push(file.clone());
        }
        for file in &candidates {
            if !self.check(file) {
                return;
            }
        }
        self.skip_work();
    }

    /// The scan could not read the folder. After [`UNAVAILABLE_SCANS`]
    /// such scans in a row, the status is unavailable. What the watcher
    /// knows of the files stays: a folder that went away is not a delete.
    fn failed_scan(&mut self, folder: &Path, error: &io::Error) {
        self.failed_scans = self.failed_scans.saturating_add(1);
        self.watch = None;
        log::warn!(
            "watched folder: cannot read the folder ({} of {UNAVAILABLE_SCANS}): {error}",
            self.failed_scans
        );
        if self.failed_scans >= UNAVAILABLE_SCANS {
            self.set_status(WatchedFolderStatus::Unavailable {
                path: folder.to_string_lossy().into_owned(),
            });
        }
    }

    /// Read the stable file `file` for its identity, check the import
    /// record, and import the file when the record does not hold it (R12,
    /// R12a, R13). Returns false when no more reads can start in this
    /// scan: a recording runs, or the app stops.
    fn check(&mut self, file: &Listed) -> bool {
        let known = self
            .tracked
            .get(&file.path)
            .and_then(|t| t.identity.clone());
        let identity = match known {
            Some(identity) => identity,
            None => match self.read(&file.path) {
                Read::Identity(identity) => identity,
                Read::Later => return true,
                Read::Stop => return false,
            },
        };
        // The file changed during the read: the next scans wait until it
        // is stable again.
        let now = std::fs::metadata(&file.path).ok().map(|meta| {
            (
                meta.len(),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            )
        });
        if now != Some((file.size, file.modified)) {
            self.tracked.remove(&file.path);
            return true;
        }
        if let Some(tracked) = self.tracked.get_mut(&file.path) {
            tracked.identity = Some(identity.clone());
        }
        let Some(root) = self.record_folder() else {
            return true;
        };
        if !self.imported.contains(&identity) {
            let import = match import_record::lookup(&root, &identity) {
                Lookup::New => true,
                Lookup::Failed(entry) => entry.allows_retry(&self.env.longhand_version),
                Lookup::Imported(_) | Lookup::Skipped(_) => false,
                // An entry file cannot be read now: a sync can be in
                // progress.
                Lookup::Wait => return true,
            };
            if import {
                match (self.env.import)(&file.path) {
                    Ok(note_id) => {
                        log::info!("watched folder: a new file became the note {note_id}");
                        self.imported.insert(identity);
                    }
                    Err(e) => {
                        log::error!("watched folder: cannot import a file: {e}");
                        return true;
                    }
                }
            }
        }
        if let Some(tracked) = self.tracked.get_mut(&file.path) {
            tracked.known = true;
        }
        true
    }

    /// The skip work of the snapshot (KTD12, R12b): read each held file
    /// and write a `skipped` entry when the record does not hold it. A
    /// file that is done, or gone, leaves the snapshot. The snapshot file
    /// is written once, when the pass ends or stops. After a crash, the
    /// next pass reads the files again and finds their entries.
    fn skip_work(&mut self) {
        let before = self.held.len();
        self.skip_pass();
        if self.held.len() != before {
            self.write_held();
        }
    }

    /// The pass of [`Worker::skip_work`]. It removes each file that is
    /// done, or gone, from `held`. A file whose size or modification time
    /// is not the one in the snapshot is not the file that the user chose
    /// to skip: it leaves `held` with no skipped entry, and the scan
    /// imports it when it is stable.
    fn skip_pass(&mut self) {
        let mut i = 0;
        while i < self.held.len() {
            let path = self.held[i].path.clone();
            let meta = match std::fs::metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    self.held.remove(i);
                    continue;
                }
                Err(_) => {
                    i += 1;
                    continue;
                }
            };
            if !self.is_held_file(i, &meta) {
                self.held.remove(i);
                continue;
            }
            let identity = match self.read(&path) {
                Read::Identity(identity) => identity,
                Read::Later => {
                    i += 1;
                    continue;
                }
                Read::Stop => return,
            };
            // The file can change during the read.
            match std::fs::metadata(&path) {
                Ok(after) if self.is_held_file(i, &after) => {}
                Ok(_) => {
                    self.held.remove(i);
                    continue;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    self.held.remove(i);
                    continue;
                }
                Err(_) => {
                    i += 1;
                    continue;
                }
            }
            let Some(root) = self.record_folder() else {
                return;
            };
            let done = match import_record::lookup(&root, &identity) {
                Lookup::Imported(_) | Lookup::Skipped(_) => true,
                Lookup::New | Lookup::Failed(_) => {
                    let skipped = Entry::new(
                        State::Skipped,
                        &audio_import::file_name(&path),
                        meta.len(),
                        self.env.computer_id.clone(),
                        Local::now().fixed_offset(),
                    );
                    match import_record::write(&root, &identity, &skipped) {
                        Ok(_) => true,
                        Err(e) => {
                            log::warn!("watched folder: cannot write a skipped entry: {e}");
                            false
                        }
                    }
                }
                Lookup::Wait => false,
            };
            if !done {
                i += 1;
                continue;
            }
            let listed = Listed {
                path: path.clone(),
                size: meta.len(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            };
            let mut tracked = Tracked::new(&listed, self.last_scan);
            tracked.identity = Some(identity);
            tracked.known = true;
            self.tracked.insert(path, tracked);
            self.held.remove(i);
        }
    }

    /// Whether `meta` has the size and the modification time of the held
    /// file `held[i]` in the snapshot.
    fn is_held_file(&self, i: usize, meta: &std::fs::Metadata) -> bool {
        let path = &self.held[i].path;
        let now = Held::from(&Listed {
            path: path.clone(),
            size: meta.len(),
            modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
        now == self.held[i]
    }

    /// The storage folder whose import record the watcher reads, when it is
    /// a folder now. A lookup in a folder that is not there finds no entry,
    /// and that is not "no entry": the disk can be disconnected.
    fn record_folder(&self) -> Option<PathBuf> {
        (self.env.storage_folder)().filter(|root| root.is_dir())
    }

    /// Write the snapshot file again with the held files.
    fn write_held(&self) {
        let Some(folder) = &self.folder else {
            return;
        };
        if let Err(e) = write_snapshot(&self.env.snapshot, folder, &self.held) {
            log::error!("watched folder: cannot write the snapshot: {e}");
        }
    }

    /// Read `path` for its identity (KTD8). No read starts while a
    /// recording runs. An online-only file is downloaded by the read, so
    /// its read starts only when the free space minus its size stays above
    /// the recording reserve.
    fn read(&self, path: &Path) -> Read {
        if self.stopped.load(Ordering::SeqCst) || self.is_paused() || (self.env.suspended)() {
            return Read::Stop;
        }
        let Ok(meta) = std::fs::metadata(path) else {
            return Read::Later;
        };
        if (self.env.dataless)(path, &meta) {
            let folder = path.parent().unwrap_or(path);
            let needed = meta
                .len()
                .saturating_add(recorder::storage::DISK_RESERVE_BYTES);
            if (self.env.free_space)(folder).is_some_and(|free| free < needed) {
                log::info!("watched folder: a download waits for free space");
                return Read::Later;
            }
        }
        match self.hash(path) {
            Ok(hash) => Identity::parse(&hash).map_or(Read::Later, Read::Identity),
            Err(e) => {
                log::warn!("watched folder: cannot read a file now: {e}");
                Read::Later
            }
        }
    }

    /// Hash `path` on a thread of its own. A read with no progress for
    /// [`Timing::stall_after`] ends as a read failure; its thread is left
    /// to end by itself. Progress is a byte read, or a byte that a download
    /// put on disk.
    fn hash(&self, path: &Path) -> io::Result<String> {
        let read = Arc::new(AtomicU64::new(0));
        let (sent, result) = mpsc::channel();
        let hash = Arc::clone(&self.env.hash);
        let (file, counter) = (path.to_path_buf(), Arc::clone(&read));
        std::thread::Builder::new()
            .name("watched-folder-read".into())
            .spawn(move || {
                let _ = sent.send(hash(&file, &counter));
            })?;
        let progress = || read.load(Ordering::SeqCst).saturating_add(allocated(path));
        let check = self.timing.stall_after.min(PROGRESS_CHECK);
        let mut last = progress();
        let mut since = Instant::now();
        loop {
            match result.recv_timeout(check) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::other("the read stopped"));
                }
                Err(RecvTimeoutError::Timeout) => {
                    if self.stopped.load(Ordering::SeqCst) {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "the app stops"));
                    }
                    if self.is_paused() {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "the watcher pauses",
                        ));
                    }
                    let now = progress();
                    if now != last {
                        last = now;
                        since = Instant::now();
                    } else if since.elapsed() >= self.timing.stall_after {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the read made no progress",
                        ));
                    }
                }
            }
        }
    }

    /// When the thread must wake with no event: for the next full scan, or
    /// when a settling file becomes stable.
    fn deadline(&self) -> Instant {
        let settled = self
            .tracked
            .values()
            .filter(|t| !t.known)
            .map(|t| t.since + self.timing.stable_for)
            .filter(|due| *due > self.last_scan)
            .min();
        settled.map_or(self.next_full, |due| due.min(self.next_full))
    }

    fn timeout(&mut self, now: Instant) {
        if now >= self.next_full {
            self.sync_watch();
            self.next_full = now + self.timing.full_scan_every;
        }
        self.scan(now);
    }

    fn run(mut self, commands: Receiver<Command>) {
        self.scan(Instant::now());
        loop {
            let wait = self.deadline().saturating_duration_since(Instant::now());
            match commands.recv_timeout(wait) {
                Ok(Command::Events) => self.scan(Instant::now()),
                Ok(Command::Folder { folder, skip }) => {
                    self.set_folder(folder, Some(skip));
                    self.scan(Instant::now());
                }
                Ok(Command::Paused(reply)) => {
                    let _ = reply.send(());
                }
                Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => self.timeout(Instant::now()),
            }
        }
        self.watch = None;
    }
}

/// Watches the watched folder until it is stopped or dropped.
pub struct WatchedFolder {
    commands: Sender<Command>,
    status: Arc<Mutex<WatchedFolderStatus>>,
    stopped: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    /// The thread, and the channel on which it says that it stopped.
    thread: Mutex<Option<(Receiver<()>, JoinHandle<()>)>>,
}

impl WatchedFolder {
    /// Start watching `folder`, or nothing when it is `None`. The first
    /// scan runs at once: a file that arrived while Longhand was closed is
    /// imported now (R5). With `paused`, the watcher starts paused, and the
    /// first scan runs at [`WatchedFolder::resume`].
    pub fn start(
        folder: Option<PathBuf>,
        mut env: Environment,
        timing: Timing,
        paused: bool,
    ) -> io::Result<Self> {
        let (commands, received) = mpsc::channel();
        let status = Arc::new(Mutex::new(WatchedFolderStatus::Off));
        let shared = Arc::clone(&status);
        let tell = env.status;
        env.status = Box::new(move |next| {
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = next.clone();
            tell(next);
        });
        let stopped = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(paused));
        let mut worker = Worker::new(
            env,
            timing,
            commands.clone(),
            Arc::clone(&stopped),
            Arc::clone(&paused),
        );
        worker.set_folder(folder, None);
        let (done, finished) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("watched-folder".into())
            .spawn(move || {
                worker.run(received);
                let _ = done.send(());
            })?;
        Ok(Self {
            commands,
            status,
            stopped,
            paused,
            thread: Mutex::new(Some((finished, thread))),
        })
    }

    /// Pause the watcher while the storage folder changes: a read in
    /// progress stops, and no new scan, read or import starts. Returns when
    /// the thread does no more work, but not later than [`PAUSE_WAIT`].
    /// Returns false when the thread did not reply in that time.
    pub fn pause(&self) -> bool {
        self.paused.store(true, Ordering::SeqCst);
        let (reply, replied) = mpsc::channel();
        if self.commands.send(Command::Paused(reply)).is_err() {
            return false;
        }
        let done = replied.recv_timeout(PAUSE_WAIT).is_ok();
        if !done {
            log::warn!("watched folder: the watcher did not pause in {PAUSE_WAIT:?}");
        }
        done
    }

    /// End the pause, and scan at once.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        let _ = self.commands.send(Command::Events);
    }

    /// The watched folder changed: watch `folder`, or nothing. `skip` holds
    /// the files that the user chose not to import (R6): the watcher writes
    /// them as the new snapshot. An empty `skip` removes the snapshot.
    pub fn set_folder(&self, folder: Option<PathBuf>, skip: Vec<Held>) {
        let _ = self.commands.send(Command::Folder { folder, skip });
    }

    /// The status now.
    pub fn status(&self) -> WatchedFolderStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Stop the watch and the thread. Waits for a scan in progress, but
    /// not longer than [`STOP_WAIT`], so a read that does not finish does
    /// not stop the quit.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = self.commands.send(Command::Stop);
        let taken = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some((finished, thread)) = taken {
            match finished.recv_timeout(STOP_WAIT) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = thread.join();
                }
                Err(RecvTimeoutError::Timeout) => {
                    log::warn!("watched folder: the watcher did not stop in {STOP_WAIT:?}");
                }
            }
        }
    }
}

impl Drop for WatchedFolder {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THIS: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const VERSION: &str = "1.5.0";
    /// Longer than the stability window.
    const LATER: Duration = Duration::from_secs(31);

    fn fast() -> Timing {
        Timing {
            debounce: Duration::from_millis(100),
            full_scan_every: Duration::from_secs(3600),
            stable_for: Duration::from_secs(30),
            stall_after: Duration::from_millis(50),
        }
    }

    /// What the fake app saw.
    #[derive(Default)]
    struct Seen {
        imports: Vec<PathBuf>,
        statuses: Vec<WatchedFolderStatus>,
        hashes: Vec<PathBuf>,
    }

    /// The switches of the fake app.
    struct Fake {
        seen: Arc<Mutex<Seen>>,
        suspended: Arc<AtomicBool>,
        free: Arc<AtomicU64>,
        dataless: Arc<Mutex<HashSet<PathBuf>>>,
        /// What the fake import writes: `None` is `imported`.
        fails: Arc<AtomicBool>,
        open: Arc<Mutex<Vec<PathBuf>>>,
        /// Sends each import.
        imported: Option<Sender<PathBuf>>,
    }

    struct Folders {
        _dir: tempfile::TempDir,
        watched: PathBuf,
        storage: PathBuf,
        snapshot: PathBuf,
    }

    fn folders() -> Folders {
        let dir = tempfile::tempdir().unwrap();
        let watched = dir.path().join("Recordings");
        let storage = dir.path().join("Notes");
        std::fs::create_dir_all(&watched).unwrap();
        std::fs::create_dir_all(&storage).unwrap();
        let snapshot = dir.path().join("data").join(SNAPSHOT_FILE);
        Folders {
            _dir: dir,
            watched,
            storage,
            snapshot,
        }
    }

    fn identity_of(path: &Path) -> Identity {
        Identity::parse(&audio_import::hash_file(path).unwrap()).unwrap()
    }

    fn computer(id: &str) -> ComputerId {
        ComputerId::parse(id).unwrap()
    }

    fn entry(state: State, path: &Path, id: &str) -> Entry {
        Entry::new(
            state,
            &path.file_name().unwrap().to_string_lossy(),
            std::fs::metadata(path).unwrap().len(),
            computer(id),
            Local::now().fixed_offset(),
        )
    }

    /// The state of the import record of `storage` for the file `path`.
    fn record_of(storage: &Path, path: &Path) -> Lookup {
        import_record::lookup(storage, &identity_of(path))
    }

    impl Fake {
        fn new() -> Self {
            Self {
                seen: Arc::default(),
                suspended: Arc::default(),
                free: Arc::new(AtomicU64::new(u64::MAX)),
                dataless: Arc::default(),
                fails: Arc::default(),
                open: Arc::default(),
                imported: None,
            }
        }

        fn imports(&self) -> Vec<PathBuf> {
            self.seen.lock().unwrap().imports.clone()
        }

        fn statuses(&self) -> Vec<WatchedFolderStatus> {
            self.seen.lock().unwrap().statuses.clone()
        }

        fn hashes(&self) -> Vec<PathBuf> {
            self.seen.lock().unwrap().hashes.clone()
        }

        /// An environment whose import does what the import worker does
        /// to the record: an `imported` entry, or a `failed` entry with
        /// `version`.
        fn env(&self, f: &Folders, version: &str, hash: Option<HashFn>) -> Environment {
            let storage = f.storage.clone();
            let record = f.storage.clone();
            let seen = Arc::clone(&self.seen);
            let fails = Arc::clone(&self.fails);
            let sent = self.imported.clone();
            let written_version = version.to_string();
            let open = Arc::clone(&self.open);
            let suspended = Arc::clone(&self.suspended);
            let free = Arc::clone(&self.free);
            let dataless = Arc::clone(&self.dataless);
            let statuses = Arc::clone(&self.seen);
            let hashes = Arc::clone(&self.seen);
            let hash = hash.unwrap_or_else(|| {
                Arc::new(|path: &Path, read: &AtomicU64| {
                    audio_import::hash_file_counted(path, read)
                })
            });
            Environment {
                storage_folder: Box::new(move || Some(storage.clone())),
                computer_id: computer(THIS),
                longhand_version: version.to_string(),
                import: Box::new(move |path| {
                    seen.lock().unwrap().imports.push(path.to_path_buf());
                    let written = if fails.load(Ordering::SeqCst) {
                        Entry::failed(
                            &path.file_name().unwrap().to_string_lossy(),
                            std::fs::metadata(path).unwrap().len(),
                            computer(THIS),
                            Local::now().fixed_offset(),
                            &written_version,
                            "symphonia",
                            "0.6.1",
                        )
                    } else {
                        entry(State::Imported, path, THIS)
                    };
                    import_record::write(&record, &identity_of(path), &written).unwrap();
                    if let Some(sent) = &sent {
                        let _ = sent.send(path.to_path_buf());
                    }
                    Ok("note".into())
                }),
                open_sources: Box::new(move || open.lock().unwrap().clone()),
                retry: Box::new(|| {}),
                suspended: Box::new(move || suspended.load(Ordering::SeqCst)),
                free_space: Box::new(move |_| Some(free.load(Ordering::SeqCst))),
                dataless: Box::new(move |path, _| dataless.lock().unwrap().contains(path)),
                hash: Arc::new(move |path: &Path, read: &AtomicU64| {
                    hashes.lock().unwrap().hashes.push(path.to_path_buf());
                    hash(path, read)
                }),
                snapshot: f.snapshot.clone(),
                status: Box::new(move |status| {
                    statuses.lock().unwrap().statuses.push(status.clone());
                }),
            }
        }

        /// A worker with no thread that watches `f.watched`, for the tests
        /// that call its scans.
        fn worker(&self, f: &Folders) -> Worker {
            self.worker_with(f, VERSION, None)
        }

        fn worker_with(&self, f: &Folders, version: &str, hash: Option<HashFn>) -> Worker {
            let (commands, _) = mpsc::channel();
            let mut worker = Worker::new(
                self.env(f, version, hash),
                fast(),
                commands,
                Arc::default(),
                Arc::default(),
            );
            worker.set_folder(Some(f.watched.clone()), None);
            self.seen.lock().unwrap().statuses.clear();
            worker
        }
    }

    fn write(path: &Path, bytes: &[u8]) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
        path.to_path_buf()
    }

    /// Scan at `start` and again after the stability window.
    fn settle(worker: &mut Worker, start: Instant) -> Instant {
        worker.scan(start);
        let later = start + LATER;
        worker.scan(later);
        later
    }

    // R5: a new file becomes one note after it is stable.
    #[test]
    fn a_new_m4a_becomes_one_note_after_it_is_stable() {
        let f = folders();
        let mut fake = Fake::new();
        let (sent, imported) = mpsc::channel();
        fake.imported = Some(sent);
        let timing = Timing {
            stable_for: Duration::from_millis(100),
            ..fast()
        };
        let watcher = WatchedFolder::start(
            Some(f.watched.clone()),
            fake.env(&f, VERSION, None),
            timing,
            false,
        )
        .unwrap();
        let memo = f.watched.join("Team sync.m4a");
        write(&memo, b"memo one");
        // Wait for the import, not for a time. A write each second makes a
        // new event when FSEvents is slow. A limit of 120 s stops a hang.
        let limit = Instant::now() + Duration::from_secs(120);
        let path = loop {
            match imported.recv_timeout(Duration::from_secs(1)) {
                Ok(path) => break path,
                Err(_) => {
                    assert!(Instant::now() < limit, "no import");
                    write(&memo, b"memo one");
                }
            }
        };
        assert_eq!(path, memo);
        watcher.stop();
        assert_eq!(fake.imports(), vec![memo]);
        assert!(matches!(record_of(&f.storage, &path), Lookup::Imported(_)));
    }

    // R5: the watched folder includes its subfolders.
    #[test]
    fn a_file_in_a_dated_subfolder_imports() {
        let f = folders();
        let fake = Fake::new();
        let mut worker = fake.worker(&f);
        let memo = write(&f.watched.join("2026/10/24/Memo.m4a"), b"dated");
        settle(&mut worker, Instant::now());
        assert_eq!(fake.imports(), vec![memo]);
    }

    // R8, KTD8: a file that still changes waits.
    #[test]
    fn a_file_that_grows_across_scans_waits_until_it_is_stable() {
        let f = folders();
        let fake = Fake::new();
        let mut worker = fake.worker(&f);
        let memo = write(&f.watched.join("Memo.m4a"), b"part");
        let start = Instant::now();
        worker.scan(start);
        std::fs::write(&memo, b"part two").unwrap();
        worker.scan(start + LATER);
        assert!(fake.imports().is_empty());
        assert!(
            fake.hashes().is_empty(),
            "no read before the file is stable"
        );
        worker.scan(start + LATER * 2);
        assert_eq!(fake.imports(), vec![memo]);
    }

    // R19, R20: other files make no note and no message.
    #[test]
    fn a_txt_file_and_a_dot_file_are_ignored_with_no_message() {
        let f = folders();
        let fake = Fake::new();
        let mut worker = fake.worker(&f);
        write(&f.watched.join("Agenda.txt"), b"text");
        write(&f.watched.join(".Memo.m4a"), b"hidden");
        write(&f.watched.join(".hidden/Memo.m4a"), b"hidden folder");
        write(&f.watched.join("Memo.m4a.tmp"), b"temporary");
        settle(&mut worker, Instant::now());
        assert!(fake.imports().is_empty());
        assert!(fake.hashes().is_empty());
        assert!(fake.statuses().is_empty());
    }

    // Covers AE3 (R12): the identity is the content.
    #[test]
    fn a_renamed_imported_file_makes_no_note() {
        let f = folders();
        let fake = Fake::new();
        let mut worker = fake.worker(&f);
        let memo = write(&f.watched.join("Team sync.m4a"), b"same content");
        let at = settle(&mut worker, Instant::now());
        let renamed = f.watched.join("Renamed.m4a");
        std::fs::rename(&memo, &renamed).unwrap();
        settle(&mut worker, at);
        assert_eq!(fake.imports(), vec![memo]);
        // The rename is Longhand's test step, not the watcher's: the
        // watcher changed no file (R7).
        assert_eq!(std::fs::read(&renamed).unwrap(), b"same content");
    }

    // A copy of a file whose import is not recorded yet makes no second
    // note.
    #[test]
    fn a_copy_of_a_file_that_this_run_imports_makes_no_second_note() {
        let f = folders();
        let fake = Fake::new();
        // The import is open: the import worker has not written the record
        // yet.
        let seen = Arc::clone(&fake.seen);
        let mut env = fake.env(&f, VERSION, None);
        env.import = Box::new(move |path| {
            seen.lock().unwrap().imports.push(path.to_path_buf());
            Ok("note".into())
        });
        let (commands, _) = mpsc::channel();
        let mut worker = Worker::new(env, fast(), commands, Arc::default(), Arc::default());
        worker.set_folder(Some(f.watched.clone()), None);
        write(&f.watched.join("Memo.m4a"), b"one recording");
        write(&f.watched.join("Memo copy.m4a"), b"one recording");
        settle(&mut worker, Instant::now());
        assert_eq!(fake.imports().len(), 1, "{:?}", fake.imports());
    }

    // R12: a storage folder on a disk that is not connected has no import
    // record to read. Its absence is not "no entry".
    #[test]
    fn no_file_imports_while_the_storage_folder_is_not_available() {
        let f = folders();
        let fake = Fake::new();
        // A file with no entry: only the storage check stops its import.
        let memo = write(&f.watched.join("Memo.m4a"), b"not imported");
        let unplugged = f.storage.with_file_name("Unplugged");
        std::fs::rename(&f.storage, &unplugged).unwrap();
        let mut worker = fake.worker(&f);
        let at = settle(&mut worker, Instant::now());
        worker.scan(at + LATER);
        assert!(fake.imports().is_empty());
        std::fs::rename(&unplugged, &f.storage).unwrap();
        worker.scan(at + LATER * 2);
        assert_eq!(fake.imports(), vec![memo.clone()]);
        assert!(matches!(record_of(&f.storage, &memo), Lookup::Imported(_)));
    }

    // KTD6: a file whose import is open is not imported again.
    #[test]
    fn a_file_with_an_open_import_is_not_imported_again() {
        let f = folders();
        let fake = Fake::new();
        let mut worker = fake.worker(&f);
        let memo = write(&f.watched.join("Memo.m4a"), b"open");
        fake.open.lock().unwrap().push(memo.clone());
        settle(&mut worker, Instant::now());
        assert!(fake.imports().is_empty());
    }

    // Covers AE1 (R12): a deleted note does not come back after a relaunch.
    #[test]
    fn a_file_whose_note_was_deleted_makes_no_note_after_a_relaunch() {
        let f = folders();
        let fake = Fake::new();
        let memo = write(&f.watched.join("Team sync.m4a"), b"deleted note");
        let mut worker = fake.worker(&f);
        settle(&mut worker, Instant::now());
        assert_eq!(fake.imports(), vec![memo.clone()]);
        // The user deletes the note; the import record keeps the entry.
        drop(worker);
        let mut relaunched = fake.worker(&f);
        settle(&mut relaunched, Instant::now());
        assert_eq!(fake.imports(), vec![memo]);
    }

    // Covers AE6 (R13): another computer's entry stops the import.
    #[test]
    fn a_file_with_an_imported_entry_from_another_computer_makes_no_note() {
        let f = folders();
        let fake = Fake::new();
        let memo = write(&f.watched.join("Shared.m4a"), b"from computer A");
        import_record::write(
            &f.storage,
            &identity_of(&memo),
            &entry(State::Imported, &memo, OTHER),
        )
        .unwrap();
        let mut worker = fake.worker(&f);
        settle(&mut worker, Instant::now());
        assert!(fake.imports().is_empty());
    }

    // Covers AE8 (R12a): a damaged file gets one try, so one notice.
    #[test]
    fn a_damaged_m4a_gets_one_failed_entry_and_later_scans_do_not_try_it_again() {
        let f = folders();
        let fake = Fake::new();
        fake.fails.store(true, Ordering::SeqCst);
        let damaged = write(&f.watched.join("Damaged.m4a"), b"not audio");
        let mut worker = fake.worker(&f);
        let at = settle(&mut worker, Instant::now());
        worker.scan(at + LATER);
        assert_eq!(fake.imports(), vec![damaged.clone()]);
        assert!(matches!(record_of(&f.storage, &damaged), Lookup::Failed(_)));
        // A launch of the same version tries nothing.
        let mut relaunched = fake.worker(&f);
        settle(&mut relaunched, Instant::now());
        assert_eq!(fake.imports().len(), 1);
        // A newer version tries the file again.
        let mut newer = fake.worker_with(&f, "1.6.0", None);
        settle(&mut newer, Instant::now());
        assert_eq!(fake.imports().len(), 2);
    }

    // Covers R20: a failed entry that another computer wrote gives no
    // attempt here, so no notice. The computer that tried owns the notice.
    #[test]
    fn a_failed_entry_from_another_computer_makes_no_attempt() {
        let f = folders();
        let fake = Fake::new();
        let damaged = write(&f.watched.join("Damaged.m4a"), b"not audio");
        let failed = Entry::failed(
            "Damaged.m4a",
            9,
            computer(OTHER),
            Local::now().fixed_offset(),
            VERSION,
            "decoder",
            "1",
        );
        import_record::write(&f.storage, &identity_of(&damaged), &failed).unwrap();
        let mut worker = fake.worker(&f);
        settle(&mut worker, Instant::now());
        assert!(fake.imports().is_empty());
    }

    // Covers AE5 (R8): an online-only file waits for room, downloads, then
    // imports.
    #[test]
    fn an_online_only_file_waits_for_room_then_downloads_and_imports() {
        let f = folders();
        let fake = Fake::new();
        let memo = write(&f.watched.join("iCloud.m4a"), b"online only");
        fake.dataless.lock().unwrap().insert(memo.clone());
        fake.free
            .store(recorder::storage::DISK_RESERVE_BYTES + 5, Ordering::SeqCst);
        let mut worker = fake.worker(&f);
        let at = settle(&mut worker, Instant::now());
        assert!(fake.hashes().is_empty(), "no download below the reserve");
        assert!(fake.imports().is_empty());
        fake.free.store(u64::MAX, Ordering::SeqCst);
        worker.scan(at + LATER);
        assert_eq!(fake.hashes(), vec![memo.clone()]);
        assert_eq!(fake.imports(), vec![memo]);
    }

    // KTD8, KTD11: a stalled download is a read failure with no message,
    // and the next scan tries the file again.
    #[test]
    fn a_download_with_no_progress_ends_as_a_read_failure_and_the_next_scan_tries_again() {
        let f = folders();
        let fake = Fake::new();
        let memo = write(&f.watched.join("Stalled.m4a"), b"stalled");
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let first = AtomicBool::new(true);
        let hash: HashFn = Arc::new(move |path: &Path, read: &AtomicU64| {
            if first.swap(false, Ordering::SeqCst) {
                // No byte arrives until the test ends.
                let _ = gate.lock().unwrap().recv();
                return Err(io::Error::other("released"));
            }
            audio_import::hash_file_counted(path, read)
        });
        let mut worker = fake.worker_with(&f, VERSION, Some(hash));
        let at = settle(&mut worker, Instant::now());
        assert!(fake.imports().is_empty());
        assert!(fake.statuses().is_empty(), "no message");
        worker.scan(at + LATER);
        assert_eq!(fake.imports(), vec![memo]);
        let _ = release.send(());
    }

    // KTD8, KTD9: the skip work downloads nothing during a recording or
    // below the reserve, and continues after.
    #[test]
    fn a_skip_work_download_waits_for_a_recording_and_for_room() {
        let f = folders();
        let fake = Fake::new();
        let old = write(&f.watched.join("Old.m4a"), b"old recording");
        write_snapshot(
            &f.snapshot,
            &f.watched,
            &list(&f.watched)
                .unwrap()
                .iter()
                .map(Held::from)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        fake.dataless.lock().unwrap().insert(old.clone());
        fake.suspended.store(true, Ordering::SeqCst);
        let mut worker = fake.worker(&f);
        let at = settle(&mut worker, Instant::now());
        assert!(fake.hashes().is_empty(), "no download during a recording");
        fake.suspended.store(false, Ordering::SeqCst);
        fake.free.store(1, Ordering::SeqCst);
        worker.scan(at + LATER);
        assert!(fake.hashes().is_empty(), "no download below the reserve");
        fake.free.store(u64::MAX, Ordering::SeqCst);
        worker.scan(at + LATER * 2);
        assert_eq!(fake.hashes(), vec![old.clone()]);
        assert!(matches!(record_of(&f.storage, &old), Lookup::Skipped(_)));
        assert!(fake.imports().is_empty());
    }

    // Covers AE4 (R6, R12b).
    #[test]
    fn only_new_files_skips_the_files_present_and_imports_a_later_file() {
        let f = folders();
        let fake = Fake::new();
        let old: Vec<PathBuf> = (0..40)
            .map(|i| {
                write(
                    &f.watched.join(format!("Old {i}.m4a")),
                    format!("old {i}").as_bytes(),
                )
            })
            .collect();
        let files: Vec<Held> = list(&f.watched).unwrap().iter().map(Held::from).collect();
        assert_eq!(files.len(), 40);
        // The answer "Only New Files", then a quit before the skip work:
        // the snapshot still holds the files at the next launch.
        fake.suspended.store(true, Ordering::SeqCst);
        let mut first = fake.worker(&f);
        first.set_folder(Some(f.watched.clone()), Some(files));
        settle(&mut first, Instant::now());
        assert!(f.snapshot.exists());
        drop(first);
        fake.suspended.store(false, Ordering::SeqCst);
        let mut worker = fake.worker(&f);
        let at = settle(&mut worker, Instant::now());
        assert!(fake.imports().is_empty());
        for path in &old {
            assert!(
                matches!(record_of(&f.storage, path), Lookup::Skipped(_)),
                "{path:?}"
            );
        }
        assert!(!f.snapshot.exists(), "the snapshot is done");
        let new = write(&f.watched.join("New.m4a"), b"new recording");
        worker.scan(at);
        worker.scan(at + LATER);
        assert_eq!(fake.imports(), vec![new]);
    }

    /// Write the snapshot of the files in the watched folder now.
    fn hold_all(f: &Folders) {
        let held: Vec<Held> = list(&f.watched).unwrap().iter().map(Held::from).collect();
        write_snapshot(&f.snapshot, &f.watched, &held).unwrap();
    }

    // R6, R12b: a file that replaced a held file before the skip work is
    // not the file that the user chose to skip. It gets no skipped entry,
    // and it imports.
    #[test]
    fn a_file_that_replaces_a_held_file_before_the_skip_work_imports() {
        let f = folders();
        let fake = Fake::new();
        let memo = write(&f.watched.join("Memo.m4a"), b"old recording");
        let old = identity_of(&memo);
        hold_all(&f);
        write(&memo, b"a new recording with the same name");
        let mut worker = fake.worker(&f);
        let at = settle(&mut worker, Instant::now());
        worker.scan(at + LATER);
        assert_eq!(import_record::lookup(&f.storage, &old), Lookup::New);
        assert_eq!(fake.imports(), vec![memo.clone()]);
        assert!(matches!(record_of(&f.storage, &memo), Lookup::Imported(_)));
        assert!(!f.snapshot.exists());
    }

    // R6, R12b: a file that replaces a held file during the read of the
    // skip work gets no skipped entry, and it imports.
    #[test]
    fn a_file_that_replaces_a_held_file_during_the_skip_read_imports() {
        let f = folders();
        let fake = Fake::new();
        let memo = write(&f.watched.join("Memo.m4a"), b"old recording");
        let old = identity_of(&memo);
        hold_all(&f);
        let first = AtomicBool::new(true);
        let hash: HashFn = Arc::new(move |path: &Path, read: &AtomicU64| {
            let hash = audio_import::hash_file_counted(path, read);
            if first.swap(false, Ordering::SeqCst) {
                // The new file arrives after the read of the held file.
                std::fs::write(path, b"a new recording with the same name").unwrap();
            }
            hash
        });
        let mut worker = fake.worker_with(&f, VERSION, Some(hash));
        let at = settle(&mut worker, Instant::now());
        worker.scan(at + LATER);
        assert_eq!(import_record::lookup(&f.storage, &old), Lookup::New);
        assert_eq!(fake.imports(), vec![memo.clone()]);
        assert!(matches!(record_of(&f.storage, &memo), Lookup::Imported(_)));
        assert!(!f.snapshot.exists());
    }

    // R10: turning off stops new imports.
    #[test]
    fn turning_the_watched_folder_off_stops_imports() {
        let f = folders();
        let fake = Fake::new();
        let mut worker = fake.worker(&f);
        let old = write(&f.watched.join("Old.m4a"), b"old");
        let held = list(&f.watched).unwrap().iter().map(Held::from).collect();
        worker.set_folder(Some(f.watched.clone()), Some(held));
        assert!(f.snapshot.exists());
        worker.set_folder(None, None);
        assert!(!f.snapshot.exists(), "off drops the snapshot");
        write(&f.watched.join("Memo.m4a"), b"after off");
        settle(&mut worker, Instant::now());
        assert!(fake.imports().is_empty());
        assert!(old.exists());
        assert_eq!(fake.statuses(), vec![WatchedFolderStatus::Off]);
    }

    // The folder notice on Home shows after 3 scans that cannot read the
    // folder, and clears when it is back. Nothing else happens to a folder
    // that went away.
    #[test]
    fn a_removed_folder_is_unavailable_after_three_scans_and_clears_when_it_returns() {
        let f = folders();
        let fake = Fake::new();
        let mut worker = fake.worker(&f);
        let parked = f.watched.with_file_name("Parked");
        std::fs::rename(&f.watched, &parked).unwrap();
        let start = Instant::now();
        worker.scan(start);
        worker.scan(start + LATER);
        assert!(fake.statuses().is_empty());
        worker.scan(start + LATER * 2);
        let path = f.watched.to_string_lossy().into_owned();
        assert_eq!(
            fake.statuses(),
            vec![WatchedFolderStatus::Unavailable { path: path.clone() }]
        );
        std::fs::rename(&parked, &f.watched).unwrap();
        worker.scan(start + LATER * 3);
        assert_eq!(
            fake.statuses(),
            vec![
                WatchedFolderStatus::Unavailable { path: path.clone() },
                WatchedFolderStatus::Watching { path }
            ]
        );
        assert!(fake.imports().is_empty());
    }

    // R6, KTD12: the question counts by name and size, before any read.
    #[test]
    fn the_preview_counts_the_files_whose_name_and_size_match_no_entry() {
        let f = folders();
        let known = write(&f.watched.join("Known.m4a"), b"known");
        write(&f.watched.join("New.m4a"), b"new file");
        write(&f.watched.join("Sub/Also new.wav"), b"also new");
        write(&f.watched.join("Agenda.txt"), b"text");
        import_record::write(
            &f.storage,
            &identity_of(&known),
            &entry(State::Imported, &known, OTHER),
        )
        .unwrap();
        let listed = list(&f.watched).unwrap();
        assert_eq!(listed.len(), 3);
        let counted = preview(&listed, Some(&f.storage));
        assert_eq!(counted.count, 2);
        assert_eq!(counted.bytes, 16);
        assert_eq!(preview(&listed, None).count, 3);
    }

    #[test]
    fn a_folder_that_cannot_be_listed_gives_an_error() {
        let f = folders();
        assert!(list(&f.watched.join("gone")).is_err());
    }

    // While the storage folder changes, the watcher reads and imports
    // nothing. After the pause, the file imports.
    #[test]
    fn a_paused_watcher_reads_and_imports_nothing_until_it_resumes() {
        let f = folders();
        let fake = Fake::new();
        let mut worker = fake.worker(&f);
        let memo = write(&f.watched.join("Memo.m4a"), b"memo");
        worker.paused.store(true, Ordering::SeqCst);
        let at = settle(&mut worker, Instant::now());
        assert!(fake.hashes().is_empty());
        assert!(fake.imports().is_empty());
        worker.paused.store(false, Ordering::SeqCst);
        settle(&mut worker, at);
        assert_eq!(fake.imports(), vec![memo]);
    }

    // The pause stops a read in progress, and returns only when the
    // watcher thread does no more work. A resume imports the file.
    #[test]
    fn a_pause_stops_a_read_in_progress_and_a_resume_imports_the_file() {
        let f = folders();
        let mut fake = Fake::new();
        let (sent, imported) = mpsc::channel();
        fake.imported = Some(sent);
        let memo = write(&f.watched.join("Memo.m4a"), b"memo");
        let (started, reading) = mpsc::channel::<()>();
        let (release, gate) = mpsc::channel::<()>();
        let (started, gate) = (Mutex::new(started), Mutex::new(gate));
        let first = AtomicBool::new(true);
        let hash: HashFn = Arc::new(move |path: &Path, read: &AtomicU64| {
            if first.swap(false, Ordering::SeqCst) {
                let _ = started.lock().unwrap().send(());
                // No byte arrives until the test releases the read.
                let _ = gate.lock().unwrap().recv();
                return Err(io::Error::other("released"));
            }
            audio_import::hash_file_counted(path, read)
        });
        let timing = Timing {
            stable_for: Duration::ZERO,
            stall_after: Duration::from_secs(3600),
            ..fast()
        };
        let watcher = WatchedFolder::start(
            Some(f.watched.clone()),
            fake.env(&f, VERSION, Some(hash)),
            timing,
            false,
        )
        .unwrap();
        reading
            .recv_timeout(Duration::from_secs(120))
            .expect("the read starts");
        // The read is still blocked: only the pause can stop it.
        assert!(watcher.pause(), "the thread replies when it stops the read");
        assert!(fake.imports().is_empty());
        let _ = release.send(());
        watcher.resume();
        let path = imported
            .recv_timeout(Duration::from_secs(120))
            .expect("the file imports after the resume");
        assert_eq!(path, memo);
        watcher.stop();
    }
}
