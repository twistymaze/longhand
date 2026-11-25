//! The note selection and the snapshot of a run (KTD8). The snapshot is a
//! storage folder in the run folder with the selected notes and the
//! `.longhand/` settings. The tools, the baseline and the judges read only
//! the snapshot. The copy skips symlinks and in-use markers.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use note_tools::storage::{PartState, SUMMARY_FILE, Storage, TRANSCRIPT_FILE};
use note_tools::{ListNotes, NoteTools, Tool};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use summarize::chunk::Line;

use crate::run_config::NotesFilter;

/// The settings folder of a storage folder.
pub const SETTINGS_FOLDER: &str = ".longhand";

/// A note that the run uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selected {
    pub note_id: String,
    /// The note folder, relative to the storage folder.
    pub folder: String,
    pub note_type: Option<String>,
}

/// A note or a folder that the run does not use, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    /// The note id, or the folder when the folder has no readable id.
    pub note: String,
    pub reason: String,
}

/// The notes of `root` that `filter` selects, and the notes and folders
/// that the run skips. A busy note, a note with a duplicate id, and a note
/// with no transcript or no speech are skipped. An id in `filter.ids` that
/// no note has is an error.
pub fn select(root: &Path, filter: &NotesFilter) -> Result<(Vec<Selected>, Vec<Skipped>), String> {
    let storage = Storage::open(root).map_err(|e| e.to_string())?;
    let mut skipped: Vec<Skipped> = storage
        .problems
        .iter()
        .map(|problem| Skipped {
            note: problem
                .note_id
                .clone()
                .unwrap_or_else(|| problem.folder.clone()),
            reason: format!(
                "{}: {}",
                serde_json::to_value(problem.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default(),
                problem.reason
            ),
        })
        .collect();
    let problem_ids: BTreeSet<&str> = storage
        .problems
        .iter()
        .filter_map(|p| p.note_id.as_deref())
        .collect();

    let mut ids: Vec<String> = Vec::new();
    for id in &filter.ids {
        if !storage.notes.iter().any(|n| &n.id == id) && !problem_ids.contains(id.as_str()) {
            return Err(format!("no note has the id {id}"));
        }
        ids.push(id.clone());
    }
    if let Some(list) = &filter.list {
        let output = NoteTools::new(1_000)
            .call(root, ListNotes::NAME, list.clone())
            .map_err(|e| format!("notes.list: {e}"))?;
        let listed: Vec<String> = output["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| n["note_id"].as_str().map(str::to_string))
            .collect();
        if filter.ids.is_empty() {
            ids = listed;
        } else {
            ids.retain(|id| listed.contains(id));
        }
    }
    ids.dedup();

    let mut selected = Vec::new();
    for id in ids {
        if problem_ids.contains(id.as_str()) {
            continue;
        }
        let Some(note) = storage.notes.iter().find(|n| n.id == id) else {
            continue;
        };
        let reason = if !note.busy.is_empty() {
            Some(format!("busy: {}", note.busy.join(", ")))
        } else if note.duplicates > 0 {
            Some("another folder has the same note id".to_string())
        } else {
            match note.read_part(&storage.root, TRANSCRIPT_FILE) {
                (PartState::Present, Some(text)) => {
                    let said = pipeline::summarize::lines_of(&text)
                        .iter()
                        .any(|line| matches!(line, Line::Turn(_)));
                    (!said).then(|| "the transcript has no speech".to_string())
                }
                (state, _) => Some(format!("no transcript ({state:?})").to_lowercase()),
            }
        };
        match reason {
            Some(reason) => skipped.push(Skipped { note: id, reason }),
            None => selected.push(Selected {
                note_id: note.id.clone(),
                folder: note.folder.clone(),
                note_type: note.note_type.clone(),
            }),
        }
    }
    Ok((selected, skipped))
}

/// Copy the settings folder and the folders of `notes` from `root` into
/// `dest`, which must not exist. Only regular files are copied: a symlink,
/// an in-use marker and a subfolder of a note folder (the audio) are not.
/// Without `keep_summary`, the summary file of a note is not copied.
pub fn copy(
    root: &Path,
    dest: &Path,
    notes: &[Selected],
    keep_summary: bool,
) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    copy_tree(&root.join(SETTINGS_FOLDER), &dest.join(SETTINGS_FOLDER))?;
    for note in notes {
        let from = root.join(&note.folder);
        let to = dest.join(&note.folder);
        fs::create_dir_all(&to).map_err(|e| format!("cannot create {}: {e}", to.display()))?;
        for (name, path) in regular_files(&from)? {
            if !keep_summary && name == SUMMARY_FILE {
                continue;
            }
            fs::copy(&path, to.join(&name))
                .map_err(|e| format!("cannot copy {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// The fingerprint of the files of a note folder: the names and the bytes
/// of its regular files, in name order.
pub fn fingerprint(folder: &Path) -> Result<String, String> {
    let mut hash = Sha256::new();
    for (name, path) in regular_files(folder)? {
        let bytes = fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// The regular files directly in `folder`, by name, with no symlinks and
/// no in-use markers.
fn regular_files(folder: &Path) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    let mut files = Vec::new();
    let entries =
        fs::read_dir(folder).map_err(|e| format!("cannot read {}: {e}", folder.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", folder.display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let kind = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("cannot read {}: {e}", entry.path().display()))?
            .file_type();
        if kind.is_file() && !session_save::in_use_marker::is_marker_name(&name) {
            files.push((name, entry.path()));
        }
    }
    files.sort();
    Ok(files)
}

/// Copy the folder `from` into `to` with its subfolders. Symlinks and
/// in-use markers are not copied. A missing `from` copies nothing.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    // `from` itself can be a link: the walk below checks only its entries.
    if !fs::symlink_metadata(from).is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    fs::create_dir_all(to).map_err(|e| format!("cannot create {}: {e}", to.display()))?;
    let entries = fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", from.display()))?;
        let name = entry.file_name();
        let kind = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("cannot read {}: {e}", entry.path().display()))?
            .file_type();
        if kind.is_dir() {
            copy_tree(&entry.path(), &to.join(&name))?;
        } else if kind.is_file()
            && !session_save::in_use_marker::is_marker_name(&name.to_string_lossy())
        {
            fs::copy(entry.path(), to.join(&name))
                .map_err(|e| format!("cannot copy {}: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}
