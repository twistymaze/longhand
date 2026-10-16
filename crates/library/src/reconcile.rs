//! Reconcile (KTD6): apply a library scan to the store.
//!
//! - A note id with no row is adopted (KTD1).
//! - A known note is refreshed from its folder (KTD3), but only when its row
//!   is Saved and no stage has claimed it. Any other row with a home folder
//!   takes only its notes from the folder (R22, KTD6 of the notes without a
//!   recording plan).
//! - A note in a new folder moves the row's home path there, whatever the
//!   row's state or claim, so the home path follows a rename by id (R8). A
//!   row with a rename intent on this computer, or a note that the Move has
//!   not finished (KTD10), keeps its row as it is: this computer moves that
//!   folder itself.
//! - A row whose folder is gone from an available Destination gets a
//!   pending delete. After [`DELETE_WAIT`] with no folder for its id, a
//!   full scan forgets the row, if it is Saved, unclaimed and has no rename
//!   intent (R10). A targeted scan never forgets a row (KTD12 of the note
//!   types plan). Forget changes no folder.
//! - The note type of a row is the type folder that holds its home folder
//!   (KTD1 of the note types plan). Each scan sets it from the path, also
//!   for a busy row, and writes no file.
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

use session_core::folder::SessionDoc;
use session_core::jobs::busy;
use session_core::note_folder::Layout;
use session_core::{Adoption, Segment, SessionRecord, Store, StoreError, Title};

use crate::refresh::{self, FolderContents, RefreshError, Scope};
use crate::scan::{self, DocumentState, Known, LibraryScan, NoteFolder, ScanOptions};
use crate::status::LibraryStatus;

/// How long a row stays after its folder is gone from an available
/// Destination (R10). A sync can deliver a rename or a type change as a
/// removed folder and then a new one (AE4), up to 2 minutes later (R26 and
/// KTD12 of the note types plan).
pub const DELETE_WAIT: Duration = Duration::from_secs(120);

/// How long `transcript.md` or `summary.md` can differ from `session.md`,
/// or be missing, before the note counts as a hand edit (R8, KTD7). A sync
/// of one note usually finishes in a few seconds.
pub const HAND_EDIT_WAIT: Duration = Duration::from_secs(60);

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
    /// Whether the scan read the home folder `home`. A targeted scan of a
    /// type folder also covers the home folders in it (KTD4).
    pub(crate) fn covers(&self, home: &Path) -> bool {
        match self {
            Coverage::Full => true,
            Coverage::Folders(folders) => {
                folders.contains(home) || home.parent().is_some_and(|p| folders.contains(p))
            }
        }
    }

    /// Whether `scan`, with this coverage, read the note folder `folder`.
    pub(crate) fn read(&self, scan: &LibraryScan, folder: &Path) -> bool {
        match self {
            Coverage::Full => scan::place_in(folder, &scan_roots(scan)).is_some(),
            Coverage::Folders(_) => self.covers(folder),
        }
    }

    /// Whether a wait for the note folder `folder`, which `scan` did not
    /// find, stays after the scan: the scan did not read the folder. A full
    /// scan of some Destinations says nothing about the folders in the
    /// others. When a Destination is removed,
    /// [`Reconciler::retain_roots`] drops the waits of its folders.
    pub(crate) fn keeps_wait(&self, scan: &LibraryScan, folder: &Path) -> bool {
        !self.read(scan, folder)
    }
}

/// The Destinations of `scan`.
fn scan_roots(scan: &LibraryScan) -> Vec<PathBuf> {
    scan.roots.iter().map(|r| r.root.clone()).collect()
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
    /// After [`Reconciler::run`]: the library notices of Home changed, for
    /// example the folders in [`LibraryStatus::problems`].
    pub notices: bool,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty() && !self.notices
    }
}

/// What a scan found for the result documents of a note that do not match
/// `session.md`. When a scan finds another value, the wait starts again
/// (KTD7).
#[derive(Debug, Clone, PartialEq, Eq)]
struct MismatchKey {
    recorded_transcript: Option<String>,
    recorded_summary: Option<String>,
    transcript: DocumentState,
    transcript_fingerprint: Option<String>,
    summary: DocumentState,
    summary_fingerprint: Option<String>,
}

impl MismatchKey {
    /// `None` when both result documents of `note` match, or when the note
    /// has no result yet (R6, KTD1): it waits for no result document.
    fn of(note: &NoteFolder) -> Option<Self> {
        if note.transcript == DocumentState::Matches && note.summary == DocumentState::Matches
            || note.doc.result_flag().is_some()
        {
            return None;
        }
        Some(Self {
            recorded_transcript: note.doc.transcript_fingerprint.clone(),
            recorded_summary: note.doc.summary_fingerprint.clone(),
            transcript: note.transcript,
            transcript_fingerprint: note.transcript_fingerprint.clone(),
            summary: note.summary,
            summary_fingerprint: note.summary_fingerprint.clone(),
        })
    }

    /// Whether a document differs or is missing. Only such a document can
    /// become a hand edit: a document that cannot be read waits (R8).
    fn can_expire(&self) -> bool {
        let expires = |state| matches!(state, DocumentState::Differs | DocumentState::Missing);
        expires(self.transcript) || expires(self.summary)
    }
}

/// A note whose result documents do not match `session.md`.
#[derive(Debug, Clone)]
struct Mismatch {
    key: MismatchKey,
    /// When a scan first found `key`.
    since: Instant,
    folder: PathBuf,
    /// A scan found the mismatch at or after its deadline.
    expired: bool,
}

/// The title of a `session.md` that differs from the title that Longhand
/// last wrote into it (KTD5). A title write changes `session.md` and
/// `longhand.json`, and a sync can deliver them in either order. When the
/// two still differ after [`HAND_EDIT_WAIT`], a person or an agent changed
/// the title. When a scan finds other values, the wait starts again.
#[derive(Debug, Clone)]
struct TitleWait {
    key: TitleKey,
    since: Instant,
    /// A scan found the difference at or after its deadline.
    expired: bool,
    folder: PathBuf,
}

/// The values that a [`TitleWait`] waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TitleKey {
    /// The title of `session.md`.
    session_md: Option<String>,
    /// The last-written title.
    written: Option<String>,
}

impl TitleWait {
    /// The key of `doc` with the last-written title `written`, in the new
    /// layout. `None` when the titles are equal, or in the old layout.
    fn key_of(layout: Layout, doc: &SessionDoc, written: Option<&Title>) -> Option<TitleKey> {
        let written = written.map(|w| w.text.clone());
        (layout == Layout::New && doc.title != written).then(|| TitleKey {
            session_md: doc.title.clone(),
            written,
        })
    }
}

/// Applies scans to the store, and keeps the pending deletes and the
/// mismatches between two scans. They stay in memory only (KTD7, KTD8).
#[derive(Debug, Clone)]
pub struct Reconciler {
    /// When each pending delete started.
    pending: HashMap<String, Instant>,
    /// The notes whose result documents do not match `session.md`, by id.
    mismatches: HashMap<String, Mismatch>,
    /// The notes whose `session.md` title differs from the title that
    /// Longhand last wrote, by id (KTD5).
    titles: HashMap<String, TitleWait>,
    read_timeout: Duration,
    /// The run IDs of this computer's in-use markers ([`ScanOptions::own_runs`]).
    own_runs: Vec<String>,
    /// What the scans of [`Reconciler::run`] found (KTD10).
    status: Arc<LibraryStatus>,
}

impl Default for Reconciler {
    fn default() -> Self {
        Self::new(&ScanOptions::default())
    }
}

impl Reconciler {
    /// A reconciler whose scans use `options`.
    pub fn new(options: &ScanOptions) -> Self {
        Self {
            pending: HashMap::new(),
            mismatches: HashMap::new(),
            titles: HashMap::new(),
            read_timeout: options.read_timeout,
            own_runs: options.own_runs.clone(),
            status: Arc::default(),
        }
    }

    /// Count the in-use markers of the runs `run_ids` as this computer's
    /// own work: they do not make a note in use on another computer
    /// (KTD5).
    pub fn with_own_runs(mut self, run_ids: impl IntoIterator<Item = String>) -> Self {
        self.own_runs = run_ids.into_iter().collect();
        self
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

    /// The earliest time at which a pending delete can complete, at which
    /// the wait for the sync of a note ends, or at which the status can
    /// report another version of a note. A reconcile at or after this time,
    /// with a new scan, forgets that row, reports that note as a hand edit
    /// (KTD7) or reports the other version.
    pub fn next_due(&self) -> Option<Instant> {
        let deletes = self.pending.values().map(|since| *since + DELETE_WAIT);
        let hand_edits = self
            .mismatches
            .values()
            .filter(|mismatch| !mismatch.expired && mismatch.key.can_expire())
            .map(|mismatch| mismatch.since + HAND_EDIT_WAIT);
        let titles = self
            .titles
            .values()
            .filter(|wait| !wait.expired)
            .map(|wait| wait.since + HAND_EDIT_WAIT);
        deletes
            .chain(hand_edits)
            .chain(titles)
            .chain(self.status.next_due())
            .min()
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
        self.run_at(store, target, claimed, Instant::now())
    }

    /// Drop the waits and the status of each note folder in none of
    /// `roots`, the Destinations of the library after a change. A scan
    /// keeps the waits of the folders that it did not read, so the waits
    /// of a removed Destination stay until this call. Rows are not
    /// changed: [`forget_outside`] forgets them.
    pub fn retain_roots(&mut self, roots: &[PathBuf]) {
        let inside = |folder: &Path| scan::place_in(folder, roots).is_some();
        self.mismatches
            .retain(|_, mismatch| inside(&mismatch.folder));
        self.titles.retain(|_, wait| inside(&wait.folder));
        self.status.retain_roots(roots);
    }

    /// [`Reconciler::run`] with `now` as the time of the scan.
    pub fn run_at(
        &mut self,
        store: &Mutex<Store>,
        target: &Target,
        claimed: &dyn Fn(&str) -> bool,
        now: Instant,
    ) -> Result<Changes, StoreError> {
        let lock = || store.lock().unwrap_or_else(|e| e.into_inner());
        let known = Known::from_records(&lock().sessions()?);
        let options = ScanOptions {
            read_timeout: self.read_timeout,
            own_runs: self.own_runs.clone(),
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
        self.track_titles(&result, &coverage, now);
        let stale = self.stale(&lock(), &result, claimed)?;
        // A cloud file can take seconds to read: the lock is free meanwhile.
        let rereads = self.read_again(stale);
        let mut store = lock();
        let mut changes = self.apply(&mut store, &result, &coverage, now, claimed, &rereads)?;
        // A note that this computer moves or renames now can be in two
        // folders: the other folder is not another version.
        let moving = moved_here(&store)?;
        let hand_edits = self.track_mismatches(&result, &coverage, now);
        let notices = self.status.notices_revision();
        let noted = self.status.apply(
            &result,
            &coverage,
            &store.sessions()?,
            &moving,
            &hand_edits,
            now,
        );
        changes.notices = self.status.notices_revision() != notices;
        // A file that changed in a note folder can leave the row as it is,
        // for example a new summary.md. The note is reported as changed, so
        // an open note page reads its documents again.
        let touched = match target {
            Target::Folders { folders, .. } => result
                .notes
                .iter()
                .filter(|note| folders.contains(&note.path))
                .map(|note| note.doc.session_id.clone())
                .collect(),
            Target::Full(_) => Vec::new(),
        };
        changes.changed.extend(
            noted
                .into_iter()
                .chain(touched)
                .filter(|id| !changes.removed.contains(id)),
        );
        changes.changed.sort();
        changes.changed.dedup();
        Ok(changes)
    }

    /// Record the mismatches that `scan` found at `now` (KTD7). A note
    /// whose documents match, or that the scan read and did not find, has
    /// no mismatch. Returns the notes whose mismatch started at least
    /// [`HAND_EDIT_WAIT`] ago: they are hand edits.
    fn track_mismatches(
        &mut self,
        scan: &LibraryScan,
        coverage: &Coverage,
        now: Instant,
    ) -> HashSet<String> {
        let found: HashSet<&str> = scan
            .notes
            .iter()
            .map(|note| note.doc.session_id.as_str())
            .collect();
        self.mismatches.retain(|id, mismatch| {
            found.contains(id.as_str()) || coverage.keeps_wait(scan, &mismatch.folder)
        });
        let mut hand_edits = HashSet::new();
        for note in &scan.notes {
            let id = &note.doc.session_id;
            let Some(key) = MismatchKey::of(note) else {
                self.mismatches.remove(id);
                continue;
            };
            let mismatch = self
                .mismatches
                .entry(id.clone())
                .or_insert_with(|| Mismatch {
                    key: key.clone(),
                    since: now,
                    folder: note.path.clone(),
                    expired: false,
                });
            if mismatch.key != key {
                mismatch.key = key;
                mismatch.since = now;
                mismatch.expired = false;
            }
            mismatch.folder = note.path.clone();
            if now.saturating_duration_since(mismatch.since) >= HAND_EDIT_WAIT {
                mismatch.expired = true;
                hand_edits.insert(id.clone());
            }
        }
        hand_edits
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
        self.track_titles(scan, coverage, now);
        let stale = self.stale(store, scan, claimed)?;
        let rereads = self.read_again(stale);
        self.apply(store, scan, coverage, now, claimed, &rereads)
    }

    /// Record the titles that `scan` found at `now` that differ from the
    /// title that Longhand last wrote (KTD5). A note whose titles are
    /// equal, or that the scan read and did not find, has no wait.
    fn track_titles(&mut self, scan: &LibraryScan, coverage: &Coverage, now: Instant) {
        let found: HashMap<&str, &NoteFolder> = scan
            .notes
            .iter()
            .map(|note| (note.doc.session_id.as_str(), note))
            .collect();
        self.titles.retain(|id, wait| {
            found.contains_key(id.as_str()) || coverage.keeps_wait(scan, &wait.folder)
        });
        for (id, note) in found {
            let Some(key) = TitleWait::key_of(note.layout, &note.doc, note.written_title.as_ref())
            else {
                self.titles.remove(id);
                continue;
            };
            let wait = self
                .titles
                .entry(id.to_string())
                .or_insert_with(|| TitleWait {
                    key: key.clone(),
                    since: now,
                    expired: false,
                    folder: note.path.clone(),
                });
            wait.folder = note.path.clone();
            if wait.key != key {
                wait.key = key;
                wait.since = now;
                wait.expired = false;
            }
            if now.saturating_duration_since(wait.since) >= HAND_EDIT_WAIT {
                wait.expired = true;
            }
        }
    }

    /// `contents` of the note `id`, with the title rule of [`TitleWait`]
    /// applied: the title of `session.md` is a user title only when the
    /// wait for the same values has expired.
    fn with_title_rule(&self, id: &str, mut contents: FolderContents) -> FolderContents {
        let key = TitleWait::key_of(
            contents.layout,
            &contents.doc,
            contents.written_title.as_ref(),
        );
        contents.title_settled = key.is_some_and(|key| {
            self.titles
                .get(id)
                .is_some_and(|wait| wait.expired && wait.key == key)
        });
        contents
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
        let intents = moved_here(store)?;
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
            let retyped = record.note_type != note.note_type;
            let refreshes = match refresh_scope(&record, claimed) {
                Some(scope) => refresh::differs(
                    store,
                    id,
                    &self.with_title_rule(id, FolderContents::from_scan(note)),
                    scope,
                )?,
                None => false,
            };
            let changes = moved || retyped || refreshes;
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
                let contents = FolderContents::read(&stale.folder, self.read_timeout)
                    .map(|contents| self.with_title_rule(&stale.id, contents));
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
        let intents = moved_here(store)?;
        self.pending.retain(|id, _| records.contains_key(id));

        let mut changed = Vec::new();
        for note in &scan.notes {
            let id = &note.doc.session_id;
            self.pending.remove(id);
            let updated = match records.get(id) {
                // The scan read the folder without the lock. A delete removes
                // the folder before it deletes the row under the lock, so a
                // folder that is gone now is a deleted note, not a new one.
                None if !scan::is_note_folder(&note.path) => false,
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
                    .chain(&scan.waiting_for_record)
                    .filter_map(|problem| problem.known_id.as_deref()),
            )
            .collect();
        // A folder that waits for its `longhand.json`, that is not valid, or
        // that cannot be read, and has no id can be the moved folder of any
        // row of its Destination: a rename moves the folder, and the sync can
        // deliver `session.md` first or not deliver the files yet. While such
        // a folder is there, no row of that Destination is forgotten.
        let unidentified: HashSet<&Path> = scan
            .invalid
            .iter()
            .chain(&scan.waiting_for_record)
            .chain(&scan.unreadable)
            .filter(|problem| problem.known_id.is_none())
            .map(|problem| problem.root.as_path())
            .collect();
        let roots = scan_roots(scan);
        let mut removed = Vec::new();
        for record in records.values() {
            let Some(home) = record.home_path.as_deref().map(Path::new) else {
                continue;
            };
            if present.contains(record.id.as_str()) || !coverage.covers(home) {
                continue;
            }
            // A scan of some Destinations says nothing about the others.
            let Some(place) = scan::place_in(home, &roots) else {
                continue;
            };
            // The wait starts again when the Destination is available and
            // has no folder with no id.
            if !scan.is_available(&place.root) || unidentified.contains(place.root.as_path()) {
                self.pending.remove(&record.id);
                continue;
            }
            let since = *self.pending.entry(record.id.clone()).or_insert(now);
            let due = now.saturating_duration_since(since) >= DELETE_WAIT;
            // Only a full scan that misses the id can forget the row, and
            // only while the Destination still has its marker (KTD12).
            if due
                && *coverage == Coverage::Full
                && scan::marker(&place.root).exists()
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
        Ok(Changes {
            changed,
            removed,
            notices: false,
        })
    }
}

/// The rows whose folders this computer moves itself: a folder rename
/// intent (KTD8), or a note that the Move has not finished (KTD10). The
/// reconcile keeps their rows as they are.
fn moved_here(store: &Store) -> Result<HashSet<String>, StoreError> {
    Ok(store
        .rename_intents()?
        .into_iter()
        .map(|(id, _)| id)
        .chain(store.moving_notes()?)
        .collect())
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
    removed_audio: Vec<String>,
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
            removed_audio: store.removed_audio(id)?,
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
    // The folder decides the type (KTD1, R29). The Destination must still
    // have its marker at the time of the change.
    let retyped = record.note_type != note.note_type && scan::marker(&note.root).exists();
    if retyped {
        log::info!(
            "library: the type of {id} is now {}",
            note.note_type.as_deref().unwrap_or("General")
        );
        store.set_note_type(id, note.note_type.as_deref())?;
    }
    let changed = moved || retyped;
    let Some(scope) = refresh_scope(record, claimed) else {
        return Ok(changed);
    };
    match refresh::apply_scoped(store, id, contents, scope) {
        Ok(refreshed) => Ok(changed || refreshed.changed),
        Err(RefreshError::Store(e)) => Err(e),
        Err(e) => {
            log::warn!("library: cannot refresh {id}: {e}");
            Ok(changed)
        }
    }
}

/// Which parts of the row `record` a refresh takes from its folder. A
/// Saved row that no stage has claimed takes every part. A busy row with
/// a home folder takes only the notes, so a notes edit from another
/// computer shows while the note records or processes here (R22, KTD6).
/// The work on this computer owns the rest of the row. A row with no home
/// folder has no folder to follow.
fn refresh_scope(record: &SessionRecord, claimed: bool) -> Option<Scope> {
    if !busy(record, claimed) {
        Some(Scope::All)
    } else if record.home_path.is_some() {
        Some(Scope::Notes)
    } else {
        None
    }
}

/// Adopt a note that has no row (KTD1). A note whose `notes.md` has not
/// arrived starts with empty notes, and a later refresh reads them.
fn adopt(store: &mut Store, note: &NoteFolder) -> Result<bool, StoreError> {
    let adoption = Adoption {
        removed_audio: note.removed_audio.clone(),
        note_type: note.note_type.clone(),
        ..Adoption::from_doc(
            &note.doc,
            note.notes.clone().unwrap_or_default(),
            note.path.to_string_lossy().into_owned(),
            note.transcript_fingerprint.clone(),
        )
    };
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

/// Forget each row whose home folder is in none of `roots`, for example
/// the notes of an old storage folder after a change of folder with Leave
/// them (R7, R8). A row that is busy or has a rename intent stays, and a
/// later call forgets it when it is free. No folder changes. With no root,
/// nothing is forgotten. Returns the forgotten ids, sorted.
pub fn forget_outside(
    store: &mut Store,
    roots: &[PathBuf],
    claimed: &dyn Fn(&str) -> bool,
) -> Result<Vec<String>, StoreError> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    let mut removed = Vec::new();
    for record in store.sessions()? {
        let Some(home) = record.home_path.as_deref().map(Path::new) else {
            continue;
        };
        if scan::place_in(home, roots).is_some() {
            continue;
        }
        if !busy(&record, claimed(&record.id)) && store.forget(&record.id)? {
            log::info!(
                "library: {} is not in the storage folder, so its row is removed",
                record.id
            );
            removed.push(record.id);
        }
    }
    removed.sort();
    Ok(removed)
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
            ends_in_pause: false,
        };
        let folder = root.path().join("2026-09-25 0900 Planning");
        fs::create_dir(&folder).unwrap();
        let doc = SessionDoc::new("a1", None, &[segment]).unwrap();
        session_save::note_files::write_doc(&folder, &doc).unwrap();
        fs::write(folder.join("notes.md"), "first\n").unwrap();
        let store = Mutex::new(Store::open_in_memory().unwrap());
        let mut reconciler = Reconciler::default();
        let target = Target::Full(vec![root.path().to_path_buf()]);
        let changes = reconciler.run(&store, &target, &never_claimed).unwrap();
        assert_eq!(changes.changed, vec!["a1".to_string()]);
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
            s.set_notes("a1", "mine\n").unwrap();
            fs::write(folder.join("notes.md"), "mine\n").unwrap();
        }

        let mut s = store.lock().unwrap();
        let changes = apply(&mut s, &mut reconciler, &result, &rereads);
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(s.notes("a1").unwrap(), "mine\n");
        drop(s);

        // The next scan reads the folder with this computer's notes.
        let target = Target::Full(vec![root.path().to_path_buf()]);
        reconciler.run(&store, &target, &never_claimed).unwrap();
        assert_eq!(store.lock().unwrap().notes("a1").unwrap(), "mine\n");
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
            s.set_home_path("a1", &renamed.to_string_lossy()).unwrap();
        }

        let mut s = store.lock().unwrap();
        let changes = apply(&mut s, &mut reconciler, &result, &rereads);
        assert!(changes.changed.is_empty(), "{changes:?}");
        let home = s.session("a1").unwrap().unwrap().home_path.unwrap();
        assert_eq!(PathBuf::from(home), renamed);
    }

    #[test]
    fn a_note_that_changes_only_in_its_folder_is_refreshed_by_a_run() {
        let (root, folder, store, mut reconciler) = adopted();
        fs::write(folder.join("notes.md"), "theirs\n").unwrap();
        let target = Target::Full(vec![root.path().to_path_buf()]);
        let changes = reconciler.run(&store, &target, &never_claimed).unwrap();
        assert_eq!(changes.changed, vec!["a1".to_string()]);
        assert_eq!(store.lock().unwrap().notes("a1").unwrap(), "theirs\n");
    }

    /// KTD10: during a Move the note is in both folders for a time. The
    /// reconcile keeps the row as the Move sets it, as for a rename intent.
    #[test]
    fn a_note_that_the_move_works_on_keeps_its_row() {
        let (root, folder, store, mut reconciler) = adopted();
        let new = tempfile::tempdir().unwrap();
        crate::scan::write_marker(new.path());
        let placed = new.path().join("2026-09-25 0900 Planning");
        fs::create_dir(&placed).unwrap();
        fs::copy(folder.join("session.md"), placed.join("session.md")).unwrap();
        fs::copy(folder.join("longhand.json"), placed.join("longhand.json")).unwrap();
        fs::write(placed.join("notes.md"), "placed\n").unwrap();
        fs::write(folder.join("notes.md"), "theirs\n").unwrap();
        store
            .lock()
            .unwrap()
            .open_move(root.path(), new.path())
            .unwrap();
        fs::remove_dir_all(&folder).unwrap();

        let target = Target::Full(vec![root.path().to_path_buf(), new.path().to_path_buf()]);
        let changes = reconciler.run(&store, &target, &never_claimed).unwrap();
        assert!(changes.is_empty(), "{changes:?}");
        let s = store.lock().unwrap();
        let record = s.session("a1").unwrap().unwrap();
        assert_eq!(record.home_path.map(PathBuf::from), Some(folder.clone()));
        assert_eq!(s.notes("a1").unwrap(), "first\n");
        drop(s);

        // The pending delete of the old folder does not forget the row.
        let result = scan_of(root.path(), &store);
        let later = Instant::now() + DELETE_WAIT * 2;
        let mut s = store.lock().unwrap();
        reconciler
            .reconcile(
                &mut s,
                &result,
                &Coverage::Full,
                Instant::now(),
                &never_claimed,
            )
            .unwrap();
        let changes = reconciler
            .reconcile(&mut s, &result, &Coverage::Full, later, &never_claimed)
            .unwrap();
        assert!(changes.removed.is_empty(), "{changes:?}");
        assert!(s.session("a1").unwrap().is_some());
    }

    #[test]
    fn a_note_in_a_root_is_not_forgotten() {
        let (root, _folder, store, _) = adopted();
        let removed = forget_outside(
            &mut store.lock().unwrap(),
            &[root.path().to_path_buf()],
            &never_claimed,
        )
        .unwrap();
        assert!(removed.is_empty());
        assert!(store.lock().unwrap().session("a1").unwrap().is_some());
    }

    /// Leave them (R7): the library forgets the notes of the old folder and
    /// keeps their files.
    #[test]
    fn a_note_outside_every_root_is_forgotten_and_its_folder_stays() {
        let (_root, folder, store, _) = adopted();
        let other = tempfile::tempdir().unwrap();
        let removed = forget_outside(
            &mut store.lock().unwrap(),
            &[other.path().to_path_buf()],
            &never_claimed,
        )
        .unwrap();
        assert_eq!(removed, vec!["a1".to_string()]);
        assert!(store.lock().unwrap().session("a1").unwrap().is_none());
        assert!(folder.join("session.md").exists());
        assert!(folder.join("notes.md").exists());
    }

    #[test]
    fn a_busy_note_outside_every_root_is_forgotten_when_it_is_free() {
        let (_root, _folder, store, _) = adopted();
        let other = tempfile::tempdir().unwrap();
        let roots = [other.path().to_path_buf()];
        let removed = forget_outside(&mut store.lock().unwrap(), &roots, &|_| true).unwrap();
        assert!(removed.is_empty());
        assert!(store.lock().unwrap().session("a1").unwrap().is_some());
        let removed = forget_outside(&mut store.lock().unwrap(), &roots, &never_claimed).unwrap();
        assert_eq!(removed, vec!["a1".to_string()]);
    }

    #[test]
    fn with_no_root_and_for_a_note_with_no_home_nothing_is_forgotten() {
        let (_root, _folder, store, _) = adopted();
        let start = DateTime::parse_from_rfc3339("2026-09-25T10:00:00-05:00").unwrap();
        store
            .lock()
            .unwrap()
            .create_session("recording", start, None)
            .unwrap();
        let removed = forget_outside(&mut store.lock().unwrap(), &[], &never_claimed).unwrap();
        assert!(removed.is_empty());
        let other = tempfile::tempdir().unwrap();
        let removed = forget_outside(
            &mut store.lock().unwrap(),
            &[other.path().to_path_buf()],
            &never_claimed,
        )
        .unwrap();
        assert_eq!(removed, vec!["a1".to_string()]);
        assert!(
            store
                .lock()
                .unwrap()
                .session("recording")
                .unwrap()
                .is_some()
        );
    }

    /// Covers C18: a scan that read the folder before a delete must not
    /// adopt the note again after the delete.
    #[test]
    fn a_note_deleted_after_the_scan_read_it_is_not_adopted_again() {
        let (root, folder, store, mut reconciler) = adopted();
        let result = scan_of(root.path(), &store);
        // The delete removes the folder, then the row under the lock.
        fs::remove_dir_all(&folder).unwrap();
        assert!(store.lock().unwrap().forget("a1").unwrap());

        let mut s = store.lock().unwrap();
        let changes = apply(&mut s, &mut reconciler, &result, &HashMap::new());
        assert!(changes.changed.is_empty(), "{changes:?}");
        assert!(s.session("a1").unwrap().is_none());
    }

    /// Covers C19: a renamed folder that cannot be read and has no id can
    /// be the folder of any row of its Destination, so no row is forgotten.
    #[cfg(unix)]
    #[test]
    fn a_renamed_folder_that_cannot_be_read_keeps_the_rows_of_its_destination() {
        use std::os::unix::fs::PermissionsExt;
        let (root, folder, store, mut reconciler) = adopted();
        let renamed = root.path().join("2026-09-25 0900 Budget");
        fs::rename(&folder, &renamed).unwrap();
        let files = [renamed.join("session.md"), renamed.join("longhand.json")];
        for file in &files {
            fs::set_permissions(file, fs::Permissions::from_mode(0o000)).unwrap();
        }
        let result = scan_of(root.path(), &store);
        for file in &files {
            fs::set_permissions(file, fs::Permissions::from_mode(0o644)).unwrap();
        }
        // A process that ignores the permissions (root) reads the folder.
        if result.unreadable.is_empty() {
            return;
        }
        assert_eq!(result.unreadable[0].known_id, None);

        let mut s = store.lock().unwrap();
        let now = Instant::now();
        for at in [now, now + DELETE_WAIT * 2] {
            let changes = reconciler
                .reconcile(&mut s, &result, &Coverage::Full, at, &never_claimed)
                .unwrap();
            assert!(changes.removed.is_empty(), "{changes:?}");
        }
        assert!(s.session("a1").unwrap().is_some());
    }

    /// A note folder in `root` named `name` with the id `id`, no `notes.md`
    /// (the status waits for it) and a title in `session.md` that differs
    /// from the title that Longhand last wrote (a title wait).
    fn waiting_note(root: &Path, name: &str, id: &str) -> PathBuf {
        let folder = root.join(name);
        fs::create_dir(&folder).unwrap();
        let start = DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap();
        let mut doc = SessionDoc::at(id, start, None, &[]);
        doc.title = Some("Planning".into());
        session_save::note_files::write_doc(&folder, &doc).unwrap();
        fs::write(folder.join("session.md"), "---\ntitle: Budget\n---\n").unwrap();
        folder
    }

    /// Covers C89: a full scan of one Destination keeps the waits and the
    /// status of the notes in another, and a change of the Destinations
    /// drops them.
    #[test]
    fn a_full_scan_of_one_destination_keeps_the_waits_of_another() {
        let (root, _folder, store, mut reconciler) = adopted();
        let other = tempfile::tempdir().unwrap();
        crate::scan::write_marker(other.path());
        waiting_note(other.path(), "2026-09-25 0900 Budget", "b1");
        let both = Target::Full(vec![root.path().to_path_buf(), other.path().to_path_buf()]);
        reconciler.run(&store, &both, &never_claimed).unwrap();
        assert!(reconciler.titles.contains_key("b1"));
        assert!(!reconciler.status.note("b1").waiting.is_empty());

        let one = Target::Full(vec![root.path().to_path_buf()]);
        reconciler.run(&store, &one, &never_claimed).unwrap();
        assert!(reconciler.titles.contains_key("b1"));
        assert!(!reconciler.status.note("b1").waiting.is_empty());

        reconciler.retain_roots(&[root.path().to_path_buf()]);
        assert!(!reconciler.titles.contains_key("b1"));
        assert!(reconciler.status.note("b1").waiting.is_empty());
    }
}
