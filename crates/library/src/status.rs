//! What the latest scans found about each note and each Destination
//! (KTD8, KTD10): the parts that wait for the sync, the other versions of
//! the note, and the Destinations that are unavailable. The note page and
//! Home read it, so no command scans a folder. It stays in memory only,
//! and each scan builds it again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use session_core::SessionRecord;

use crate::reconcile::Coverage;
use crate::scan::{DocumentState, LibraryScan, NoteFolder, RootState, Waiting};

/// What the latest scan found about one note folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteStatus {
    /// The parts that the sync has not delivered yet (R6).
    pub waiting: Vec<Waiting>,
    /// Conflict copies of the documents, then other folders with the same
    /// note id (R15).
    pub other_versions: Vec<PathBuf>,
    /// `session.md` exists but cannot be read (KTD5): the note waits as if
    /// its Destination were unavailable.
    pub unreadable: bool,
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

#[derive(Debug, Default)]
struct Inner {
    /// Each note with something to show, and the folder that the scan read.
    notes: HashMap<String, (PathBuf, NoteStatus)>,
    unavailable: HashSet<PathBuf>,
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

    /// Whether the note with the home folder `home` cannot be read now: its
    /// Destination is unavailable, or its `session.md` cannot be read (R16).
    pub fn storage_unavailable(&self, id: &str, home: Option<&Path>) -> bool {
        let inner = self.lock();
        let root_gone = home
            .and_then(Path::parent)
            .is_some_and(|root| inner.unavailable.contains(root));
        root_gone || inner.notes.get(id).is_some_and(|(_, s)| s.unreadable)
    }

    /// Apply a scan. `records` are the rows after the reconcile of the
    /// scan. `moving` holds the notes that the Move has not finished
    /// (KTD10): such a note can be in the old and the new folder, and that
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
            let root = record
                .home_path
                .as_deref()
                .map(Path::new)
                .and_then(Path::parent);
            if root.is_some_and(|root| roots_changed.contains(root)) {
                changed.insert(record.id.clone());
            }
        }

        let mut found: HashMap<String, (PathBuf, NoteStatus)> = HashMap::new();
        for note in &scan.notes {
            let duplicates: &[PathBuf] = if moving.contains(&note.doc.session_id) {
                &[]
            } else {
                &note.duplicate_folders
            };
            let status = NoteStatus {
                waiting: waiting(note, hand_edits.contains(&note.doc.session_id)),
                other_versions: note
                    .conflict_copies
                    .iter()
                    .chain(duplicates)
                    .cloned()
                    .collect(),
                unreadable: false,
            };
            found.insert(note.doc.session_id.clone(), (note.path.clone(), status));
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
        // show now.
        inner.notes.retain(|id, (path, _)| {
            let keep = found.contains_key(id) || !coverage.read(scan, path);
            if !keep {
                changed.insert(id.clone());
            }
            keep
        });
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
