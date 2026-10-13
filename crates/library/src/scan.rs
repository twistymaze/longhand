//! The library scan (KTD5): the note folders in each retained Destination,
//! what each one still waits for, its conflict copies, and whether each
//! Destination is available.
//!
//! The scan lists the top-level folders of a Destination and reads only
//! the documents of each one. A folder that holds `session.md` or
//! `longhand.json` is a note folder. The note's identity and records come
//! from `longhand.json`, and its title and tags from `session.md`. A
//! `session.md` that holds `session_id` is a note in the old layout, whose
//! records are in `session.md` (KTD6 of the safe agent edits plan). A
//! new-layout `session.md` with no `longhand.json` waits for a sync, and is
//! not a note yet. A top-level folder with neither file is a type folder
//! when it holds note folders: the scan also reads the folders in it, and
//! gives their notes that type (KTD4 of the note types plan). It also
//! reads the folders in a top-level folder whose files cannot be read or
//! are not valid, so that no note in it is forgotten.
//! Hidden folders and anything deeper are not read. It never opens audio: it only asks whether
//! each segment's audio file exists, so an online-only audio file is not
//! downloaded (R12). It also asks whether each segment's removal marker
//! exists, and does not open it. Each read has a timeout, so a file that
//! does not download does not stop the scan.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use chrono::{Local, TimeDelta};
use session_core::folder::{self, ResultFlag, SessionDoc, fingerprint};
use session_core::note_folder::{Layout, NoteFiles, NoteFilesError, SESSION_FILE};
use session_core::note_record::NOTE_RECORD_FILE;
use session_core::{SessionRecord, Title};
use session_save::in_use_marker;

use crate::conflict;

/// The hidden folder in a Destination root that holds Longhand's own files.
/// The scan skips it, as it skips each hidden name.
pub const LONGHAND_FOLDER: &str = ".longhand";

/// The file in [`LONGHAND_FOLDER`] that tells an available Destination from
/// one whose sync client removed its files (KTD5, KD7).
pub const MARKER: &str = "library";

/// The path of the library marker in `root`.
pub fn marker(root: &Path) -> PathBuf {
    root.join(LONGHAND_FOLDER).join(MARKER)
}

/// The position of a home folder in its storage folder (KTD4 of the note
/// types plan): the storage folder, and the note type that the type
/// folder gives. A home folder directly in the storage folder is in General.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub root: PathBuf,
    pub note_type: Option<String>,
}

/// The place of the home folder `home` in one of `roots`, at depth 0 or 1.
/// `None` when `home` is in none of them. This reads no file.
pub fn place_in(home: &Path, roots: &[PathBuf]) -> Option<Place> {
    roots.iter().find_map(|root| {
        folder::note_type_in(root, home).map(|note_type| Place {
            root: root.clone(),
            note_type,
        })
    })
}

/// The place of the home folder `home` from the library marker: the parent
/// folder when it has the marker, else the parent of a type folder when
/// that folder has the marker. `None` when neither has it: a removed disk,
/// a lost share or a signed-out sync client. A step that has no list of
/// storage folders uses this, so it never takes a type folder for a
/// storage folder.
pub fn storage_place(home: &Path) -> Option<Place> {
    let parent = home.parent()?;
    if marker(parent).exists() {
        return Some(Place {
            root: parent.to_path_buf(),
            note_type: None,
        });
    }
    let root = parent.parent()?;
    let note_type = folder::note_type_in(root, home)??;
    marker(root).exists().then(|| Place {
        root: root.to_path_buf(),
        note_type: Some(note_type),
    })
}

/// The default time limit of one file read.
pub const READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    pub read_timeout: Duration,
    /// The run IDs of this computer's in-use markers: this run and the
    /// earlier runs on this computer. Such a marker shows this computer's
    /// own work, so it does not make a note in use on another computer
    /// (KTD5). An empty list counts every marker.
    pub own_runs: Vec<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            read_timeout: READ_TIMEOUT,
            own_runs: Vec::new(),
        }
    }
}

/// Write the library marker in `root`. Returns false, and logs the error,
/// when the marker cannot be written.
pub fn write_marker(root: &Path) -> bool {
    let folder = root.join(LONGHAND_FOLDER);
    let written = std::fs::create_dir_all(&folder)
        .and_then(|()| session_save::home_write::write_atomic(&folder, MARKER, b""));
    match written {
        Ok(()) => true,
        Err(e) => {
            log::warn!(
                "library: cannot write the marker {}: {e}",
                marker(root).display()
            );
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

    /// Record that the row `id` has its home folder at `home`. The home
    /// folder can be directly in its storage folder or in a type folder
    /// (KTD4), so both folders above it count as a storage folder with a
    /// row.
    pub fn add(&mut self, id: &str, home: &Path) {
        self.homes.insert(home.to_path_buf(), id.to_string());
        let Some(parent) = home.parent() else {
            return;
        };
        self.roots.insert(parent.to_path_buf());
        if let Some(root) = parent.parent()
            && folder::note_type_in(root, home).is_some()
        {
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

/// What the scan found for `transcript.md` or `summary.md`, compared with
/// the fingerprint that `session.md` records (R6, R8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentState {
    /// The file has the recorded fingerprint, or `session.md` records none.
    Matches,
    /// The file has another fingerprint.
    Differs,
    Missing,
    /// The file exists, but the read failed or did not finish in time.
    Unreadable,
}

/// What a note folder shows about work on the note on another computer
/// (KTD5). The in-use markers decide first, then the `result` flag of
/// `session.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteState {
    /// Another computer records into or processes the note: a fresh in-use
    /// marker of another run is in the folder, whatever the flag says
    /// (R12, R20, R21). A note whose flag says "recording not saved" and
    /// whose `session.md` is newer than [`in_use_marker::FRESH_FOR`] is also in use: its
    /// marker can still be on its way.
    InUse,
    /// The flag says "recording not saved", no in-use marker is fresh, and
    /// `session.md` is older than [`in_use_marker::FRESH_FOR`]: the computer that records
    /// into the note stopped before it saved the recording (R23).
    NotFinished,
}

/// One note: the real folder of one note id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFolder {
    pub root: PathBuf,
    pub path: PathBuf,
    /// The type folder that holds the note folder (KTD1). `None` for a
    /// note folder directly in `root`.
    pub note_type: Option<String>,
    /// The records and the user fields of the note ([`NoteFiles::doc`]).
    pub doc: SessionDoc,
    pub layout: Layout,
    /// The title that Longhand last wrote into `session.md` (KTD5). `None`
    /// in the old layout.
    pub written_title: Option<Title>,
    pub waiting: Vec<Waiting>,
    /// The state of `transcript.md`. A state other than
    /// [`DocumentState::Matches`] puts [`Waiting::Transcript`] in `waiting`.
    pub transcript: DocumentState,
    /// The state of `summary.md`, as for `transcript`.
    pub summary: DocumentState,
    /// The audio files, as `session.md` names them, of the segments whose
    /// removal marker is in the folder (KTD1 of the audio removal plan).
    /// Such a segment does not wait for its audio, also when the file is
    /// gone (R15).
    pub removed_audio: Vec<String>,
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
    /// What the folder shows about work on the note on another computer.
    /// The folder alone cannot tell this computer's own work from the work
    /// of an earlier run, so the reader also checks the row
    /// ([`crate::status::LibraryStatus::remote`]).
    pub remote: Option<RemoteState>,
}

/// A folder whose `session.md` or `longhand.json` exists but gives no
/// note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemFolder {
    pub root: PathBuf,
    pub path: PathBuf,
    /// The note of the folder, if any: the id in its `longhand.json`, else
    /// the row whose home folder this is.
    pub known_id: Option<String>,
    pub reason: String,
    /// The file that gives no note, for a folder that is not valid or that
    /// waits for its `longhand.json` (R6). `None` for a read that failed.
    pub problem: Option<FileProblem>,
}

/// What the user can do about a file that gives no note (R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProblemKind {
    /// `session.md` does not parse. The user can correct the file.
    UserFile,
    /// `longhand.json` does not parse or is not in the folder. The sync can
    /// still deliver it, or another computer has an older Longhand.
    Record,
    /// A newer Longhand wrote `longhand.json`. This computer must update
    /// Longhand to change the note.
    NewerRecord,
}

/// The file of a note folder that gives no note, and the line in it when
/// the parse tells the line (R6).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileProblem {
    pub kind: ProblemKind,
    pub file: &'static str,
    pub line: Option<usize>,
}

impl FileProblem {
    /// The problem of the file `file` that does not parse at `line`.
    fn parse_error(file: &'static str, line: Option<usize>) -> Self {
        let kind = if file == NOTE_RECORD_FILE {
            ProblemKind::Record
        } else {
            ProblemKind::UserFile
        };
        Self { kind, file, line }
    }

    /// A new-layout `session.md` with no `longhand.json` (KTD6).
    fn waiting_for_record() -> Self {
        Self {
            kind: ProblemKind::Record,
            file: NOTE_RECORD_FILE,
            line: None,
        }
    }
}

/// A note folder whose files are not valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidNote {
    pub problem: FileProblem,
    pub reason: String,
}

impl std::fmt::Display for InvalidNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl InvalidNote {
    /// The problem of a parse that gave `error`.
    fn of(error: NoteFilesError) -> Self {
        let reason = error.to_string();
        let problem = match error {
            NoteFilesError::Newer { .. } => FileProblem {
                kind: ProblemKind::NewerRecord,
                file: NOTE_RECORD_FILE,
                line: None,
            },
            NoteFilesError::Invalid(error) => match error {
                folder::FolderError::MissingFrontmatter { file }
                | folder::FolderError::Yaml { file, .. } => FileProblem::parse_error(file, None),
                folder::FolderError::Line { file, line, .. }
                | folder::FolderError::At { file, line, .. } => {
                    FileProblem::parse_error(file, Some(line))
                }
            },
            NoteFilesError::NotANote | NoteFilesError::WaitingForRecord => {
                FileProblem::waiting_for_record()
            }
        };
        Self { problem, reason }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LibraryScan {
    /// One entry for each Destination, in the order given.
    pub roots: Vec<RootScan>,
    /// The notes in the available Destinations, one for each note id.
    pub notes: Vec<NoteFolder>,
    /// Folders whose `session.md` or `longhand.json` cannot be read: an I/O
    /// error or a read that timed out. Such a note is not missing (KTD5).
    pub unreadable: Vec<ProblemFolder>,
    /// Folders whose `session.md` or `longhand.json` is not valid. They
    /// are not notes yet.
    pub invalid: Vec<ProblemFolder>,
    /// Folders with a new-layout `session.md` and no `longhand.json`: the
    /// sync has not delivered the records yet (KTD6). They are not notes
    /// yet, and are not missing.
    pub waiting_for_record: Vec<ProblemFolder>,
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
    if !root.is_dir() || !marker(root).exists() {
        return None;
    }
    let mut read: HashSet<PathBuf> = HashSet::new();
    let mut reads = Reads::default();
    // A type folder sorts before the home folders in it. When the scan
    // reads the type folder, it reads those home folders too, and it does
    // not read them again: a second read gives the note as a duplicate
    // folder of itself.
    let mut sorted: Vec<&PathBuf> = folders.iter().collect();
    sorted.sort();
    for path in sorted {
        if !read.insert(path.clone()) {
            continue;
        }
        let hidden = path
            .file_name()
            .is_none_or(|name| is_hidden(&name.to_string_lossy()));
        if hidden || !path.is_dir() {
            continue;
        }
        match folder::note_type_in(root, path) {
            // A top-level folder: a home folder, or else a type folder,
            // whose notes are read (KTD4). A folder whose `session.md` gives
            // no note can still hold notes, so they are read too.
            Some(None) => {
                if reads.read(root, path, None, known, options) != FolderKind::Note
                    && let Ok(inside) = reads.read_type_folder(root, path, known, options)
                {
                    read.extend(inside);
                }
            }
            // A folder in a type folder. A folder in a home folder is not
            // a home folder.
            Some(Some(note_type)) => {
                let parent = path.parent().unwrap_or(root);
                if !is_note_folder(parent) {
                    reads.read(root, path, Some(&note_type), known, options);
                }
            }
            None => {}
        }
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
        // The home folder can be in another storage folder, or in a type
        // folder (KTD4).
        let place = place_in(home, &[root.to_path_buf()])
            .or_else(|| storage_place(home))
            .unwrap_or_else(|| Place {
                root: home.parent().unwrap_or(root).to_path_buf(),
                note_type: None,
            });
        read.insert(home.to_path_buf());
        reads.read(
            &place.root,
            home,
            place.note_type.as_deref(),
            known,
            options,
        );
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
        waiting_for_record: reads.waiting_for_record,
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
            Ok(m) if m.is_dir() => {
                if reads.read(root, &path, None, known, options) == FolderKind::Note {
                    continue;
                }
                // A top-level folder with no `session.md` can be a type
                // folder (KTD4). Its list must be complete too. A folder
                // whose `session.md` cannot be read or is not valid can
                // also hold notes: they are read, so a full scan does not
                // forget them (KTD12).
                match reads.read_type_folder(root, &path, known, options) {
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => {
                        log::warn!("library: cannot list {}: {e}", path.display());
                        return unavailable(Unavailable::Unreadable(e.to_string()));
                    }
                }
            }
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
        waiting_for_record,
    } = reads;

    let mut marker_written = false;
    if !marker(root).exists() {
        if notes.is_empty() && known.has_rows_in(root) {
            return unavailable(Unavailable::NoMarker);
        }
        marker_written = write_marker(root);
    }
    result.unreadable.extend(unreadable);
    result.invalid.extend(invalid);
    result.waiting_for_record.extend(waiting_for_record);
    (
        RootScan {
            root: root.to_path_buf(),
            state: RootState::Available,
            marker_written,
        },
        notes,
    )
}

/// What one folder is, from its `session.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderKind {
    /// No `session.md` and no `longhand.json`.
    NotANote,
    Note,
    /// A file cannot be read or is not valid, or the folder waits for its
    /// `longhand.json`.
    Problem,
}

/// The folders that one scan read, by kind.
#[derive(Default)]
struct Reads {
    notes: Vec<Found>,
    unreadable: Vec<ProblemFolder>,
    invalid: Vec<ProblemFolder>,
    waiting_for_record: Vec<ProblemFolder>,
}

impl Reads {
    /// Read the folder at `path` in `root`, which is in the type folder
    /// `note_type` or directly in `root`. Returns what the folder is.
    fn read(
        &mut self,
        root: &Path,
        path: &Path,
        note_type: Option<&str>,
        known: &Known,
        options: &ScanOptions,
    ) -> FolderKind {
        let problem = |reason: String, problem: Option<FileProblem>| ProblemFolder {
            root: root.to_path_buf(),
            known_id: record_id(path, options.read_timeout)
                .or_else(|| known.id_at(path).map(str::to_string)),
            path: path.to_path_buf(),
            reason,
            problem,
        };
        match read_folder(path, options) {
            FolderRead::NotANote => return FolderKind::NotANote,
            FolderRead::Note(note) => {
                self.notes.push(Found {
                    root: root.to_path_buf(),
                    note_type: note_type.map(str::to_string),
                    ..*note
                });
                return FolderKind::Note;
            }
            FolderRead::Unreadable(reason) => {
                log::warn!("library: cannot read {}: {reason}", path.display());
                self.unreadable.push(problem(reason, None));
            }
            FolderRead::Invalid(invalid) => self
                .invalid
                .push(problem(invalid.reason, Some(invalid.problem))),
            FolderRead::WaitingForRecord => self.waiting_for_record.push(problem(
                NoteFilesError::WaitingForRecord.to_string(),
                Some(FileProblem::waiting_for_record()),
            )),
        }
        FolderKind::Problem
    }

    /// Read each folder in the top-level folder `folder` of `root`, which
    /// gives no note: a type folder (KTD4). Hidden names are skipped,
    /// and nothing deeper is read. Returns the folders that it read, or the
    /// error when the folder cannot be listed completely.
    fn read_type_folder(
        &mut self,
        root: &Path,
        folder: &Path,
        known: &Known,
        options: &ScanOptions,
    ) -> io::Result<Vec<PathBuf>> {
        let note_type = folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        let mut read = Vec::new();
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            if is_hidden(&entry.file_name().to_string_lossy()) {
                continue;
            }
            let path = entry.path();
            match std::fs::metadata(&path) {
                Ok(m) if m.is_dir() => {
                    self.read(root, &path, note_type.as_deref(), known, options);
                    read.push(path);
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(read)
    }
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// A note folder before the choice among folders with one note id.
struct Found {
    root: PathBuf,
    path: PathBuf,
    note_type: Option<String>,
    name: String,
    created: Option<SystemTime>,
    doc: SessionDoc,
    layout: Layout,
    written_title: Option<Title>,
    waiting: Vec<Waiting>,
    transcript: DocumentState,
    summary: DocumentState,
    removed_audio: Vec<String>,
    transcript_fingerprint: Option<String>,
    summary_fingerprint: Option<String>,
    notes: Option<String>,
    conflict_copies: Vec<PathBuf>,
    remote: Option<RemoteState>,
}

enum FolderRead {
    /// No `session.md` and no `longhand.json`.
    NotANote,
    Note(Box<Found>),
    Unreadable(String),
    Invalid(InvalidNote),
    /// A new-layout `session.md` and no `longhand.json` (KTD6).
    WaitingForRecord,
}

/// What the two files that define a note folder give (KTD1, KTD6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteRead {
    Note(Box<NoteFiles>),
    /// No `session.md` and no `longhand.json`.
    NotANote,
    /// A new-layout `session.md` and no `longhand.json`.
    WaitingForRecord,
    /// A file cannot be read, or the read did not finish in time.
    Unreadable(String),
    /// A file is not valid, or a newer build wrote `longhand.json`.
    Invalid(InvalidNote),
}

/// Read `session.md` and `longhand.json` in the note folder `dir`. Each
/// read stops after `timeout`.
pub fn read_note(dir: &Path, timeout: Duration) -> NoteRead {
    let text = |name: &'static str| match read_timed(dir.join(name), timeout) {
        Read::Missing => Ok(None),
        Read::Failed(reason) => Err(NoteRead::Unreadable(reason)),
        Read::Bytes(bytes) => String::from_utf8(bytes).map(Some).map_err(|e| {
            NoteRead::Invalid(InvalidNote {
                problem: FileProblem::parse_error(name, None),
                reason: format!("{name}: {e}"),
            })
        }),
    };
    let session_md = match text(SESSION_FILE) {
        Ok(text) => text,
        Err(read) => return read,
    };
    let record = match text(NOTE_RECORD_FILE) {
        Ok(text) => text,
        Err(read) => return read,
    };
    match NoteFiles::parse(session_md.as_deref(), record.as_deref()) {
        Ok(files) => NoteRead::Note(Box::new(files)),
        Err(NoteFilesError::NotANote) => NoteRead::NotANote,
        Err(NoteFilesError::WaitingForRecord) => NoteRead::WaitingForRecord,
        Err(e) => NoteRead::Invalid(InvalidNote::of(e)),
    }
}

/// Whether the folder `dir` holds `session.md` or `longhand.json`: it is a
/// note folder, not a type folder. This reads metadata only.
pub fn is_note_folder(dir: &Path) -> bool {
    exists(&dir.join(SESSION_FILE)) || exists(&dir.join(NOTE_RECORD_FILE))
}

/// The note id in the `longhand.json` of `dir`, when the file parses. A
/// folder whose `session.md` does not parse still gives its id. A file
/// that a newer Longhand wrote gives its `session_id` when it has one as
/// text, so that the row of the note stays.
fn record_id(dir: &Path, timeout: Duration) -> Option<String> {
    let Read::Bytes(bytes) = read_timed(dir.join(NOTE_RECORD_FILE), timeout) else {
        return None;
    };
    let text = std::str::from_utf8(&bytes).ok()?;
    match session_core::note_record::NoteRecord::parse(text).ok()? {
        session_core::note_record::NoteRecordRead::Current(record) => Some(record.session_id),
        session_core::note_record::NoteRecordRead::Newer { .. } => {
            let value: serde_json::Value = serde_json::from_str(text).ok()?;
            value.get("session_id")?.as_str().map(str::to_string)
        }
    }
}

fn read_folder(dir: &Path, options: &ScanOptions) -> FolderRead {
    let files = match read_note(dir, options.read_timeout) {
        NoteRead::Note(files) => *files,
        NoteRead::NotANote => return FolderRead::NotANote,
        NoteRead::WaitingForRecord => return FolderRead::WaitingForRecord,
        NoteRead::Unreadable(reason) => return FolderRead::Unreadable(reason),
        NoteRead::Invalid(reason) => return FolderRead::Invalid(reason),
    };
    let layout = files.layout;
    let written_title = files.written_title().cloned();
    let doc = files.doc;

    // A note with the result flag has no result yet: a missing transcript,
    // summary or audio does not wait for the sync (R6, KTD1). Its notes
    // still do.
    let flag = doc.result_flag();
    let mut waiting = Vec::new();
    let (transcript, transcript_fingerprint) = document_state(
        dir,
        "transcript.md",
        doc.transcript_fingerprint.as_deref(),
        options,
    );
    if transcript != DocumentState::Matches && flag.is_none() {
        waiting.push(Waiting::Transcript);
    }
    let (summary, summary_fingerprint) = document_state(
        dir,
        "summary.md",
        doc.summary_fingerprint.as_deref(),
        options,
    );
    if summary != DocumentState::Matches && flag.is_none() {
        waiting.push(Waiting::Summary);
    }
    let notes = read_notes(dir, options.read_timeout);
    if notes.is_none() {
        waiting.push(Waiting::Notes);
    }
    // Only the metadata: the audio is never opened (R12). A segment with a
    // removal marker does not wait for its audio (R15).
    let removed_audio = removed_audio(dir, &doc);
    for segment in &doc.segments {
        if flag.is_none()
            && !removed_audio.contains(&segment.audio)
            && !exists(&dir.join(&segment.audio))
        {
            waiting.push(Waiting::Audio(segment.audio.clone()));
        }
    }
    let remote = remote_state(dir, layout, flag, options);

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
        note_type: None,
        name: dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        created: std::fs::metadata(dir)
            .ok()
            .and_then(|m| m.created().or_else(|_| m.modified()).ok()),
        doc,
        layout,
        written_title,
        waiting,
        transcript,
        summary,
        removed_audio,
        transcript_fingerprint,
        summary_fingerprint,
        notes,
        conflict_copies,
        remote,
    }))
}

/// What the note folder `dir` shows about work on the note on another
/// computer (KTD5). A fresh in-use marker of another run decides first,
/// whatever `flag` says. A marker that cannot be read counts by its file
/// time ([`in_use_marker::marker_fresh`]), and a folder whose names cannot
/// be listed shows no marker. This reads the names in `dir`, the small
/// marker files and the file time of the file that holds the records only:
/// `longhand.json`, or `session.md` in the old layout.
fn remote_state(
    dir: &Path,
    layout: Layout,
    flag: Option<ResultFlag>,
    options: &ScanOptions,
) -> Option<RemoteState> {
    let now = Local::now().fixed_offset();
    let in_use = in_use_marker::in_use_except(dir, &options.own_runs, now).unwrap_or_else(|e| {
        log::warn!(
            "library: cannot read the in-use markers in {}: {e}",
            dir.display()
        );
        false
    });
    if in_use {
        return Some(RemoteState::InUse);
    }
    if flag != Some(ResultFlag::NotSaved) {
        return None;
    }
    // A new note's marker can sync after its records. Until the file is as
    // old as a marker stays fresh, the note counts as in use.
    let records = match layout {
        Layout::New => NOTE_RECORD_FILE,
        Layout::Old => SESSION_FILE,
    };
    let age = std::fs::metadata(dir.join(records))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .and_then(|age| TimeDelta::from_std(age).ok());
    match age {
        Some(age) if age >= in_use_marker::FRESH_FOR => Some(RemoteState::NotFinished),
        _ => Some(RemoteState::InUse),
    }
}

/// The text of `notes.md` in `dir`. `None` when the file is missing, cannot
/// be read in `timeout`, or is not UTF-8: the notes then wait for the sync.
pub(crate) fn read_notes(dir: &Path, timeout: Duration) -> Option<String> {
    match read_timed(dir.join("notes.md"), timeout) {
        Read::Bytes(bytes) => String::from_utf8(bytes).ok(),
        Read::Missing | Read::Failed(_) => None,
    }
}

/// The state of the document `name` in `dir`, and its fingerprint when the
/// scan read it.
fn document_state(
    dir: &Path,
    name: &str,
    expected: Option<&str>,
    options: &ScanOptions,
) -> (DocumentState, Option<String>) {
    match read_timed(dir.join(name), options.read_timeout) {
        Read::Bytes(bytes) => {
            let actual = fingerprint(&bytes);
            let state = if expected.is_none_or(|expected| expected == actual) {
                DocumentState::Matches
            } else {
                DocumentState::Differs
            };
            (state, Some(actual))
        }
        Read::Missing => (DocumentState::Missing, None),
        Read::Failed(_) => (DocumentState::Unreadable, None),
    }
}

/// The audio files of the segments of `doc` whose removal marker is in the
/// note folder `dir`, sorted. This reads metadata only and opens no file.
pub(crate) fn removed_audio(dir: &Path, doc: &SessionDoc) -> Vec<String> {
    let mut removed: Vec<String> = doc
        .segments
        .iter()
        .filter(|segment| folder::audio_removed(dir, &segment.audio))
        .map(|segment| segment.audio.clone())
        .collect();
    removed.sort();
    removed.dedup();
    removed
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

/// Keep one folder for each note id (KTD5): the one folder whose name is
/// the name that its `session.md` gives, then the folder at the row's home
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
                note_type: real.note_type,
                doc: real.doc,
                layout: real.layout,
                written_title: real.written_title,
                waiting: real.waiting,
                transcript: real.transcript,
                summary: real.summary,
                removed_audio: real.removed_audio,
                transcript_fingerprint: real.transcript_fingerprint,
                summary_fingerprint: real.summary_fingerprint,
                notes: real.notes,
                conflict_copies: real.conflict_copies,
                duplicate_folders,
                remote: real.remote,
            }
        })
        .collect();
    notes.sort_by(|a, b| a.path.cmp(&b.path));
    notes
}

fn real_index(id: &str, group: &[Found], known: &Known) -> usize {
    // The folder name that the note's start time and title give. A conflict
    // copy has another name, also when it synced first and became the home
    // folder.
    let expected = |f: &Found| {
        session_save::naming::folder_name(f.doc.started, f.doc.title.as_deref()) == f.name
    };
    if let [only] = group
        .iter()
        .enumerate()
        .filter(|(_, f)| expected(f))
        .map(|(i, _)| i)
        .collect::<Vec<_>>()[..]
    {
        return only;
    }
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
