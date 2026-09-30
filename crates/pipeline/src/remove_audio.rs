//! Audio removal (U3 of the audio removal plan): the step that permanently
//! deletes the audio of one saved note, and the eligibility check that the
//! removal pass and the count before a setting change share.
//!
//! For each segment, the step writes the removal marker first and then
//! deletes the audio file (KTD2). The marker makes the audio of the segment
//! removed, so a crash between the two leaves a state that the next run
//! finishes. A repeated run changes nothing. The step deletes only a plain
//! relative name inside the home folder (R7, KTD10). The delete does not
//! move the file to the Trash (R6), and it opens no audio file, so an
//! online-only file is not downloaded (R12, KTD11).

use std::io;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, FixedOffset, TimeDelta};
use session_core::folder::{self, REMOVAL_MARKER_TEXT};
use session_core::jobs::Claims;
use session_core::{SessionRecord, SessionState, Store, StoreError};
use session_save::home_write;

use crate::{SharedStore, StageError, lock};

/// The documents of a note folder. A segment audio name that is one of
/// them is not audio, and the step does not delete it.
const DOCUMENTS: [&str; 4] = ["session.md", "transcript.md", "summary.md", "notes.md"];

/// What the step did to the audio of one segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentOutcome {
    /// This run wrote the marker, deleted the audio file, or both.
    Removed,
    /// The marker was there and the audio file was not: nothing to do.
    AlreadyRemoved,
    /// The audio name is not a plain relative name inside the home folder
    /// (KTD10). The step wrote no marker and deleted nothing.
    UnsafeName,
    /// The marker write or the delete failed. A marker that the step wrote
    /// stays, so the audio counts as removed, and the next run tries the
    /// delete again.
    Failed(String),
}

/// The result of the step for one segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRemoval {
    /// The audio name from the segment, relative to the home folder.
    pub audio_file: String,
    pub outcome: SegmentOutcome,
}

/// The result of the step for one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// One entry for each segment, in the order of the segments.
    pub segments: Vec<SegmentRemoval>,
    /// The row of the note changed: the caller sends a row change.
    pub row_changed: bool,
}

impl Removal {
    /// Whether every segment has its marker and no audio file.
    pub fn complete(&self) -> bool {
        self.segments.iter().all(|s| {
            matches!(
                s.outcome,
                SegmentOutcome::Removed | SegmentOutcome::AlreadyRemoved
            )
        })
    }
}

/// Why the step did not start. It changed no file.
#[derive(Debug, thiserror::Error)]
pub enum RemoveAudioError {
    /// The note is not saved, other work holds a claim on it, or the guard
    /// of the caller refused. The text names the reason.
    #[error("the note is busy: {0}")]
    Busy(&'static str),
    /// The note has no home folder, or its home folder is not there.
    #[error("the note's folder is not available")]
    NoFolder,
    #[error("{0}")]
    Failed(String),
}

impl From<StageError> for RemoveAudioError {
    fn from(e: StageError) -> Self {
        RemoveAudioError::Failed(e.to_string())
    }
}

impl From<StoreError> for RemoveAudioError {
    fn from(e: StoreError) -> Self {
        RemoveAudioError::Failed(e.to_string())
    }
}

/// Whether `audio_file` is a plain relative name (KTD10): it is not empty,
/// not absolute, has no `.` or `..` part, and is not a document of the
/// note. This check uses the name only. The step also checks that the name
/// resolves inside the home folder.
pub fn safe_audio_name(audio_file: &str) -> bool {
    if audio_file.is_empty() || audio_file.ends_with('/') || audio_file.ends_with('\\') {
        return false;
    }
    if DOCUMENTS.contains(&audio_file) {
        return false;
    }
    let path = Path::new(audio_file);
    !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// Remove the audio of the saved note `session_id`: [`remove_audio_with`]
/// with a permanent delete.
pub fn remove_audio<G>(
    store: &SharedStore,
    claims: &Claims,
    session_id: &str,
    guard: G,
) -> Result<Removal, RemoveAudioError>
where
    G: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
{
    remove_audio_with(store, claims, session_id, guard, |path| {
        std::fs::remove_file(path)
    })
}

/// Remove the audio of the saved note `session_id`, and delete each audio
/// file with `remove`.
///
/// The step takes a delete claim for the time it runs, as Delete Note does,
/// so a summary regeneration, a Move and a folder rename wait or refuse.
/// Then it brings the row up to date from the home folder, as other writers
/// do. Under the store lock it checks that the note is saved, and calls
/// `guard` with the store and the row: the caller adds its own checks
/// there, and an error from `guard` stops the step. The step does not hold
/// the store lock while it writes markers and deletes files. At the end it
/// sets the removed flag of each segment that has its marker.
pub fn remove_audio_with<G, R>(
    store: &SharedStore,
    claims: &Claims,
    session_id: &str,
    guard: G,
    mut remove: R,
) -> Result<Removal, RemoveAudioError>
where
    G: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
    R: FnMut(&Path) -> io::Result<()>,
{
    // A second delete claim is not refused by `Claims::delete`, so a
    // running Delete Note is checked first.
    if claims.is_regenerating_or_deleting(session_id) && !claims.is_regenerating(session_id) {
        return Err(RemoveAudioError::Busy("deleting"));
    }
    // The claim comes before the home folder is read, as in
    // `session_core::jobs::delete_session`.
    let _claim = claims.delete(session_id).map_err(RemoveAudioError::Busy)?;

    let (home, segments, refreshed) = {
        let mut store = lock(store);
        let refreshed = crate::follow::follow_folder(&mut store, session_id, false)?;
        let record = store
            .session(session_id)?
            .ok_or_else(|| RemoveAudioError::Failed(format!("no session {session_id}")))?;
        if record.state != SessionState::Saved {
            return Err(RemoveAudioError::Busy(record.state.name()));
        }
        guard(&store, &record).map_err(RemoveAudioError::Busy)?;
        let Some(home) = record.home_path.clone() else {
            return Err(RemoveAudioError::NoFolder);
        };
        (PathBuf::from(home), store.segments(session_id)?, refreshed)
    };
    // The step never makes a home folder. The canonical path is the base of
    // the check that a name resolves inside the home folder.
    let home = match home.canonicalize() {
        Ok(path) if path.is_dir() => path,
        _ => return Err(RemoveAudioError::NoFolder),
    };

    let results: Vec<SegmentRemoval> = segments
        .iter()
        .map(|segment| SegmentRemoval {
            audio_file: segment.audio_file.clone(),
            outcome: remove_segment(&home, &segment.audio_file, &mut remove),
        })
        .collect();

    let store = lock(store);
    let mut flags = store.removed_audio(session_id)?;
    flags.extend(
        results
            .iter()
            .filter(|r| r.outcome != SegmentOutcome::UnsafeName)
            .filter(|r| folder::audio_removed(&home, &r.audio_file))
            .map(|r| r.audio_file.clone()),
    );
    flags.sort();
    flags.dedup();
    let flags_changed = store.set_removed_audio(session_id, &flags)?;
    Ok(Removal {
        segments: results,
        row_changed: refreshed.changed || flags_changed,
    })
}

/// Write the marker of `audio` if it is missing, then delete the audio file
/// if it is there.
fn remove_segment<R>(home: &Path, audio: &str, remove: &mut R) -> SegmentOutcome
where
    R: FnMut(&Path) -> io::Result<()>,
{
    if !safe_audio_name(audio) {
        return SegmentOutcome::UnsafeName;
    }
    let path = home.join(audio);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return SegmentOutcome::UnsafeName;
    };
    let dir = match dir.canonicalize() {
        Ok(dir) if dir.starts_with(home) => dir,
        Ok(_) => return SegmentOutcome::UnsafeName,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return SegmentOutcome::Failed(format!(
                "the folder of {audio} is missing, so its marker cannot be written"
            ));
        }
        Err(e) => return SegmentOutcome::Failed(format!("cannot read the folder of {audio}: {e}")),
    };
    let path = dir.join(name);
    let marker = folder::removal_marker(name);

    let mut wrote = false;
    if !dir.join(&marker).try_exists().unwrap_or(false) {
        if let Err(e) = home_write::write_atomic(&dir, &marker, REMOVAL_MARKER_TEXT.as_bytes()) {
            return SegmentOutcome::Failed(format!("cannot write the marker of {audio}: {e}"));
        }
        wrote = true;
    }
    // Metadata only: an online-only file is not downloaded (R12).
    let present = path.symlink_metadata().is_ok();
    if present {
        match remove(&path) {
            Ok(()) => return SegmentOutcome::Removed,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return SegmentOutcome::Failed(format!("cannot delete {audio}: {e}")),
        }
    }
    if wrote {
        SegmentOutcome::Removed
    } else {
        SegmentOutcome::AlreadyRemoved
    }
}

// ---------------------------------------------------------------- eligibility

/// The inputs of the eligibility check that do not come from the store.
/// The removal pass (U6) and the count before a setting change (R17) give
/// the same inputs. Later guards (an open Move, the storage folder) add
/// their fields here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibilityInputs {
    /// The time of the check.
    pub now: DateTime<FixedOffset>,
    /// The delay of the setting, in whole days (R1).
    pub delay_days: u32,
    /// Other work holds a claim on the note ([`Claims::busy`]).
    pub claimed: bool,
    /// The home folder has a fresh in-use marker, or cannot be read
    /// ([`crate::in_use::in_use`], R10, KTD4).
    pub in_use: bool,
}

/// The result of the eligibility check for one note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    /// The removal of the note's audio can start now.
    Eligible,
    /// The note is not saved, or has no home folder (R8).
    NotSaved,
    /// Other work holds a claim on the note (KTD5).
    Claimed,
    /// A run on this computer or another one resumes or processes the note
    /// (R10, KTD4).
    InUse,
    /// The note has no segment, so it has no last recording end (KTD7).
    NoRecording,
    /// Every segment of the note has the removed flag.
    NoAudio,
    /// The delay has not passed. The removal can start at `due`.
    NotYet { due: DateTime<FixedOffset> },
}

/// Whether the removal of the audio of the note `session_id` can start
/// (KTD5, KTD7). The last recording end is the latest segment `wall_end`.
/// The check uses the store only and opens no file (KTD11). The step
/// checks the state again after it takes its claim.
pub fn eligibility(
    store: &Store,
    session_id: &str,
    inputs: &EligibilityInputs,
) -> Result<Eligibility, StoreError> {
    let record = store
        .session(session_id)?
        .ok_or_else(|| StoreError::NoSuchSession(session_id.to_string()))?;
    if record.state != SessionState::Saved || record.home_path.is_none() {
        return Ok(Eligibility::NotSaved);
    }
    if inputs.claimed {
        return Ok(Eligibility::Claimed);
    }
    if inputs.in_use {
        return Ok(Eligibility::InUse);
    }
    let segments = store.segments(session_id)?;
    let Some(end) = segments.iter().map(|s| s.wall_end).max() else {
        return Ok(Eligibility::NoRecording);
    };
    let removed = store.removed_audio(session_id)?;
    if segments.iter().all(|s| removed.contains(&s.audio_file)) {
        return Ok(Eligibility::NoAudio);
    }
    let due = end + TimeDelta::days(i64::from(inputs.delay_days));
    if due > inputs.now {
        return Ok(Eligibility::NotYet { due });
    }
    Ok(Eligibility::Eligible)
}
