//! What the latest scans found about each note and each Destination
//! (KTD8, KTD10): the parts that wait for the sync, the other versions of
//! the note, and the Destinations that are unavailable. The note page and
//! Home read it, so no command scans a folder. It stays in memory only,
//! and each scan builds it again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use session_core::{SessionRecord, SessionState};

use crate::reconcile::Coverage;
use crate::scan::{DocumentState, LibraryScan, NoteFolder, RemoteState, RootState, Waiting};

/// How long the scans must find another version of a note before the
/// status reports it (R15). A move of the note can show two folders for a
/// short time: a scan that runs during a rename or a type move, or a sync
/// that delivers the new folder before it removes the old one. A sync can
/// take up to 2 minutes for that, as for [`crate::reconcile::DELETE_WAIT`].
/// A sync service can also remove a conflict copy that it made for a short
/// time, so a conflict copy gets the same wait.
pub const OTHER_VERSION_WAIT: Duration = Duration::from_secs(120);

/// Whether the home folder `home` is in one of `roots`, directly or in a
/// type folder (KTD4 of the note types plan).
fn in_any(home: &Path, roots: &HashSet<PathBuf>) -> bool {
    roots
        .iter()
        .any(|root| session_core::folder::note_type_in(root, home).is_some())
}

/// What the latest scan found about one note folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteStatus {
    /// The parts that the sync has not delivered yet (R6).
    pub waiting: Vec<Waiting>,
    /// Conflict copies of the documents, then other folders with the same
    /// note id (R15). Each is here only after the scans found it for
    /// [`OTHER_VERSION_WAIT`].
    pub other_versions: Vec<PathBuf>,
    /// `session.md` exists but cannot be read (KTD5): the note waits as if
    /// its Destination were unavailable.
    pub unreadable: bool,
    /// What the folder shows about work on the note on another computer
    /// ([`NoteFolder::remote`]). Read it with [`LibraryStatus::remote`],
    /// which also checks the row.
    pub remote: Option<RemoteState>,
}

impl NoteStatus {
    /// Whether the transcript or the summary waits for the sync (R7). The
    /// actions that read or replace the result do not start then.
    pub fn result_waits(&self) -> bool {
        self.waiting
            .iter()
            .any(|part| matches!(part, Waiting::Transcript | Waiting::Summary))
    }
}

/// The parts that `note` waits for. After the wait for the sync ends
/// (`hand_edit`), a result document that differs or is missing is a hand
/// edit and does not wait. A document that cannot be read still waits
/// (R8).
fn waiting(note: &NoteFolder, hand_edit: bool) -> Vec<Waiting> {
    let expired = |state: DocumentState| hand_edit && state != DocumentState::Unreadable;
    note.waiting
        .iter()
        .filter(|part| match part {
            Waiting::Transcript => !expired(note.transcript),
            Waiting::Summary => !expired(note.summary),
            Waiting::Notes | Waiting::Audio(_) => true,
        })
        .cloned()
        .collect()
}

/// Another version of a note that the scans found.
#[derive(Debug, Clone, Copy)]
struct Seen {
    /// When a scan first found it.
    since: Instant,
    /// A duplicate folder, not a conflict copy.
    folder: bool,
    /// The status reports it.
    reported: bool,
}

#[derive(Debug, Default)]
struct Inner {
    /// Each note with something to show, and the folder that the scan read.
    notes: HashMap<String, (PathBuf, NoteStatus)>,
    unavailable: HashSet<PathBuf>,
    /// The other versions of each note, reported or not yet, and the
    /// folder of the note that the scan read.
    seen: HashMap<String, (PathBuf, HashMap<PathBuf, Seen>)>,
}

/// The other versions of `note` that the scan found at `now`, and the
/// duplicate folders that the scan did not read (`unread`), each with the
/// time when a scan first found it. `before` holds the earlier times.
/// `held` is a note that this computer moves now: its other folders are
/// not other versions. Each version that waited for
/// [`OTHER_VERSION_WAIT`] is marked as reported.
fn track(
    note: &NoteFolder,
    held: bool,
    before: Option<&HashMap<PathBuf, Seen>>,
    unread: &dyn Fn(&Path) -> bool,
    now: Instant,
) -> HashMap<PathBuf, Seen> {
    let entry = |path: &PathBuf, folder: bool| {
        let since = before.and_then(|b| b.get(path)).map_or(now, |s| s.since);
        let reported = now.saturating_duration_since(since) >= OTHER_VERSION_WAIT;
        Seen {
            since,
            folder,
            reported,
        }
    };
    let mut seen = HashMap::new();
    for path in &note.conflict_copies {
        seen.insert(path.clone(), entry(path, false));
    }
    if held {
        return seen;
    }
    for path in &note.duplicate_folders {
        seen.insert(path.clone(), entry(path, true));
    }
    // A targeted scan reads only some folders. A folder that it did not
    // read keeps its time, so its wait continues.
    for (path, old) in before.into_iter().flatten() {
        if old.folder && *path != note.path && unread(path) {
            seen.entry(path.clone())
                .or_insert_with(|| entry(path, true));
        }
    }
    seen
}

/// The reported paths in `seen`: the conflict copies, then the duplicate
/// folders, each sorted.
fn reported(seen: &HashMap<PathBuf, Seen>) -> Vec<PathBuf> {
    let mut due: Vec<(bool, &PathBuf)> = seen
        .iter()
        .filter(|(_, s)| s.reported)
        .map(|(path, s)| (s.folder, path))
        .collect();
    due.sort();
    due.into_iter().map(|(_, path)| path.clone()).collect()
}

/// The status of the library, shared by the reconciler that writes it and
/// the commands that read it.
#[derive(Debug, Default)]
pub struct LibraryStatus {
    inner: Mutex<Inner>,
}

impl LibraryStatus {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The status of the note `id`. A note that the scans found complete,
    /// or have not read yet, has the default status.
    pub fn note(&self, id: &str) -> NoteStatus {
        self.lock()
            .notes
            .get(id)
            .map(|(_, status)| status.clone())
            .unwrap_or_default()
    }

    /// The work on the note of `record` on another computer, as the latest
    /// scan found it (KTD5, R12, R20, R21, R23). Only a Saved row can show
    /// it: a row that records, processes or waits on this computer does
    /// that work here, and its folder shows this computer's own work.
    pub fn remote(&self, record: &SessionRecord) -> Option<RemoteState> {
        if record.state != SessionState::Saved {
            return None;
        }
        self.lock()
            .notes
            .get(&record.id)
            .and_then(|(_, status)| status.remote)
    }

    /// Whether the note with the home folder `home` cannot be read now: its
    /// Destination is unavailable, or its `session.md` cannot be read (R16).
    /// The home folder can be in a type folder of its Destination (KTD4).
    pub fn storage_unavailable(&self, id: &str, home: Option<&Path>) -> bool {
        let inner = self.lock();
        let root_gone = home.is_some_and(|home| in_any(home, &inner.unavailable));
        root_gone || inner.notes.get(id).is_some_and(|(_, s)| s.unreadable)
    }

    /// The earliest time at which another version that the status does
    /// not report yet reaches [`OTHER_VERSION_WAIT`]. A scan at or after
    /// this time reports it.
    pub fn next_due(&self) -> Option<Instant> {
        self.lock()
            .seen
            .values()
            .flat_map(|(_, versions)| versions.values())
            .filter(|s| !s.reported)
            .map(|s| s.since + OTHER_VERSION_WAIT)
            .min()
    }

    /// Apply a scan made at `now`. `records` are the rows after the
    /// reconcile of the scan. `moving` holds the notes that the Move has
    /// not finished (KTD10), and the notes whose folder this computer
    /// renames: such a note can be in the old and the new folder, and that
    /// other folder is not another version (R6). `hand_edits` holds the
    /// notes whose wait for the sync ended (KTD7). Returns the ids whose
    /// status changed, sorted.
    pub fn apply(
        &self,
        scan: &LibraryScan,
        coverage: &Coverage,
        records: &[SessionRecord],
        moving: &HashSet<String>,
        hand_edits: &HashSet<String>,
        now: Instant,
    ) -> Vec<String> {
        let mut inner = self.lock();
        let mut changed = HashSet::new();

        let mut roots_changed = HashSet::new();
        for root in &scan.roots {
            let gone = root.state != RootState::Available;
            let was_gone = inner.unavailable.contains(&root.root);
            if gone != was_gone {
                roots_changed.insert(root.root.clone());
                if gone {
                    inner.unavailable.insert(root.root.clone());
                } else {
                    inner.unavailable.remove(&root.root);
                }
            }
        }
        for record in records {
            let home = record.home_path.as_deref().map(Path::new);
            if home.is_some_and(|home| in_any(home, &roots_changed)) {
                changed.insert(record.id.clone());
            }
        }

        let unread = |path: &Path| !coverage.read(scan, path);
        let mut found: HashMap<String, (PathBuf, NoteStatus)> = HashMap::new();
        let mut seen = HashMap::new();
        for note in &scan.notes {
            let id = &note.doc.session_id;
            let before = inner.seen.get(id).map(|(_, versions)| versions);
            let versions = track(note, moving.contains(id), before, &unread, now);
            let status = NoteStatus {
                waiting: waiting(note, hand_edits.contains(id)),
                other_versions: reported(&versions),
                unreadable: false,
                remote: note.remote,
            };
            found.insert(id.clone(), (note.path.clone(), status));
            if !versions.is_empty() {
                seen.insert(id.clone(), (note.path.clone(), versions));
            }
        }
        for problem in &scan.unreadable {
            if let Some(id) = &problem.known_id {
                let status = NoteStatus {
                    unreadable: true,
                    ..NoteStatus::default()
                };
                found.insert(id.clone(), (problem.path.clone(), status));
            }
        }

        // A folder that the scan read and that gave no note has nothing to
        // show now. A full scan also drops a folder outside its roots, for
        // example in a removed Destination. A note that the scan found, or
        // that cannot be read, has the other versions of this scan only.
        inner.notes.retain(|id, (path, _)| {
            let keep = found.contains_key(id) || coverage.keeps_wait(scan, path);
            if !keep {
                changed.insert(id.clone());
            }
            keep
        });
        inner
            .seen
            .retain(|id, (path, _)| !found.contains_key(id) && coverage.keeps_wait(scan, path));
        inner.seen.extend(seen);
        for (id, (path, status)) in found {
            let before = inner
                .notes
                .get(&id)
                .map(|(_, s)| s.clone())
                .unwrap_or_default();
            if before != status {
                changed.insert(id.clone());
            }
            if status == NoteStatus::default() {
                inner.notes.remove(&id);
            } else {
                inner.notes.insert(id, (path, status));
            }
        }

        let mut changed: Vec<String> = changed.into_iter().collect();
        changed.sort();
        changed
    }
}
