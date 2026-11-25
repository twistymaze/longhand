//! The notes of one storage folder, read with the library scan, and the
//! guarded reads of their files.
//!
//! Before the scan, a walk with `symlink_metadata` finds each link. A
//! folder that is a link is not read. A note folder whose `session.md` or
//! `longhand.json` links outside the storage folder is not read. A note
//! folder with another file that links outside the storage folder is not
//! scanned: only its two definition files are read, and each linked part
//! has the state `unreadable`. So no tool reads a file outside the storage
//! folder (R7). The scan writes the library marker only when it is
//! missing, and the tools refuse a folder with no marker, so the scan
//! writes nothing (R1).

use std::collections::{BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, FixedOffset, Local};
use library::scan::{
    Known, NoteRead, ProblemKind, Read, ScanOptions, marker, read_note, read_timed, scan_folders,
};
use schemars::JsonSchema;
use serde::Serialize;
use session_core::folder::{ResultFlag, SessionDoc, transcript_removed};
use session_core::note_folder::{Layout, READ_TIMEOUT, SESSION_FILE};
use session_core::note_record::NOTE_RECORD_FILE;
use session_save::in_use_marker::{is_marker_name, marker_fresh};

use crate::{ErrorKind, ToolError};

pub const TRANSCRIPT_FILE: &str = "transcript.md";
pub const SUMMARY_FILE: &str = "summary.md";
pub const NOTES_FILE: &str = "notes.md";

/// The name of the type of a note that is not in a type folder.
pub use session_save::naming::GENERAL;

/// The state of one part of a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PartState {
    /// The file is there and can be read.
    Present,
    /// The file is not there.
    Missing,
    /// Longhand removed the transcript.
    Removed,
    /// The file cannot be read, or it links outside the storage folder.
    Unreadable,
}

/// The kind of a folder that gives no note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProblemFolderKind {
    /// A file cannot be read.
    Unreadable,
    /// `session.md` or `longhand.json` is not valid.
    Invalid,
    /// A newer Longhand wrote `longhand.json`.
    NewerLayout,
    /// The records have not come from the sync yet.
    WaitingForRecord,
    /// The folder, or a file that defines the note, links to another
    /// place.
    Link,
}

/// A folder that looks like a note folder but gives no note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProblemFolder {
    /// The folder, relative to the storage folder.
    pub folder: String,
    pub note_id: Option<String>,
    pub kind: ProblemFolderKind,
    pub reason: String,
}

/// One note of the storage folder.
#[derive(Debug, Clone)]
pub struct Note {
    pub id: String,
    /// The note folder.
    pub path: PathBuf,
    /// The note folder, relative to the storage folder.
    pub folder: String,
    pub note_type: Option<String>,
    pub doc: SessionDoc,
    pub layout: Layout,
    /// Why the note is busy. Empty when it is not busy (KTD6).
    pub busy: Vec<String>,
    /// Other folders with the same note id.
    pub duplicates: usize,
    /// The files of the note folder that link outside the storage folder.
    linked: BTreeSet<String>,
}

impl Note {
    pub fn type_name(&self) -> String {
        self.note_type.clone().unwrap_or_else(|| GENERAL.into())
    }

    pub fn started(&self) -> DateTime<FixedOffset> {
        self.doc.started
    }

    /// Read the part `name` of the note. A file that links outside the
    /// storage folder is not read.
    pub fn read_part(&self, root: &Path, name: &str) -> (PartState, Option<String>) {
        if name == TRANSCRIPT_FILE && transcript_removed(&self.path) {
            return (PartState::Removed, None);
        }
        if self.linked.contains(name) || !inside(root, &self.path.join(name)) {
            return (PartState::Unreadable, None);
        }
        match read_timed(self.path.join(name), READ_TIMEOUT) {
            Read::Missing => (PartState::Missing, None),
            Read::Failed(_) => (PartState::Unreadable, None),
            Read::Bytes(bytes) => match String::from_utf8(bytes) {
                Ok(text) => (PartState::Present, Some(text)),
                Err(_) => (PartState::Unreadable, None),
            },
        }
    }
}

/// The notes and the problem folders of one storage folder.
#[derive(Debug, Clone)]
pub struct Storage {
    /// The storage folder, with every link resolved.
    pub root: PathBuf,
    pub notes: Vec<Note>,
    pub problems: Vec<ProblemFolder>,
}

/// True when `path`, with every link resolved, is in `root`. `root` has
/// every link resolved. A path that does not exist is in `root` when its
/// folder is.
fn inside(root: &Path, path: &Path) -> bool {
    match path.canonicalize() {
        Ok(real) => real.starts_with(root),
        Err(_) => path
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .is_some_and(|real| real.starts_with(root)),
    }
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// The text of `path` relative to `root`, with `/` between the names.
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// What the walk found in one folder entry.
enum Entry {
    /// A folder that is not a link.
    Folder(PathBuf),
    /// A folder or a file that is a link.
    Link(PathBuf),
}

/// The entries of `dir` that are folders or links, with no hidden name.
fn folder_entries(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if is_hidden(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => out.push(Entry::Link(path)),
            Ok(m) if m.is_dir() => out.push(Entry::Folder(path)),
            _ => {}
        }
    }
    out.sort_by(|a, b| key(a).cmp(key(b)));
    Ok(out)
}

fn key(entry: &Entry) -> &Path {
    match entry {
        Entry::Folder(path) | Entry::Link(path) => path,
    }
}

/// Whether `dir` holds `session.md` or `longhand.json`. A link counts.
fn is_note_folder(dir: &Path) -> bool {
    [SESSION_FILE, NOTE_RECORD_FILE]
        .iter()
        .any(|name| std::fs::symlink_metadata(dir.join(name)).is_ok())
}

/// The names of the files in the note folder `dir` that link outside
/// `root`.
fn linked_files(root: &Path, dir: &Path) -> BTreeSet<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_symlink()) && !inside(root, &entry.path())
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Whether a subfolder of the note folder `dir` holds a file that links
/// outside `root`. The scan reads such a note folder as a type folder when
/// its own definition gives no note, so it would read the linked file.
fn nested_link(root: &Path, dir: &Path) -> bool {
    folder_entries(dir).is_ok_and(|entries| {
        entries.iter().any(|entry| match entry {
            Entry::Folder(inner) => !linked_files(root, inner).is_empty(),
            Entry::Link(_) => false,
        })
    })
}

/// The busy reasons of the note in `dir` (KTD6): a fresh in-use marker, a
/// recording that is not saved, or a move to another type that waits. A
/// marker that links outside the storage folder is not read.
fn busy_reasons(dir: &Path, doc: &SessionDoc, linked: &BTreeSet<String>) -> Vec<String> {
    let now = Local::now().fixed_offset();
    let mut reasons = Vec::new();
    let mut markers: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| is_marker_name(name) && !linked.contains(name))
                .collect()
        })
        .unwrap_or_default();
    markers.sort();
    if markers
        .iter()
        .any(|name| marker_fresh(&dir.join(name), now))
    {
        reasons.push("a computer records or processes it".to_string());
    }
    if doc.result_flag() == Some(ResultFlag::NotSaved) {
        reasons.push("its recording is not saved yet".to_string());
    }
    if doc.pending_type.is_some() {
        reasons.push("a move to another type waits".to_string());
    }
    reasons
}

impl Storage {
    /// Read the notes of the storage folder `root`.
    pub fn open(root: &Path) -> Result<Self, ToolError> {
        let real = root.canonicalize().map_err(|e| {
            ToolError::new(
                ErrorKind::NotAStorageFolder,
                format!("{} is not a folder that can be read: {e}", root.display()),
            )
        })?;
        let settings_linked = marker(&real).parent().is_some_and(|settings| {
            std::fs::symlink_metadata(settings).is_ok_and(|m| m.file_type().is_symlink())
        });
        if settings_linked {
            return Err(ToolError::new(
                ErrorKind::NotAStorageFolder,
                format!(
                    "{}: the .longhand folder is a link, so the tools do not read it",
                    root.display()
                ),
            ));
        }
        if !real.is_dir() || !marker(&real).is_file() {
            return Err(ToolError::new(
                ErrorKind::NotAStorageFolder,
                format!(
                    "{} is not a Longhand storage folder: it has no .longhand/library",
                    root.display()
                ),
            ));
        }
        let unreadable = |e: std::io::Error| {
            ToolError::new(
                ErrorKind::Unreadable,
                format!("the storage folder cannot be listed: {e}"),
            )
        };
        let mut problems = Vec::new();
        let mut candidates: Vec<(PathBuf, Option<String>)> = Vec::new();
        let link_problem = |path: &Path| ProblemFolder {
            folder: relative(&real, path),
            note_id: None,
            kind: ProblemFolderKind::Link,
            reason: "the folder is a link, so the tools do not read it".into(),
        };
        for entry in folder_entries(&real).map_err(unreadable)? {
            match entry {
                Entry::Link(path) => {
                    if path.is_dir() {
                        problems.push(link_problem(&path));
                    }
                }
                Entry::Folder(path) if is_note_folder(&path) => candidates.push((path, None)),
                Entry::Folder(path) => {
                    let note_type = path.file_name().map(|n| n.to_string_lossy().into_owned());
                    let Ok(inner) = folder_entries(&path) else {
                        continue;
                    };
                    for entry in inner {
                        match entry {
                            Entry::Link(inner) => {
                                if inner.is_dir() {
                                    problems.push(link_problem(&inner));
                                }
                            }
                            Entry::Folder(inner) if is_note_folder(&inner) => {
                                candidates.push((inner, note_type.clone()));
                            }
                            Entry::Folder(_) => {}
                        }
                    }
                }
            }
        }

        let mut clean = HashSet::new();
        let mut notes = Vec::new();
        for (path, note_type) in candidates {
            let linked = linked_files(&real, &path);
            if linked.contains(SESSION_FILE) || linked.contains(NOTE_RECORD_FILE) {
                problems.push(ProblemFolder {
                    folder: relative(&real, &path),
                    note_id: None,
                    kind: ProblemFolderKind::Link,
                    reason: "a file that defines the note links outside the storage folder".into(),
                });
            } else if linked.is_empty() && !nested_link(&real, &path) {
                clean.insert(path);
            } else {
                match read_note(&path, READ_TIMEOUT) {
                    NoteRead::Note(files) => {
                        let doc = files.doc.clone();
                        notes.push(Note {
                            id: doc.note_id.clone(),
                            folder: relative(&real, &path),
                            busy: busy_reasons(&path, &doc, &linked),
                            layout: files.layout,
                            doc,
                            note_type,
                            path,
                            duplicates: 0,
                            linked,
                        });
                    }
                    _ => problems.push(ProblemFolder {
                        folder: relative(&real, &path),
                        note_id: None,
                        kind: ProblemFolderKind::Unreadable,
                        reason: "the note files cannot be read".into(),
                    }),
                }
            }
        }

        let options = ScanOptions::default();
        if let Some((scan, _)) = scan_folders(&real, &clean, &Known::default(), &options) {
            for note in scan.notes {
                let linked = BTreeSet::new();
                notes.push(Note {
                    id: note.doc.note_id.clone(),
                    folder: relative(&real, &note.path),
                    busy: busy_reasons(&note.path, &note.doc, &linked),
                    layout: note.layout,
                    duplicates: note.duplicate_folders.len(),
                    doc: note.doc,
                    note_type: note.note_type,
                    path: note.path,
                    linked,
                });
            }
            let problem = |p: library::scan::ProblemFolder, kind| ProblemFolder {
                folder: relative(&real, &p.path),
                note_id: p.known_id,
                kind,
                reason: p.reason,
            };
            for p in scan.unreadable {
                problems.push(problem(p, ProblemFolderKind::Unreadable));
            }
            for p in scan.invalid {
                let kind = match p.problem.as_ref().map(|f| f.kind) {
                    Some(ProblemKind::NewerRecord) => ProblemFolderKind::NewerLayout,
                    _ => ProblemFolderKind::Invalid,
                };
                problems.push(problem(p, kind));
            }
            for p in scan.waiting_for_record {
                problems.push(problem(p, ProblemFolderKind::WaitingForRecord));
            }
        }
        // A note id that two folders give, one scanned and one not, is a
        // duplicate too.
        let mut counts = std::collections::HashMap::<String, usize>::new();
        for note in &notes {
            *counts.entry(note.id.clone()).or_default() += 1;
        }
        for note in &mut notes {
            note.duplicates += counts[&note.id] - 1;
        }
        notes.sort_by(|a, b| {
            b.doc
                .started
                .cmp(&a.doc.started)
                .then_with(|| a.folder.cmp(&b.folder))
        });
        problems.sort_by(|a, b| a.folder.cmp(&b.folder));
        Ok(Self {
            root: real,
            notes,
            problems,
        })
    }
}
