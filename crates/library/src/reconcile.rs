//! Reconcile (KTD6): apply a library scan to the store.
//!
//! - A note id with no row is adopted (KTD1).
//! - A known note is refreshed from its folder (KTD3), but only when its row
//!   is Saved and no stage has claimed it.
//! - A note in a new folder moves the row's home path there, whatever the
//!   row's state or claim, so the home path follows a rename by id (R8). A
//!   row with a rename intent on this computer keeps its home path: this
//!   computer moves that folder itself.
//! - A row whose folder is gone from an available Destination gets a
//!   pending delete. After [`DELETE_WAIT`] with no folder for its id, the
//!   row is forgotten, if it is Saved, unclaimed and has no rename intent
//!   (R10). Forget changes no folder.
//! - Rows in an unavailable Destination stay as they are (KD7, R16).
//! - A scan of some Destinations, or of some note folders (the watcher's
//!   targeted scan, KTD7), changes no pending delete of a row outside them.
//!
//! The scan runs without the store. Before reconcile writes from a folder,
//! it reads that folder again, so a change that this computer made after
//! the scan is not undone. [`Reconciler::run`] does that read without the
//! store lock: under the lock it records each row that it reads for, and
//! after the read it writes to a row only when the row did not change in
//! the meantime. A row that changed waits for the next scan.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use session_core::jobs::busy;
use session_core::{Adoption, Segment, SessionRecord, Store, StoreError};

use crate::refresh::{self, FolderContents, RefreshError};
use crate::scan::{self, Known, LibraryScan, NoteFolder, READ_TIMEOUT, ScanOptions};
use crate::status::LibraryStatus;

/// How long a row stays after its folder is gone from an available
/// Destination (R10). A sync can deliver a rename as a removed folder and
/// then a new one (AE4).
pub const DELETE_WAIT: Duration = Duration::from_secs(30);

/// The note folders that a scan read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    /// Every note folder in each Destination of the scan.
    Full,
    /// Only these note folders: a targeted scan. A row whose home folder is
    /// not one of them gets no pending delete from the scan.
    Folders(HashSet<PathBuf>),
}

impl Coverage {
    pub(crate) fn covers(&self, home: &Path) -> bool {
        match self {
            Coverage::Full => true,
            Coverage::Folders(folders) => folders.contains(home),
        }
    }
}

/// The folders that one reconcile reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Every note folder in each of these Destinations.
    Full(Vec<PathBuf>),
    /// Only these note folders of one Destination (KTD7). When the
    /// Destination has no marker, the scan reads all of it.
    Folders {
        root: PathBuf,
        folders: HashSet<PathBuf>,
    },
}

/// The rows that a reconcile changed, for the `LibraryChanged` event
/// (KTD10). Each list is sorted and has no repeated id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Adopted rows, refreshed rows and rows with a new home path. After
    /// [`Reconciler::run`], also the notes whose [`LibraryStatus`] changed.
    pub changed: Vec<String>,
    /// Forgotten rows.
    pub removed: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }
}

/// Applies scans to the store, and keeps the pending deletes between two
/// scans. The pending deletes stay in memory only (KTD8).
#[derive(Debug, Clone)]
pub struct Reconciler {
    /// When each pending delete started.
    pending: HashMap<String, Instant>,
    read_timeout: Duration,
    /// What the scans of [`Reconciler::run`] found (KTD10).
    status: Arc<LibraryStatus>,
}

impl Default for Reconciler {
    fn default() -> Self {
        Self {
            pending: HashMap::new(),
            read_timeout: READ_TIMEOUT,
            status: Arc::default(),
        }
    }
}

impl Reconciler {
    /// A reconciler whose folder reads use the time limit of `options`.
    pub fn new(options: &ScanOptions) -> Self {
        Self {
            pending: HashMap::new(),
            read_timeout: options.read_timeout,
            status: Arc::default(),
        }
    }

    /// Write the status of each scan into `status`, which the caller
    /// shares with the commands that read it.
    pub fn with_status(mut self, status: Arc<LibraryStatus>) -> Self {
        self.status = status;
        self
    }

    pub fn status(&self) -> &Arc<LibraryStatus> {
        &self.status
    }

    /// Whether the row `id` has a pending delete.
    pub fn is_pending(&self, id: &str) -> bool {
        self.pending.contains_key(id)
    }

    /// The earliest time at which a pending delete can complete. A
    /// reconcile at or after this time, with a new scan, forgets that row.
    pub fn next_due(&self) -> Option<Instant> {
        self.pending
            .values()
            .min()
            .map(|since| *since + DELETE_WAIT)
    }

    /// Scan `target` without the store lock, then apply the scan to the
    /// store and to the status under the lock. The folders of the rows that
    /// the scan changes are read again without the lock. `claimed` is as
    /// for [`Reconciler::reconcile`].
    pub fn run(
        &mut self,
        store: &Mutex<Store>,
        target: &Target,
        claimed: &dyn Fn(&str) -> bool,
    ) -> Result<Changes, StoreError> {
        let lock = || store.lock().unwrap_or_else(|e| e.into_inner());
        let known = Known::from_records(&lock().sessions()?);
        let options = ScanOptions {
            read_timeout: self.read_timeout,
        };
        let (result, coverage) = match target {
            Target::Full(roots) => (scan::scan(roots, &known, &options), Coverage::Full),
            Target::Folders { root, folders } => {
                match scan::scan_folders(root, folders, &known, &options) {
                    Some((result, read)) => (result, Coverage::Folders(read)),
                    None => (
                        scan::scan(std::slice::from_ref(root), &known, &options),
                        Coverage::Full,
                    ),
                }
            }
        };
        let stale = self.stale(&lock(), &result, claimed)?;
        // A cloud file can take seconds to read: the lock is free meanwhile.
        let rereads = self.read_again(stale);
        let mut store = lock();
        let mut changes = self.apply(
            &mut store,
            &result,
            &coverage,
            Instant::now(),
            claimed,
            &rereads,
        )?;
        let noted = self.status.apply(&result, &coverage, &store.sessions()?);
        changes
            .changed
            .extend(noted.into_iter().filter(|id| !changes.removed.contains(id)));
        changes.changed.sort();
        changes.changed.dedup();
        Ok(changes)
    }

    /// Apply `scan` to `store`. `now` is the time of the scan. `claimed`
    /// tells whether a stage holds a claim on a session (a summary
    /// regeneration or a delete). This reads folders again with `store` in
    /// the caller's hands: [`Reconciler::run`] does those reads without the
    /// store lock.
    pub fn reconcile(
        &mut self,
        store: &mut Store,
        scan: &LibraryScan,
        coverage: &Coverage,
        now: Instant,
        claimed: &dyn Fn(&str) -> bool,
    ) -> Result<Changes, StoreError> {
        let stale = self.stale(store, scan, claimed)?;
        let rereads = self.read_again(stale);
        self.apply(store, scan, coverage, now, claimed, &rereads)
    }

    /// The known rows that `scan` changes: a new home path or a refresh.
    /// Each comes with its row as it is now, so that [`Reconciler::apply`]
    /// can see a change that this computer makes after this call.
    fn stale(
        &self,
        store: &Store,
        scan: &LibraryScan,
        claimed: &dyn Fn(&str) -> bool,
    ) -> Result<Vec<Stale>, StoreError> {
        let intents: HashSet<String> = store
            .rename_intents()?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let mut stale = Vec::new();
        for note in &scan.notes {
            let id = note.doc.session_id.as_str();
            if intents.contains(id) {
                continue;
            }
            let Some(record) = store.session(id)? else {
                continue;
            };
            let claimed = claimed(id);
            let moved = record
                .home_path
                .as_deref()
                .is_some_and(|home| Path::new(home) != note.path);
            let refreshable = !busy(&record, claimed);
            let changes = moved
                || refreshable && refresh::differs(store, id, &FolderContents::from_scan(note))?;
            if !changes {
                continue;
            }
            if let Some(before) = RowState::of(store, id, claimed)? {
                stale.push(Stale {
                    id: id.to_string(),
                    folder: note.path.clone(),
                    before,
                });
            }
        }
        Ok(stale)
    }

    /// Read the folder of each stale row again. The caller does not hold
    /// the store lock.
    fn read_again(&self, stale: Vec<Stale>) -> HashMap<String, Reread> {
        stale
            .into_iter()
            .map(|stale| {
                let contents = FolderContents::read(&stale.folder, self.read_timeout);
                (
                    stale.id,
                    Reread {
                        folder: stale.folder,
                        before: stale.before,
                        contents,
                    },
                )
            })
            .collect()
    }

    /// Apply `scan` to `store`, with the folders that [`Reconciler::read_again`] read.
    fn apply(
        &mut self,
        store: &mut Store,
        scan: &LibraryScan,
        coverage: &Coverage,
        now: Instant,
        claimed: &dyn Fn(&str) -> bool,
        rereads: &HashMap<String, Reread>,
    ) -> Result<Changes, StoreError> {
        let records: HashMap<String, SessionRecord> = store
            .sessions()?
            .into_iter()
            .map(|record| (record.id.clone(), record))
            .collect();
        let intents: HashSet<String> = store
            .rename_intents()?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        self.pending.retain(|id, _| records.contains_key(id));

        let mut changed = Vec::new();
        for note in &scan.notes {
            let id = &note.doc.session_id;
            self.pending.remove(id);
            let updated = match records.get(id) {
                None => adopt(store, note)?,
                Some(_) if intents.contains(id) => false,
                Some(record) => follow(store, record, note, claimed, rereads.get(id))?,
            };
            if updated {
                changed.push(id.clone());
            }
        }

        // A folder that exists but gives no note is not a delete (KTD5).
        let present: HashSet<&str> = scan
            .notes
            .iter()
            .map(|note| note.doc.session_id.as_str())
            .chain(
                scan.unreadable
                    .iter()
                    .chain(&scan.invalid)
                    .filter_map(|problem| problem.known_id.as_deref()),
            )
            .collect();
        let mut removed = Vec::new();
        for record in records.values() {
            let Some(home) = record.home_path.as_deref().map(Path::new) else {
                continue;
            };
            if present.contains(record.id.as_str()) || !coverage.covers(home) {
                continue;
            }
            // A scan of some Destinations says nothing about the others.
            let root = home.parent();
            if !scan.roots.iter().any(|r| Some(r.root.as_path()) == root) {
                continue;
            }
            if !root.is_some_and(|root| scan.is_available(root)) {
                self.pending.remove(&record.id);
                continue;
            }
            let since = *self.pending.entry(record.id.clone()).or_insert(now);
            let due = now.saturating_duration_since(since) >= DELETE_WAIT;
            if due
                && !busy(record, claimed(&record.id))
                && !intents.contains(&record.id)
                && store.forget(&record.id)?
            {
                log::info!(
                    "library: the folder of {} is gone, so its row is removed",
                    record.id
                );
                self.pending.remove(&record.id);
                removed.push(record.id.clone());
            }
        }

        changed.sort();
        changed.dedup();
        removed.sort();
        Ok(Changes { changed, removed })
    }
}

/// A known row that a scan changes, before its folder is read again.
struct Stale {
    id: String,
    folder: PathBuf,
    before: RowState,
}

/// The second read of a stale row's folder.
struct Reread {
    folder: PathBuf,
    /// The row when the reconcile decided to read the folder again.
    before: RowState,
    contents: Result<FolderContents, RefreshError>,
}

/// The parts of a row that a reconcile compares with the folder, and the
/// facts that tell whether it follows the folder. When one of them changes
/// between the decision and the write, this computer changed the row, and
/// the folder that the reconcile read can be older than that change.
#[derive(Debug, PartialEq, Eq)]
struct RowState {
    record: SessionRecord,
    notes: String,
    segments: Vec<Segment>,
    stereo_from: Option<u32>,
    transcript_fingerprint: Option<String>,
    rename_intent: bool,
    claimed: bool,
}

impl RowState {
    fn of(store: &Store, id: &str, claimed: bool) -> Result<Option<Self>, StoreError> {
        let Some(record) = store.session(id)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            record,
            notes: store.notes(id)?,
            segments: store.segments(id)?,
            stereo_from: store.stereo_from(id)?,
            transcript_fingerprint: store.transcript_fingerprint(id)?,
            rename_intent: store.rename_intent(id)?.is_some(),
            claimed,
        }))
    }
}

/// Follow the note's folder for a known row: a new home path, then the
/// refresh. `reread` is the second read of the folder, when the row was
/// stale before that read. Returns whether the row changed.
fn follow(
    store: &mut Store,
    record: &SessionRecord,
    note: &NoteFolder,
    claimed: &dyn Fn(&str) -> bool,
    reread: Option<&Reread>,
) -> Result<bool, StoreError> {
    let id = record.id.as_str();
    // A row that was not stale before the read, and is now, changed on
    // this computer after the scan: the next scan follows its folder.
    let Some(reread) = reread.filter(|reread| reread.folder == note.path) else {
        return Ok(false);
    };
    let claimed = claimed(id);
    if RowState::of(store, id, claimed)?.as_ref() != Some(&reread.before) {
        log::info!(
            "library: {id} changed during the reconcile, so the next scan follows its folder"
        );
        return Ok(false);
    }
    let contents = match &reread.contents {
        Ok(contents) if contents.doc.session_id == id => contents,
        Ok(_) => return Ok(false),
        Err(e) => {
            log::warn!("library: cannot refresh {id}: {e}");
            return Ok(false);
        }
    };
    let moved = record
        .home_path
        .as_deref()
        .is_some_and(|home| Path::new(home) != note.path);
    if moved {
        log::info!("library: the folder of {id} is now {}", note.path.display());
        store.set_home_path(id, &note.path.to_string_lossy())?;
    }
    if busy(record, claimed) {
        return Ok(moved);
    }
    match refresh::apply(store, id, contents) {
        Ok(refreshed) => Ok(moved || refreshed.changed),
        Err(RefreshError::Store(e)) => Err(e),
        Err(e) => {
            log::warn!("library: cannot refresh {id}: {e}");
            Ok(moved)
        }
    }
}

/// Adopt a note that has no row (KTD1). A note whose `notes.md` has not
/// arrived starts with empty notes, and a later refresh reads them.
fn adopt(store: &mut Store, note: &NoteFolder) -> Result<bool, StoreError> {
    let adoption = Adoption::from_doc(
        &note.doc,
        note.notes.clone().unwrap_or_default(),
        note.path.to_string_lossy().into_owned(),
        note.transcript_fingerprint.clone(),
    );
    let adopted = store.adopt(&adoption)?;
    if adopted {
        log::info!(
            "library: adopted {} from {}",
            adoption.id,
            note.path.display()
        );
    }
    Ok(adopted)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::DateTime;
    use session_core::folder::SessionDoc;

    use super::*;

    fn never_claimed(_: &str) -> bool {
        false
    }

    /// A Destination with one note folder of the note `A1`, and a store
    /// that adopted it.
    fn adopted() -> (tempfile::TempDir, PathBuf, Mutex<Store>, Reconciler) {
        let root = tempfile::tempdir().unwrap();
        crate::scan::write_marker(root.path());
        let start = DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap();
        let segment = Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: start,
            wall_end: start + chrono::TimeDelta::minutes(1),
            audio_file: "audio/segment-01.opus".into(),
            channels: 1,
        };
        let folder = root.path().join("2026-09-25 0900 Planning");
        fs::create_dir(&folder).unwrap();
        let doc = SessionDoc::new("A1", None, &[segment]).unwrap();
        fs::write(folder.join("session.md"), doc.render()).unwrap();
        fs::write(folder.join("notes.md"), "first\n").unwrap();
        let store = Mutex::new(Store::open_in_memory().unwrap());
        let mut reconciler = Reconciler::default();
        let target = Target::Full(vec![root.path().to_path_buf()]);
        let changes = reconciler.run(&store, &target, &never_claimed).unwrap();
        assert_eq!(changes.changed, vec!["A1".to_string()]);
        (root, folder, store, reconciler)
    }

    fn scan_of(root: &Path, store: &Mutex<Store>) -> LibraryScan {
        let known = Known::from_records(&store.lock().unwrap().sessions().unwrap());
        scan::scan(&[root.to_path_buf()], &known, &ScanOptions::default())
    }

    /// Scan `root` and read the stale folders again, as [`Reconciler::run`]
    /// does before it takes the lock the second time.
    fn scan_and_read(
        root: &Path,
        store: &Mutex<Store>,
        reconciler: &Reconciler,
    ) -> (LibraryScan, HashMap<String, Reread>) {
        let result = scan_of(root, store);
        let stale = reconciler
            .stale(&store.lock().unwrap(), &result, &never_claimed)
            .unwrap();
        assert_eq!(stale.len(), 1);
        (result, reconciler.read_again(stale))
    }

    fn apply(
        store: &mut Store,
        reconciler: &mut Reconciler,
        result: &LibraryScan,
        rereads: &HashMap<String, Reread>,
    ) -> Changes {
        reconciler
            .apply(
                store,
                result,
                &Coverage::Full,
                Instant::now(),
                &never_claimed,
                rereads,
            )
            .unwrap()
    }

    #[test]
    fn a_notes_save_after_the_second_read_is_not_undone() {
        let (root, folder, store, mut reconciler) = adopted();
        // Another computer changes the notes, and this computer scans.
        fs::write(folder.join("notes.md"), "theirs\n").unwrap();
        let (result, rereads) = scan_and_read(root.path(), &store, &reconciler);

        // Before the write, this computer saves its own notes under the
        // lock, as `save_notes` does.
        {
            let s = store.lock().unwrap();
            s.set_notes("A1", "mine\n").unwrap();
            fs::write(folder.join("notes.md"), "mine\n").unwrap();
        }

        let mut s = store.lock().unwrap();
        let changes = apply(&mut s, &mut reconciler, &result, &rereads);
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(s.notes("A1").unwrap(), "mine\n");
        drop(s);

        // The next scan reads the folder with this computer's notes.
        let target = Target::Full(vec![root.path().to_path_buf()]);
        reconciler.run(&store, &target, &never_claimed).unwrap();
        assert_eq!(store.lock().unwrap().notes("A1").unwrap(), "mine\n");
    }

    #[test]
    fn a_rename_after_the_second_read_keeps_its_home_path() {
        let (root, folder, store, mut reconciler) = adopted();
        // A sync moves the folder, and this computer scans.
        let synced = root.path().join("2026-09-25 0900 Budget");
        fs::rename(&folder, &synced).unwrap();
        let (result, rereads) = scan_and_read(root.path(), &store, &reconciler);

        // Before the write, this computer renames the folder itself.
        let renamed = root.path().join("2026-09-25 0900 Review");
        {
            let s = store.lock().unwrap();
            fs::rename(&synced, &renamed).unwrap();
            s.set_home_path("A1", &renamed.to_string_lossy()).unwrap();
        }

        let mut s = store.lock().unwrap();
        let changes = apply(&mut s, &mut reconciler, &result, &rereads);
        assert!(changes.changed.is_empty(), "{changes:?}");
        let home = s.session("A1").unwrap().unwrap().home_path.unwrap();
        assert_eq!(PathBuf::from(home), renamed);
    }

    #[test]
    fn a_note_that_changes_only_in_its_folder_is_refreshed_by_a_run() {
        let (root, folder, store, mut reconciler) = adopted();
        fs::write(folder.join("notes.md"), "theirs\n").unwrap();
        let target = Target::Full(vec![root.path().to_path_buf()]);
        let changes = reconciler.run(&store, &target, &never_claimed).unwrap();
        assert_eq!(changes.changed, vec!["A1".to_string()]);
        assert_eq!(store.lock().unwrap().notes("A1").unwrap(), "theirs\n");
    }
}
