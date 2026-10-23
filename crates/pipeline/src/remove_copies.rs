//! The cleanup of old result copies (U7 of the result copies plan, KTD9):
//! the step that deletes, in one saved note folder, the copies that older
//! builds wrote.
//!
//! The step deletes only fixed names:
//!
//! - the `.longhand-results` folder, when it is a real folder (not a link).
//!   Each child folder must have a name of `[A-Za-z0-9-]` characters and
//!   hold only `transcript.md`, `summary.md`, `recognition.jsonl`,
//!   `.DS_Store` and write leftovers `.<one of those three names>.<16
//!   hex>.tmp`. An empty child folder also qualifies. The folder itself can
//!   also hold a `.DS_Store`.
//! - a regular `.longhand-result-transition.json` file.
//! - a regular `recognition.jsonl` file, when all audio of the note is
//!   removed (KTD8). This also finishes a delete that a crash stopped
//!   after the last audio delete.
//!
//! Each other entry stays, and the step logs it. The step deletes the
//! copies only when `transcript.md` and `summary.md` exist (R14). It checks
//! with metadata only, and opens no file (R12). Before each delete, it
//! checks that the note folder is reachable. The step never writes a file,
//! and never deletes a document or audio.

use std::io;
use std::path::{Path, PathBuf};

use session_core::folder;
use session_core::jobs::Claims;
use session_core::recognition::ARTIFACT_FILE;
use session_core::{Segment, SessionRecord, Store};

use crate::remove_audio::{
    FileDelete, Guard, RemoveAudioError, delete_claim, home_dir, remove_recognition,
    remove_regular_file, safe_audio_name, saved_row,
};
use crate::{SharedStore, lock};

/// The folder of the result copies that older builds wrote.
pub const RESULTS_FOLDER: &str = ".longhand-results";

/// The transition file that older builds wrote.
pub const TRANSITION_FILE: &str = ".longhand-result-transition.json";

/// The file that Finder writes in a folder.
const FINDER_FILE: &str = ".DS_Store";

/// The result files that a child folder of [`RESULTS_FOLDER`] can hold.
const RESULT_FILES: [&str; 3] = ["transcript.md", "summary.md", ARTIFACT_FILE];

/// What the cleanup did in one note folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cleanup {
    /// The files and folders that the step deleted.
    pub deleted: Vec<PathBuf>,
    /// The entries with a fixed name, or in the results folder, that the
    /// step did not delete. The step logs each one.
    pub kept: Vec<PathBuf>,
}

/// Whether each segment of the note has its removal marker and no audio
/// file. A note with no segment has no audio to remove. This reads
/// metadata only.
fn audio_gone(home: &Path, segments: &[Segment]) -> bool {
    !segments.is_empty()
        && segments.iter().all(|s| {
            safe_audio_name(&s.audio_file)
                && folder::audio_removed(home, &s.audio_file)
                && home
                    .join(&s.audio_file)
                    .symlink_metadata()
                    .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
        })
}

/// Whether the note folder `home` can hold something that the cleanup
/// deletes. The removal pass uses this check before it calls the step, so
/// a pass with nothing to clean takes no claim. It reads metadata only.
pub fn cleanup_due(home: &Path, segments: &[Segment]) -> bool {
    let present = |name: &str| home.join(name).symlink_metadata().is_ok();
    present(RESULTS_FOLDER)
        || present(TRANSITION_FILE)
        || (present(ARTIFACT_FILE) && audio_gone(home, segments))
}

/// Delete the old result copies of the saved note `note_id`:
/// [`remove_copies_with`] with a permanent delete.
pub fn remove_copies<F, S>(
    store: &SharedStore,
    claims: &Claims,
    note_id: &str,
    guard: Guard<F, S>,
) -> Result<Cleanup, RemoveAudioError>
where
    F: FnOnce(&SessionRecord) -> Result<(), &'static str>,
    S: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
{
    remove_copies_with(store, claims, note_id, guard, |path| {
        std::fs::remove_file(path)
    })
}

/// Delete the old result copies of the saved note `note_id`, and delete
/// each file with `remove`. See the module documentation.
///
/// The claim, the check that the note is saved and `guard` are the same as
/// in [`crate::remove_audio::remove_audio_with`]. The step does not bring
/// the row up to date, and it does not change the row. It returns
/// [`RemoveAudioError::NoFolder`] when the note folder stops being
/// reachable. It then deletes nothing more.
pub fn remove_copies_with<F, S, R>(
    store: &SharedStore,
    claims: &Claims,
    note_id: &str,
    guard: Guard<F, S>,
    mut remove: R,
) -> Result<Cleanup, RemoveAudioError>
where
    F: FnOnce(&SessionRecord) -> Result<(), &'static str>,
    S: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
    R: FnMut(&Path) -> io::Result<()>,
{
    let _claim = delete_claim(claims, note_id)?;
    // The folder check reads files, so it runs with no store lock.
    let checked = saved_row(&lock(store), note_id)?;
    let folder_check = (guard.folder)(&checked);
    let segments = {
        let store = lock(store);
        let record = saved_row(&store, note_id)?;
        if record.home_path != checked.home_path {
            return Err(RemoveAudioError::Busy("moving"));
        }
        (guard.store)(&store, &record).map_err(RemoveAudioError::Busy)?;
        folder_check.map_err(RemoveAudioError::Busy)?;
        store.segments(note_id)?
    };
    let Some(home) = checked.home_path else {
        return Err(RemoveAudioError::NoFolder);
    };
    let home = home_dir(Path::new(&home))?;

    let mut cleanup = Cleanup::default();
    let mut step = Step {
        home: &home,
        remove: &mut remove,
        cleanup: &mut cleanup,
    };
    // Metadata only: an online-only file is not downloaded (R12).
    let has_result = ["transcript.md", "summary.md"]
        .iter()
        .all(|name| home.join(name).metadata().is_ok_and(|m| m.is_file()));
    if has_result {
        step.clean_results()?;
        step.reachable()?;
        let transition = home.join(TRANSITION_FILE);
        let deleted = remove_regular_file(&transition, step.remove);
        step.record(transition, deleted);
    } else {
        // R14: the copies can hold the only result of the note.
        for name in [RESULTS_FOLDER, TRANSITION_FILE] {
            let path = home.join(name);
            if path.symlink_metadata().is_ok() {
                log::warn!(
                    "audio removal: {note_id} has no transcript.md or summary.md, so {} stays",
                    path.display()
                );
                step.cleanup.kept.push(path);
            }
        }
    }
    if audio_gone(&home, &segments) {
        step.reachable()?;
        let deleted = remove_recognition(&home, step.remove);
        step.record(home.join(ARTIFACT_FILE), deleted);
    }
    Ok(cleanup)
}

/// Whether `name` has the shape of a child folder name that older builds
/// wrote: one or more `[A-Za-z0-9-]` characters.
fn generation_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Whether `name` is a file name that a child folder of [`RESULTS_FOLDER`]
/// can hold: a result file, `.DS_Store`, or a write leftover
/// `.<result file>.<16 lowercase hex>.tmp`.
fn copy_file_name(name: &str) -> bool {
    if RESULT_FILES.contains(&name) || name == FINDER_FILE {
        return true;
    }
    let Some((file, hex)) = name
        .strip_prefix('.')
        .and_then(|rest| rest.strip_suffix(".tmp"))
        .and_then(|rest| rest.rsplit_once('.'))
    else {
        return false;
    };
    RESULT_FILES.contains(&file)
        && hex.len() == 16
        && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The state of the step in one note folder.
struct Step<'a, R> {
    /// The canonical home folder.
    home: &'a Path,
    remove: &'a mut R,
    cleanup: &'a mut Cleanup,
}

impl<R> Step<'_, R>
where
    R: FnMut(&Path) -> io::Result<()>,
{
    /// Stop the step when the note folder or its storage folder is not
    /// reachable: a removed disk, a lost share or a signed-out sync client.
    /// The storage folder is the parent folder, or the parent of a type
    /// folder (KTD4 of the note types plan), with its library marker.
    fn reachable(&self) -> Result<(), RemoveAudioError> {
        let reachable = self.home.is_dir() && library::scan::storage_place(self.home).is_some();
        if reachable {
            Ok(())
        } else {
            Err(RemoveAudioError::NoFolder)
        }
    }

    fn keep(&mut self, path: PathBuf, reason: &str) {
        log::warn!("audio removal: {} stays: {reason}", path.display());
        self.cleanup.kept.push(path);
    }

    fn record(&mut self, path: PathBuf, deleted: FileDelete) {
        match deleted {
            FileDelete::Deleted => self.cleanup.deleted.push(path),
            FileDelete::Kept => self.cleanup.kept.push(path),
            FileDelete::Missing => {}
        }
    }

    /// Delete the empty folder `path`. A folder that is not empty stays.
    fn remove_folder(&mut self, path: PathBuf) -> Result<bool, RemoveAudioError> {
        self.reachable()?;
        match std::fs::remove_dir(&path) {
            Ok(()) => {
                self.cleanup.deleted.push(path);
                Ok(true)
            }
            Err(e) => {
                self.keep(path, &format!("cannot delete the folder: {e}"));
                Ok(false)
            }
        }
    }

    /// Delete each child folder of [`RESULTS_FOLDER`] that has the shape
    /// that older builds wrote, then the folder itself when nothing else is
    /// in it.
    fn clean_results(&mut self) -> Result<(), RemoveAudioError> {
        let dir = self.home.join(RESULTS_FOLDER);
        match dir.symlink_metadata() {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                self.keep(dir, "it is not a real folder");
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                self.keep(dir, &format!("cannot read it: {e}"));
                return Ok(());
            }
        }
        let entries = match list(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                self.keep(dir, &format!("cannot list it: {e}"));
                return Ok(());
            }
        };
        let mut left = false;
        let mut finder = None;
        for (name, path, meta) in entries {
            if name.as_deref() == Some(FINDER_FILE) && meta.is_file() {
                finder = Some(path);
            } else if meta.is_dir() && name.as_deref().is_some_and(generation_name) {
                left |= !self.clean_child(path)?;
            } else {
                self.keep(path, "older builds do not write this entry");
                left = true;
            }
        }
        if left {
            return Ok(());
        }
        if let Some(path) = finder {
            self.reachable()?;
            let deleted = remove_regular_file(&path, self.remove);
            self.record(path, deleted);
            if deleted == FileDelete::Kept {
                return Ok(());
            }
        }
        self.remove_folder(dir)?;
        Ok(())
    }

    /// Delete the child folder `dir` and its files when it holds only the
    /// files of [`copy_file_name`]. Returns whether the folder went.
    fn clean_child(&mut self, dir: PathBuf) -> Result<bool, RemoveAudioError> {
        let entries = match list(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                self.keep(dir, &format!("cannot list it: {e}"));
                return Ok(false);
            }
        };
        // A link, a folder or an unknown name keeps the whole child folder.
        let expected = entries
            .iter()
            .all(|(name, _, meta)| meta.is_file() && name.as_deref().is_some_and(copy_file_name));
        if !expected {
            self.keep(dir, "it holds an entry that older builds do not write");
            return Ok(false);
        }
        for (_, path, _) in entries {
            self.reachable()?;
            let deleted = remove_regular_file(&path, self.remove);
            self.record(path, deleted);
            if deleted == FileDelete::Kept {
                return Ok(false);
            }
        }
        self.remove_folder(dir)
    }
}

/// The entries of `dir`: the name (`None` when it is not UTF-8), the path
/// and the metadata of the entry itself, not of a link target.
fn list(dir: &Path) -> io::Result<Vec<(Option<String>, PathBuf, std::fs::Metadata)>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let meta = path.symlink_metadata()?;
        entries.push((entry.file_name().into_string().ok(), path, meta));
    }
    entries.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(entries)
}
