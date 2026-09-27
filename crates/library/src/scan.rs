//! The library scan (KTD5): the note folders in each retained Destination,
//! what each one still waits for, its conflict copies, and whether each
//! Destination is available.
//!
//! The scan lists the top-level folders of a Destination and reads only
//! the documents of each one. It never opens audio: it only asks whether
//! each segment's audio file exists, so an online-only audio file is not
//! downloaded (R12). Each read has a timeout, so a file that does not
//! download does not stop the scan.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use session_core::SessionRecord;
use session_core::folder::{SessionDoc, fingerprint};

use crate::conflict;

/// The hidden file that tells an available Destination from one whose sync
/// client removed its files (KTD5, KD7).
pub const MARKER: &str = ".longhand-library";

/// The default time limit of one file read.
pub const READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    pub read_timeout: Duration,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            read_timeout: READ_TIMEOUT,
        }
    }
}

/// Write the library marker in `root`. Returns false, and logs the error,
/// when the marker cannot be written.
pub fn write_marker(root: &Path) -> bool {
    match session_save::home_write::write_atomic(root, MARKER, b"") {
        Ok(()) => true,
        Err(e) => {
            log::warn!("library: cannot write {MARKER} in {}: {e}", root.display());
            false
        }
    }
}

/// What the store knows before the scan: the home folder of each row.
#[derive(Debug, Clone, Default)]
pub struct Known {
    homes: HashMap<PathBuf, String>,
    roots: HashSet<PathBuf>,
}

impl Known {
    /// Every row that has a home folder.
    pub fn from_records(records: &[SessionRecord]) -> Self {
        let mut known = Self::default();
        for record in records {
            if let Some(home) = &record.home_path {
                known.add(&record.id, Path::new(home));
            }
        }
        known
    }

    /// Record that the row `id` has its home folder at `home`.
    pub fn add(&mut self, id: &str, home: &Path) {
        self.homes.insert(home.to_path_buf(), id.to_string());
        if let Some(root) = home.parent() {
            self.roots.insert(root.to_path_buf());
        }
    }

    /// The row whose home folder is at `path`.
    pub fn id_at(&self, path: &Path) -> Option<&str> {
        self.homes.get(path).map(String::as_str)
    }

    /// The home folder of the row `id`.
    pub fn home_of(&self, id: &str) -> Option<&Path> {
        self.homes
            .iter()
            .find(|(_, known)| known.as_str() == id)
            .map(|(home, _)| home.as_path())
    }

    /// Whether a row has its home folder in `root`.
    pub fn has_rows_in(&self, root: &Path) -> bool {
        self.roots.contains(root)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// The Destination does not exist.
    Missing,
    NotAFolder,
    /// The Destination has no marker, no note folders, and rows in the
    /// store: a signed-out sync client looks like this.
    NoMarker,
    /// The Destination cannot be listed.
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootState {
    /// Every note folder that is gone from here is a delete (KTD6).
    Available,
    /// The rows in this Destination stay as they are (KD7).
    Unavailable(Unavailable),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootScan {
    pub root: PathBuf,
    pub state: RootState,
    /// This scan wrote the marker.
    pub marker_written: bool,
}

/// A part of a note that the sync has not delivered yet (R6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waiting {
    /// `transcript.md` is missing, or is from another version of the note.
    Transcript,
    /// `summary.md` is missing, or is from another version of the note.
    Summary,
    Notes,
    /// The audio of a segment, as `session.md` names it.
    Audio(String),
}

/// One note: the real folder of one note id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFolder {
    pub root: PathBuf,
    pub path: PathBuf,
    pub doc: SessionDoc,
    pub waiting: Vec<Waiting>,
    /// The fingerprint of `transcript.md`, when the scan read it.
    pub transcript_fingerprint: Option<String>,
    /// The fingerprint of `summary.md`, when the scan read it.
    pub summary_fingerprint: Option<String>,
    /// The text of `notes.md`, when the scan read it.
    pub notes: Option<String>,
    /// Conflict copies of the four documents in this folder (R15).
    pub conflict_copies: Vec<PathBuf>,
    /// Other folders with the same note id. Their content is not read.
    pub duplicate_folders: Vec<PathBuf>,
}

/// A folder whose `session.md` exists but gives no note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemFolder {
    pub root: PathBuf,
    pub path: PathBuf,
    /// The row whose home folder this is, if any.
    pub known_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LibraryScan {
    /// One entry for each Destination, in the order given.
    pub roots: Vec<RootScan>,
    /// The notes in the available Destinations, one for each note id.
    pub notes: Vec<NoteFolder>,
    /// Folders whose `session.md` cannot be read: an I/O error or a read
    /// that timed out. Such a note is not missing (KTD5).
    pub unreadable: Vec<ProblemFolder>,
    /// Folders whose `session.md` is not valid. They are not notes yet.
    pub invalid: Vec<ProblemFolder>,
}

impl LibraryScan {
    pub fn is_available(&self, root: &Path) -> bool {
        self.roots
            .iter()
            .any(|r| r.root == root && r.state == RootState::Available)
    }
}

/// Scan every Destination in `roots`.
pub fn scan(roots: &[PathBuf], known: &Known, options: &ScanOptions) -> LibraryScan {
    let mut result = LibraryScan::default();
    let mut found = Vec::new();
    for root in roots {
        let (root_scan, notes) = scan_root(root, known, options, &mut result);
        result.roots.push(root_scan);
        found.extend(notes);
    }
    result.notes = choose_real_folders(found, known);
    result
}

/// A targeted scan (KTD7): only the note folders `folders` in `root`, and
/// the home folder of each note id that one of them holds, so that a
/// duplicate folder does not take the place of the real one. Returns the
/// scan and the folders that it read. `None` when `root` is not a folder
/// with its marker: only a full scan can tell the state of such a root.
pub fn scan_folders(
    root: &Path,
    folders: &HashSet<PathBuf>,
    known: &Known,
    options: &ScanOptions,
) -> Option<(LibraryScan, HashSet<PathBuf>)> {
    if !root.is_dir() || !root.join(MARKER).exists() {
        return None;
    }
    let mut read: HashSet<PathBuf> = HashSet::new();
    let mut reads = Reads::default();
    let mut sorted: Vec<&PathBuf> = folders.iter().collect();
    sorted.sort();
    for path in sorted {
        read.insert(path.clone());
        let hidden = path
            .file_name()
            .is_none_or(|name| is_hidden(&name.to_string_lossy()));
        if hidden || path.parent() != Some(root) || !path.is_dir() {
            continue;
        }
        reads.read(root, path, known, options);
    }
    let ids: Vec<String> = reads
        .notes
        .iter()
        .map(|note| note.doc.session_id.clone())
        .collect();
    for id in ids {
        let Some(home) = known.home_of(&id) else {
            continue;
        };
        if read.contains(home) || !home.is_dir() {
            continue;
        }
        read.insert(home.to_path_buf());
        let home_root = home.parent().unwrap_or(root).to_path_buf();
        reads.read(&home_root, home, known, options);
    }
    let result = LibraryScan {
        roots: vec![RootScan {
            root: root.to_path_buf(),
            state: RootState::Available,
            marker_written: false,
        }],
        notes: choose_real_folders(reads.notes, known),
        unreadable: reads.unreadable,
        invalid: reads.invalid,
    };
    Some((result, read))
}

fn scan_root(
    root: &Path,
    known: &Known,
    options: &ScanOptions,
    result: &mut LibraryScan,
) -> (RootScan, Vec<Found>) {
    let unavailable = |why| {
        (
            RootScan {
                root: root.to_path_buf(),
                state: RootState::Unavailable(why),
                marker_written: false,
            },
            Vec::new(),
        )
    };
    match std::fs::metadata(root) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return unavailable(Unavailable::Missing),
        Err(e) => return unavailable(Unavailable::Unreadable(e.to_string())),
        Ok(m) if !m.is_dir() => return unavailable(Unavailable::NotAFolder),
        Ok(_) => {}
    }
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => return unavailable(Unavailable::Unreadable(e.to_string())),
    };

    // An entry that cannot be listed or examined can be a note folder. The
    // list is then incomplete, so the root is unreadable and its missing
    // notes are not deletes (KTD5).
    let mut reads = Reads::default();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                log::warn!("library: cannot list {}: {e}", root.display());
                return unavailable(Unavailable::Unreadable(e.to_string()));
            }
        };
        // Staging, rename and temporary folders start with a dot.
        if is_hidden(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        match std::fs::metadata(&path) {
            Ok(m) if m.is_dir() => reads.read(root, &path, known, options),
            Ok(_) => {}
            // Removed since the list, or a link to nothing.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                log::warn!("library: cannot read {}: {e}", path.display());
                return unavailable(Unavailable::Unreadable(e.to_string()));
            }
        }
    }
    let Reads {
        notes,
        unreadable,
        invalid,
    } = reads;

    let mut marker_written = false;
    if !root.join(MARKER).exists() {
        if notes.is_empty() && known.has_rows_in(root) {
            return unavailable(Unavailable::NoMarker);
        }
        marker_written = write_marker(root);
    }
    result.unreadable.extend(unreadable);
    result.invalid.extend(invalid);
    (
        RootScan {
            root: root.to_path_buf(),
            state: RootState::Available,
            marker_written,
        },
        notes,
    )
}

/// The folders that one scan read, by kind.
#[derive(Default)]
struct Reads {
    notes: Vec<Found>,
    unreadable: Vec<ProblemFolder>,
    invalid: Vec<ProblemFolder>,
}

impl Reads {
    /// Read the folder at `path` in `root`.
    fn read(&mut self, root: &Path, path: &Path, known: &Known, options: &ScanOptions) {
        let problem = |reason: String| ProblemFolder {
            root: root.to_path_buf(),
            known_id: known.id_at(path).map(str::to_string),
            path: path.to_path_buf(),
            reason,
        };
        match read_folder(path, options) {
            FolderRead::NotANote => {}
            FolderRead::Note(note) => self.notes.push(Found {
                root: root.to_path_buf(),
                ..*note
            }),
            FolderRead::Unreadable(reason) => {
                log::warn!("library: cannot read {}: {reason}", path.display());
                self.unreadable.push(problem(reason));
            }
            FolderRead::Invalid(reason) => self.invalid.push(problem(reason)),
        }
    }
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// A note folder before the choice among folders with one note id.
struct Found {
    root: PathBuf,
    path: PathBuf,
    name: String,
    created: Option<SystemTime>,
    doc: SessionDoc,
    waiting: Vec<Waiting>,
    transcript_fingerprint: Option<String>,
    summary_fingerprint: Option<String>,
    notes: Option<String>,
    conflict_copies: Vec<PathBuf>,
}

enum FolderRead {
    /// No `session.md`.
    NotANote,
    Note(Box<Found>),
    Unreadable(String),
    Invalid(String),
}

fn read_folder(dir: &Path, options: &ScanOptions) -> FolderRead {
    let doc = match read_timed(dir.join("session.md"), options.read_timeout) {
        Read::Missing => return FolderRead::NotANote,
        Read::Failed(reason) => return FolderRead::Unreadable(reason),
        Read::Bytes(bytes) => match String::from_utf8(bytes) {
            Err(e) => return FolderRead::Invalid(e.to_string()),
            Ok(text) => match SessionDoc::parse(&text) {
                Err(e) => return FolderRead::Invalid(e.to_string()),
                Ok(doc) => doc,
            },
        },
    };

    let mut waiting = Vec::new();
    let transcript = document_state(
        dir,
        "transcript.md",
        doc.transcript_fingerprint.as_deref(),
        options,
    );
    if !transcript.current {
        waiting.push(Waiting::Transcript);
    }
    let summary = document_state(
        dir,
        "summary.md",
        doc.summary_fingerprint.as_deref(),
        options,
    );
    if !summary.current {
        waiting.push(Waiting::Summary);
    }
    let notes = read_notes(dir, options.read_timeout);
    if notes.is_none() {
        waiting.push(Waiting::Notes);
    }
    // Only the metadata: the audio is never opened (R12).
    for segment in &doc.segments {
        if !exists(&dir.join(&segment.audio)) {
            waiting.push(Waiting::Audio(segment.audio.clone()));
        }
    }

    let mut conflict_copies: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !is_hidden(&name) && conflict::document_copy(&name).is_some()
        })
        .map(|entry| entry.path())
        .collect();
    conflict_copies.sort();

    FolderRead::Note(Box::new(Found {
        root: PathBuf::new(),
        path: dir.to_path_buf(),
        name: dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        created: std::fs::metadata(dir)
            .ok()
            .and_then(|m| m.created().or_else(|_| m.modified()).ok()),
        doc,
        waiting,
        transcript_fingerprint: transcript.fingerprint,
        summary_fingerprint: summary.fingerprint,
        notes,
        conflict_copies,
    }))
}

/// The text of `notes.md` in `dir`. `None` when the file is missing, cannot
/// be read in `timeout`, or is not UTF-8: the notes then wait for the sync.
pub(crate) fn read_notes(dir: &Path, timeout: Duration) -> Option<String> {
    match read_timed(dir.join("notes.md"), timeout) {
        Read::Bytes(bytes) => String::from_utf8(bytes).ok(),
        Read::Missing | Read::Failed(_) => None,
    }
}

struct DocumentState {
    /// The file exists and, when `session.md` records a fingerprint, has it.
    current: bool,
    fingerprint: Option<String>,
}

fn document_state(
    dir: &Path,
    name: &str,
    expected: Option<&str>,
    options: &ScanOptions,
) -> DocumentState {
    match read_timed(dir.join(name), options.read_timeout) {
        Read::Bytes(bytes) => {
            let actual = fingerprint(&bytes);
            DocumentState {
                current: expected.is_none_or(|expected| expected == actual),
                fingerprint: Some(actual),
            }
        }
        Read::Missing | Read::Failed(_) => DocumentState {
            current: false,
            fingerprint: None,
        },
    }
}

/// Whether `path` exists. This reads metadata only and opens no file.
fn exists(path: &Path) -> bool {
    path.try_exists().unwrap_or(false)
}

/// The result of [`read_timed`].
pub enum Read {
    Missing,
    Bytes(Vec<u8>),
    /// The read failed or did not finish in time.
    Failed(String),
}

/// Read `path` on a separate thread, and stop waiting after `timeout`. A
/// read that does not finish keeps its thread until the read returns.
pub fn read_timed(path: PathBuf, timeout: Duration) -> Read {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("library-read".into())
        .spawn(move || {
            let _ = tx.send(std::fs::read(&path));
        });
    if let Err(e) = spawned {
        return Read::Failed(format!("cannot start the read: {e}"));
    }
    match rx.recv_timeout(timeout) {
        Ok(Ok(bytes)) => Read::Bytes(bytes),
        Ok(Err(e)) if e.kind() == io::ErrorKind::NotFound => Read::Missing,
        Ok(Err(e)) => Read::Failed(e.to_string()),
        Err(_) => Read::Failed(format!("the read did not finish in {timeout:?}")),
    }
}

/// Keep one folder for each note id (KTD5): the folder at the row's home
/// folder, then a folder whose name is not a conflict copy of another's
/// name, then the older folder. The others are duplicate folders.
fn choose_real_folders(found: Vec<Found>, known: &Known) -> Vec<NoteFolder> {
    let mut by_id: BTreeMap<String, Vec<Found>> = BTreeMap::new();
    for f in found {
        by_id.entry(f.doc.session_id.clone()).or_default().push(f);
    }
    let mut notes: Vec<NoteFolder> = by_id
        .into_iter()
        .map(|(id, mut group)| {
            let real = real_index(&id, &group, known);
            let real = group.swap_remove(real);
            let mut duplicate_folders: Vec<PathBuf> = group.into_iter().map(|f| f.path).collect();
            duplicate_folders.sort();
            NoteFolder {
                root: real.root,
                path: real.path,
                doc: real.doc,
                waiting: real.waiting,
                transcript_fingerprint: real.transcript_fingerprint,
                summary_fingerprint: real.summary_fingerprint,
                notes: real.notes,
                conflict_copies: real.conflict_copies,
                duplicate_folders,
            }
        })
        .collect();
    notes.sort_by(|a, b| a.path.cmp(&b.path));
    notes
}

fn real_index(id: &str, group: &[Found], known: &Known) -> usize {
    if let Some(home) = group.iter().position(|f| known.id_at(&f.path) == Some(id)) {
        return home;
    }
    let is_copy = |f: &Found| {
        group
            .iter()
            .any(|other| conflict::is_copy_name(&f.name, &other.name))
    };
    let mut choices: Vec<usize> = (0..group.len()).filter(|&i| !is_copy(&group[i])).collect();
    if choices.is_empty() {
        choices = (0..group.len()).collect();
    }
    choices
        .into_iter()
        .min_by(|&a, &b| {
            let (a, b) = (&group[a], &group[b]);
            // A folder with no known time sorts after one with a time.
            (a.created.is_none(), a.created, &a.path).cmp(&(
                b.created.is_none(),
                b.created,
                &b.path,
            ))
        })
        .expect("a group has at least one folder")
}
