//! The rename of a home folder (KTD8, R9): a step-then-record operation.
//! The caller records each [`Move`] durably before this module takes it,
//! so a crash at any point leaves a record that [`resolve`] can settle
//! from disk alone.
//!
//! A home folder has one name at every moment, and a rename never replaces
//! another folder. A change of case alone goes through a temporary name,
//! because a case-insensitive volume sees the old and the new name as one.

use crate::rename::{Fs, is_in_use, is_name_taken, rename_new_waiting};
pub use crate::rename::{IN_USE_DELAY, IN_USE_RETRIES};
use crate::{MAX_NAMES, fsync, naming};
use std::io;
use std::path::{Path, PathBuf};

/// The temporary name of a change of case alone starts with this.
pub const TEMP_PREFIX: &str = ".longhand-rename-";

/// One recorded move of a home folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub from: PathBuf,
    pub to: PathBuf,
    /// Set for a change of case alone: the folder goes from `from` to
    /// `temp`, then from `temp` to `to`.
    pub temp: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum RenameError {
    #[error("the folder {} is in use by another app, so it keeps its name ({source})", path.display())]
    InUse { path: PathBuf, source: io::Error },
    #[error("no free name for {name} after {MAX_NAMES} tries, so the folder keeps its name")]
    NoFreeName { name: String },
    #[error("the folder {} is missing", .0.display())]
    Missing(PathBuf),
    #[error("the rename could not be recorded: {0}")]
    Record(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// What a recorded move found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// The folder is at `to`: the rename must set `home_path` to it.
    Moved(PathBuf),
    /// The folder is still at `from`: the record can go.
    NotMoved,
    /// The folder is at none of the recorded names.
    Missing,
}

/// The temporary name for `session_id`'s change of case.
pub fn temp_name(session_id: &str) -> String {
    format!("{TEMP_PREFIX}{session_id}")
}

/// Whether a folder called `current` already has the name `name` gives:
/// the name itself or the name with a suffix that a taken name gave it
/// (see [`naming::candidate`]). A folder with a suffix is not renamed
/// again only to try for a name without one.
pub fn has_name(current: &str, name: &str) -> bool {
    current == name
        || current
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('-'))
            .and_then(|n| n.parse::<usize>().ok())
            .is_some_and(|n| (2..=MAX_NAMES).contains(&n))
}

/// Rename the folder `from` to `name`, or to the first free name with a
/// suffix, beside it. `record` receives each move before this function
/// takes it. Returns the folder's path, which is `from` when the folder
/// already has one of the names. On an error, the folder is at `from`, or
/// the last recorded move tells [`resolve`] where it is.
pub fn rename(
    from: &Path,
    name: &str,
    session_id: &str,
    fs: &dyn Fs,
    record: &mut dyn FnMut(&Move) -> Result<(), RenameError>,
) -> Result<PathBuf, RenameError> {
    let Some(parent) = from.parent() else {
        return Err(RenameError::Missing(from.to_path_buf()));
    };
    rename_into(from, parent, name, session_id, fs, record)
}

/// Move the folder `from` into the folder `parent` with the name `name`,
/// or the first free name with a suffix (R27 of the note types and tags
/// plan). The rules of [`rename`] apply: each move is recorded before it
/// is taken, and no folder is replaced. A `parent` that is the folder of
/// `from` makes this a [`rename`]. The caller makes `parent`.
pub fn rename_into(
    from: &Path,
    parent: &Path,
    name: &str,
    session_id: &str,
    fs: &dyn Fs,
    record: &mut dyn FnMut(&Move) -> Result<(), RenameError>,
) -> Result<PathBuf, RenameError> {
    if !exists_exactly(from) {
        return Err(RenameError::Missing(from.to_path_buf()));
    }
    let (Some(old_parent), Some(current)) =
        (from.parent(), from.file_name().and_then(|n| n.to_str()))
    else {
        return Err(RenameError::Missing(from.to_path_buf()));
    };
    let same_parent = old_parent == parent;
    // The folder that each next move takes. A change of case whose names
    // are both taken leaves the folder at its temporary name, and the next
    // names move it from there.
    let mut source = from.to_path_buf();
    let mut at_temp = false;
    for attempt in 1..=MAX_NAMES {
        let target = naming::candidate(name, attempt);
        if same_parent && !at_temp && target == current {
            return Ok(from.to_path_buf());
        }
        let to = parent.join(&target);
        if exists_exactly(&to) {
            continue;
        }
        let case_only = same_parent && !at_temp && target.to_lowercase() == current.to_lowercase();
        let intent = Move {
            from: source.clone(),
            to: to.clone(),
            temp: case_only.then(|| parent.join(temp_name(session_id))),
        };
        record(&intent)?;
        match take(&intent, fs) {
            Ok(()) => return Ok(to),
            Err(Refused::Taken) => continue,
            Err(Refused::AtTemp(temp)) => {
                log::info!(
                    "{} and {} are taken, so the folder goes to the next free name",
                    to.display(),
                    from.display()
                );
                source = temp;
                at_temp = true;
            }
            Err(Refused::Failed(e)) => return Err(e),
        }
    }
    Err(RenameError::NoFreeName {
        name: name.to_string(),
    })
}

/// Settle a move that was recorded and whose end was not, from disk alone:
/// - if `from` is there, the move did not happen;
/// - if only `temp` is there, the move to `to` is finished now;
/// - if `to` is there, the move happened.
pub fn resolve(intent: &Move, fs: &dyn Fs) -> Result<Resolved, RenameError> {
    if exists_exactly(&intent.from) {
        return Ok(Resolved::NotMoved);
    }
    if let Some(temp) = &intent.temp
        && exists_exactly(temp)
    {
        return match second_move(temp, &intent.to, &intent.from, fs) {
            Ok(()) => Ok(Resolved::Moved(intent.to.clone())),
            Err(Refused::Taken) => Ok(Resolved::NotMoved),
            // The record keeps the temporary name, the only name that
            // finds the folder.
            Err(Refused::AtTemp(temp)) => Err(RenameError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "the folder is at {}, and its old and new names are taken",
                    temp.display()
                ),
            ))),
            Err(Refused::Failed(e)) => Err(e),
        };
    }
    if exists_exactly(&intent.to) {
        return Ok(Resolved::Moved(intent.to.clone()));
    }
    Ok(Resolved::Missing)
}

/// Why a move did not happen.
enum Refused {
    /// Another folder has the name: the next name can be tried. The folder
    /// is at `from`.
    Taken,
    /// The second move of a change of case and the move back were both
    /// refused as taken: the folder is at this temporary name.
    AtTemp(PathBuf),
    Failed(RenameError),
}

/// Take one recorded move. When it fails, the folder is at `from`, unless
/// the error says it could not be put back.
fn take(intent: &Move, fs: &dyn Fs) -> Result<(), Refused> {
    let Some(temp) = &intent.temp else {
        move_once(&intent.from, &intent.to, fs)?;
        return sync_parent(&intent.to);
    };
    // A taken temporary name is a leftover, not a name to skip.
    move_once(&intent.from, temp, fs).map_err(|r| match r {
        Refused::Taken => Refused::Failed(RenameError::Io(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} is in the way", temp.display()),
        ))),
        other => other,
    })?;
    sync_parent(temp)?;
    second_move(temp, &intent.to, &intent.from, fs)
}

/// The move from the temporary name to `to`. When it is refused, the
/// folder goes back to `from`, so it does not stay under the hidden name.
/// When the move back fails too, the result says that the folder is at
/// `temp`.
fn second_move(temp: &Path, to: &Path, from: &Path, fs: &dyn Fs) -> Result<(), Refused> {
    match move_once(temp, to, fs) {
        Ok(()) => sync_parent(to),
        Err(refused) => {
            match move_once(temp, from, fs) {
                Ok(()) => {}
                Err(Refused::Taken) => return Err(Refused::AtTemp(temp.to_path_buf())),
                Err(Refused::AtTemp(_)) => unreachable!("one move gives no temporary name"),
                Err(Refused::Failed(e)) => {
                    return Err(Refused::Failed(RenameError::Io(io::Error::other(format!(
                        "the folder is at {}: it could not go back to its name ({e})",
                        temp.display()
                    )))));
                }
            }
            sync_parent(from)?;
            Err(refused)
        }
    }
}

/// One rename that never replaces, tried again while the folder is in use.
fn move_once(from: &Path, to: &Path, fs: &dyn Fs) -> Result<(), Refused> {
    match rename_new_waiting(fs, from, to) {
        Ok(()) => Ok(()),
        Err(e) if is_in_use(&e) => Err(Refused::Failed(RenameError::InUse {
            path: from.to_path_buf(),
            source: e,
        })),
        Err(e) if is_name_taken(&e) => Err(Refused::Taken),
        Err(e) => Err(Refused::Failed(e.into())),
    }
}

fn sync_parent(path: &Path) -> Result<(), Refused> {
    match path.parent() {
        Some(parent) => fsync::sync_dir(parent).map_err(|e| Refused::Failed(e.into())),
        None => Ok(()),
    }
}

/// Whether an entry with exactly this name is in the path's folder. On a
/// case-insensitive volume, a name that differs only in case finds the same
/// entry, so the name is compared with the folder's listing.
pub fn exists_exactly(path: &Path) -> bool {
    if path.symlink_metadata().is_err() {
        return false;
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return true;
    };
    match std::fs::read_dir(parent) {
        Ok(entries) => entries.flatten().any(|e| e.file_name() == name),
        Err(_) => true,
    }
}
