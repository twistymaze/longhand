//! What the latest scans found about each note and each Destination
//! (KTD8, KTD10): the parts that wait for the sync, the other versions of
//! the note, and the Destinations that are unavailable. The note page and
//! Home read it, so no command scans a folder. It stays in memory only,
//! and each scan builds it again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use session_core::note_folder::Layout;
use session_core::{SessionRecord, SessionState};

use crate::reconcile::Coverage;
use crate::scan::{
    DocumentState, FileProblem, LibraryScan, NoteFolder, ProblemKind, RemoteState, RootState,
    Waiting,
};

/// How long the scans must find another version of a note before the
/// status reports it (R15). A move of the note can show two folders for a
/// short time: a scan that runs during a rename or a type move, or a sync
/// that delivers the new folder before it removes the old one. A sync can
/// take up to 2 minutes for that, as for [`crate::reconcile::DELETE_WAIT`].
/// A sync service can also remove a conflict copy that it made for a short
/// time, so a conflict copy gets the same wait.
pub const OTHER_VERSION_WAIT: Duration = Duration::from_secs(120);

/// How long a file of a note folder must give no note before the status
/// reports it (R6). A shorter failure is part of a sync: a partial file, or
/// a `session.md` that arrived before its `longhand.json`. A file that a
/// newer Longhand wrote is reported at once, because a sync does not
/// change it.
pub const PROBLEM_WAIT: Duration = crate::reconcile::HAND_EDIT_WAIT;

/// A note folder whose files give no note, after [`PROBLEM_WAIT`] (R6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedProblem {
    pub path: PathBuf,
    /// The note of the folder, if any ([`crate::scan::ProblemFolder::known_id`]).
    pub known_id: Option<String>,
    pub problem: FileProblem,
}

/// A note folder whose files give no note, reported or not yet.
#[derive(Debug, Clone)]
struct Problem {
    known_id: Option<String>,
    problem: FileProblem,
    /// When a scan first found a problem of this kind in this file.
    since: Instant,
    reported: bool,
}

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
    /// A file of the note folder gives no note (R6): it does not parse, a
    /// newer Longhand wrote it, or `longhand.json` is not there. The writes
    /// that the file feeds wait at once. Home reports the problem only
    /// after [`PROBLEM_WAIT`] ([`LibraryStatus::problems`]).
    pub problem: Option<FileProblem>,
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
    /// The note folders whose files give no note, by folder.
    problems: HashMap<PathBuf, Problem>,
    /// The notes in the old layout, with their folders (KTD6).
    old_layout: HashMap<String, PathBuf>,
    /// A number that changes each time the reported problems or the notes
    /// in the old layout change.
    notices_revision: u64,
}

impl Inner {
    /// The reported problems, sorted by folder.
    fn reported_problems(&self) -> Vec<ReportedProblem> {
        let mut out: Vec<ReportedProblem> = self
            .problems
            .iter()
            .filter(|(_, p)| p.reported)
            .map(|(path, p)| ReportedProblem {
                path: path.clone(),
                known_id: p.known_id.clone(),
                problem: p.problem.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// The notes in the old layout, sorted by folder.
    fn old_layout_notes(&self) -> Vec<(String, PathBuf)> {
        let mut out: Vec<(String, PathBuf)> = self
            .old_layout
            .iter()
            .map(|(id, path)| (id.clone(), path.clone()))
            .collect();
        out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        out
    }
}

/// Record the notes of `scan` in the old layout (KTD6). A targeted scan
/// keeps an old note whose folder it did not read.
fn apply_old_layout(inner: &mut Inner, scan: &LibraryScan, coverage: &Coverage) {
    let before = inner.old_layout_notes();
    let found: HashSet<&str> = scan
        .notes
        .iter()
        .map(|note| note.doc.session_id.as_str())
        .collect();
    inner
        .old_layout
        .retain(|id, path| !found.contains(id.as_str()) && coverage.keeps_wait(scan, path));
    for note in &scan.notes {
        if note.layout == Layout::Old {
            inner
                .old_layout
                .insert(note.doc.session_id.clone(), note.path.clone());
        }
    }
    if inner.old_layout_notes() != before {
        inner.notices_revision += 1;
    }
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
        let inner = self.lock();
        let versions = inner
            .seen
            .values()
            .flat_map(|(_, versions)| versions.values())
            .filter(|s| !s.reported)
            .map(|s| s.since + OTHER_VERSION_WAIT);
        let problems = inner
            .problems
            .values()
            .filter(|p| !p.reported)
            .map(|p| p.since + PROBLEM_WAIT);
        versions.chain(problems).min()
    }

    /// The note folders whose files have given no note for
    /// [`PROBLEM_WAIT`], sorted by folder (R6). Home shows them in one
    /// notice.
    pub fn problems(&self) -> Vec<ReportedProblem> {
        self.lock().reported_problems()
    }

    /// A number that changes each time [`LibraryStatus::problems`] or
    /// [`LibraryStatus::old_layout_notes`] changes.
    pub fn notices_revision(&self) -> u64 {
        self.lock().notices_revision
    }

    /// The notes in the old layout that the latest scans found, each with
    /// its folder, sorted by folder (R11, KTD6). Empty when the storage
    /// folder has no old note: Home then shows no conversion control.
    pub fn old_layout_notes(&self) -> Vec<(String, PathBuf)> {
        self.lock().old_layout_notes()
    }

    /// Forget the notes `ids` as old notes: a conversion on this computer
    /// changed them to the new layout. The next scan reads their folders
    /// again.
    pub fn forget_old_layout(&self, ids: &[String]) {
        let mut inner = self.lock();
        let before = inner.old_layout.len();
        inner.old_layout.retain(|id, _| !ids.contains(id));
        if inner.old_layout.len() != before {
            inner.notices_revision += 1;
        }
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
                problem: None,
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
        apply_problems(&mut inner, scan, coverage, &mut found, now);
        apply_old_layout(&mut inner, scan, coverage);

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

/// Record the folders of `scan` whose files give no note, at `now` (R6).
/// A folder keeps the time of its first scan while the kind and the file
/// of its problem stay. A folder that the scan read and that gives a note,
/// or no problem, has no problem now. Each note with a problem that the
/// scan found gets the problem in `found`, so its writes wait at once.
fn apply_problems(
    inner: &mut Inner,
    scan: &LibraryScan,
    coverage: &Coverage,
    found: &mut HashMap<String, (PathBuf, NoteStatus)>,
    now: Instant,
) {
    let before = inner.reported_problems();
    let mut problems: HashMap<PathBuf, Problem> = HashMap::new();
    for folder in scan.invalid.iter().chain(&scan.waiting_for_record) {
        let Some(problem) = &folder.problem else {
            continue;
        };
        let since = inner
            .problems
            .get(&folder.path)
            .filter(|p| p.problem.kind == problem.kind && p.problem.file == problem.file)
            .map_or(now, |p| p.since);
        let reported = problem.kind == ProblemKind::NewerRecord
            || now.saturating_duration_since(since) >= PROBLEM_WAIT;
        problems.insert(
            folder.path.clone(),
            Problem {
                known_id: folder.known_id.clone(),
                problem: problem.clone(),
                since,
                reported,
            },
        );
        if let Some(id) = &folder.known_id
            && !found.contains_key(id)
        {
            let status = NoteStatus {
                problem: Some(problem.clone()),
                ..NoteStatus::default()
            };
            found.insert(id.clone(), (folder.path.clone(), status));
        }
    }
    // A targeted scan keeps the problem of a folder that it did not read.
    for (path, problem) in std::mem::take(&mut inner.problems) {
        if !problems.contains_key(&path) && coverage.keeps_wait(scan, &path) {
            problems.insert(path, problem);
        }
    }
    inner.problems = problems;
    if inner.reported_problems() != before {
        inner.notices_revision += 1;
    }
}
