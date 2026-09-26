//! Saving (KD4, R22a, R22b, KTD17): the session leaves the working location
//! exactly once, into its destination under its final name, and at every
//! moment exactly one location holds a complete copy. Written for Longhand.
//!
//! The session is assembled in a hidden `.longhand-staging-<id>/` inside
//! the destination, so the last step is a same-volume rename on every path.
//! Getting it there is a rename when the volumes agree and a verified copy
//! when the rename says they do not; no pre-check decides. Saving advances
//! one [`SaveState`] at a time, and the caller records each state durably
//! before the next step, so a crash anywhere resumes from what is on disk.

pub mod fsync;
pub mod home_write;
pub mod naming;
pub mod rename;
pub mod rename_home;
pub mod stage;

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use rename::{Fs, RealFs};

/// Working-location entries that never reach the destination: the 16 kHz
/// processing audio and the raw native-rate chunks the Opus archive has
/// superseded (about 1.4 GB for four hours).
pub const LEFT_BEHIND: &[&str] = &["proc", "audio-recovery"];

/// Written into the working directory before it moves, so a staging
/// directory can be told apart from one this session did not make. It stays
/// through the final rename, so a crash before `Saved` is recorded can find
/// the renamed folder, and is removed in the `Saved` step.
pub const MARKER: &str = ".longhand-saving";

/// Final-name attempts before giving up on the destination.
pub(crate) const MAX_NAMES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum SaveState {
    /// Everything is in the working location.
    Working,
    /// The volumes differ: staging is being filled by copy. A partial
    /// staging directory found in this state is this session's own.
    Copying {
        staging: PathBuf,
    },
    /// Staging holds a complete copy; `copied` when the working copy is
    /// there too.
    Staged {
        staging: PathBuf,
        copied: bool,
    },
    /// Under the final name. The working copy, if one was kept, goes only
    /// once this is recorded (R22b).
    Saved {
        path: PathBuf,
        working_retained: bool,
    },
    Done {
        path: PathBuf,
    },
}

pub struct Job<'a> {
    pub session_id: &'a str,
    pub working: &'a Path,
    pub destination: &'a Path,
    /// See [`naming::folder_name`].
    pub folder_name: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The destination cannot take the session: it waits for the user
    /// (R22b's awaiting-destination), whole in the working location.
    #[error("the storage folder {} {problem}", path.display())]
    Destination { path: PathBuf, problem: String },
    #[error("{} already exists in the storage folder and was not made by this session", .0.display())]
    StagingTaken(PathBuf),
    #[error("{0}")]
    Lost(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The destination exists, is a folder, and takes a new file (R22b).
pub fn check_destination(destination: &Path) -> Result<(), SaveError> {
    let problem = |problem: &str| SaveError::Destination {
        path: destination.to_path_buf(),
        problem: problem.to_string(),
    };
    match std::fs::metadata(destination) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(problem("is missing")),
        Err(e) => return Err(problem(&format!("cannot be read ({e})"))),
        Ok(m) if !m.is_dir() => return Err(problem("is not a folder")),
        Ok(_) => {}
    }
    let probe = destination.join(format!(".longhand-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(problem(&format!("is not writable ({e})"))),
    }
}

/// At launch, remove staging directories in `root` left by sessions that
/// have since completed (U18). Any other staging directory is left: one
/// whose session is still saving or waiting for its destination may hold
/// the only complete copy (R22d).
pub fn sweep_staging(root: &Path, completed: impl Fn(&str) -> bool) -> io::Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut removed = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|n| n.strip_prefix(naming::STAGING_PREFIX))
        else {
            continue;
        };
        if entry.file_type()?.is_dir() && completed(id) {
            std::fs::remove_dir_all(entry.path())?;
            removed.push(entry.path());
        }
    }
    Ok(removed)
}

fn is_ours(staging: &Path, session_id: &str) -> bool {
    std::fs::read_to_string(staging.join(MARKER)).is_ok_and(|m| m.trim() == session_id)
}

fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// Take one save step from `state`.
pub fn step(state: &SaveState, job: &Job, fs: &dyn Fs) -> Result<SaveState, SaveError> {
    let staging = job.destination.join(naming::staging_name(job.session_id));
    match state {
        SaveState::Working => {
            if !exists(job.working) {
                // The move happened and its record did not.
                if is_ours(&staging, job.session_id) {
                    return Ok(SaveState::Staged {
                        staging,
                        copied: false,
                    });
                }
                return Err(SaveError::Lost(format!(
                    "the working copy at {} is gone and the destination holds no staged copy",
                    job.working.display()
                )));
            }
            check_destination(job.destination)?;
            if exists(&staging) {
                return Err(SaveError::StagingTaken(staging));
            }
            std::fs::write(job.working.join(MARKER), job.session_id)?;
            fsync::sync_tree(job.working)?;
            match fs.rename(job.working, &staging) {
                Ok(()) => {
                    fsync::sync_dir(job.destination)?;
                    Ok(SaveState::Staged {
                        staging,
                        copied: false,
                    })
                }
                Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                    Ok(SaveState::Copying { staging })
                }
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    Err(SaveError::Destination {
                        path: job.destination.to_path_buf(),
                        problem: format!("refused the session ({e})"),
                    })
                }
                Err(e) => Err(e.into()),
            }
        }
        SaveState::Copying { staging } => {
            // Whatever a crash left here is this session's partial copy.
            if exists(staging) {
                std::fs::remove_dir_all(staging)?;
            }
            check_destination(job.destination)?;
            if let Err(e) = stage::copy_verified(job.working, staging, LEFT_BEHIND) {
                let _ = std::fs::remove_dir_all(staging);
                return Err(e.into());
            }
            fsync::sync_dir(job.destination)?;
            Ok(SaveState::Staged {
                staging: staging.clone(),
                copied: true,
            })
        }
        SaveState::Staged { staging, copied } => {
            let root = staging.parent().unwrap_or(job.destination);
            if !exists(staging) {
                // The final rename happened and its record did not: the
                // marker, kept through the rename, finds the folder.
                if let Some(path) = (1..=MAX_NAMES)
                    .map(|attempt| root.join(naming::candidate(job.folder_name, attempt)))
                    .find(|path| is_ours(path, job.session_id))
                {
                    return Ok(SaveState::Saved {
                        path,
                        working_retained: *copied,
                    });
                }
                if *copied && exists(job.working) {
                    return Ok(SaveState::Working);
                }
                return Err(SaveError::Lost(format!(
                    "the staged copy at {} is gone",
                    staging.display()
                )));
            }
            for name in LEFT_BEHIND {
                match std::fs::remove_dir_all(staging.join(name)) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
            }
            fsync::sync_dir(staging)?;
            // The final name goes beside the staged copy, a same-volume
            // rename, even if the destination setting has changed since it
            // was staged: that copy is the complete one (R22b). The marker
            // goes with it and is removed once `Saved` is recorded.
            for attempt in 1..=MAX_NAMES {
                let target = root.join(naming::candidate(job.folder_name, attempt));
                match fs.rename_new(staging, &target) {
                    Ok(()) => {
                        fsync::sync_dir(root)?;
                        return Ok(SaveState::Saved {
                            path: target,
                            working_retained: *copied,
                        });
                    }
                    Err(e) if rename::is_taken(&e) => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            Err(SaveError::Destination {
                path: job.destination.to_path_buf(),
                problem: format!(
                    "has no free name for {} after {MAX_NAMES} tries",
                    job.folder_name
                ),
            })
        }
        SaveState::Saved {
            path,
            working_retained,
        } => {
            match std::fs::remove_file(path.join(MARKER)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
            fsync::sync_dir(path)?;
            if *working_retained && exists(job.working) {
                std::fs::remove_dir_all(job.working)?;
            }
            Ok(SaveState::Done { path: path.clone() })
        }
        SaveState::Done { path } => Ok(SaveState::Done { path: path.clone() }),
    }
}
