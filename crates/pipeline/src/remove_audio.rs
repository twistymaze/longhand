//! Audio removal (U3 of the audio removal plan): the step that permanently
//! deletes the audio of one saved note, and the eligibility check that the
//! removal pass and the count before a setting change share.
//!
//! For each segment, the step writes the removal marker first and then
//! deletes the audio file (KTD2). The marker makes the audio of the segment
//! removed, so a crash between the two leaves a state that the next run
//! finishes. A repeated run changes nothing. The step deletes only a file
//! with an audio name that Longhand writes, in the audio folder of the home
//! folder (R7, KTD10). The delete does not
//! move the file to the Trash (R6), and it opens no audio file, so an
//! online-only file is not downloaded (R12, KTD11).
//!
//! When the audio of each segment is removed, the step also deletes the
//! note's `recognition.jsonl`, unless the note waits for a sync (KTD8 of the
//! result copies plan). It does not write `session.md`.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, TimeDelta};
use session_core::folder::{self, REMOVAL_MARKER_TEXT};
use session_core::jobs::{Claim, Claims};
use session_core::recognition::ARTIFACT_FILE;
use session_core::{SessionRecord, SessionState, Store, StoreError};
use session_save::home_write;

use crate::{SharedStore, StageError, lock};

/// What the step did to the audio of one segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentOutcome {
    /// This run wrote the marker, deleted the audio file, or both.
    Removed,
    /// The marker was there and the audio file was not: nothing to do.
    AlreadyRemoved,
    /// The audio name does not have the shape that Longhand writes, it does
    /// not resolve inside the audio folder of the home folder, or it names
    /// a folder (KTD10). The step wrote no marker and deleted nothing.
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

/// The folder of the audio files in a note folder.
const AUDIO_FOLDER: &str = "audio";

/// Whether `audio_file` has the shape of an audio name that Longhand writes
/// (KTD10): `audio/segment-<digits>.opus`. Every other name is not safe, so
/// a `session.md` from a sync cannot name a document, a conflict copy or a
/// file outside the audio folder. The file system can ignore case, so a
/// check that only refuses the names of the documents is not sufficient.
/// This check uses the name only. The step also checks that the name
/// resolves inside the audio folder of the home folder.
pub fn safe_audio_name(audio_file: &str) -> bool {
    let Some((folder, file)) = audio_file.split_once('/') else {
        return false;
    };
    let Some(number) = file
        .strip_prefix("segment-")
        .and_then(|rest| rest.strip_suffix(".opus"))
    else {
        return false;
    };
    folder == AUDIO_FOLDER && !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
}

/// The checks that the caller adds to the step. An error from a check stops
/// the step, and its text names the reason.
///
/// `folder` checks the files of the note, for example its in-use markers.
/// The step calls it with the row before it takes the store lock, so a slow
/// or online-only folder does not stop other users of the store. `store`
/// checks the store only. The step calls it under the store lock with the
/// row that the step brought up to date. When both checks refuse, the
/// reason of `store` is the result.
pub struct Guard<F, S> {
    pub folder: F,
    pub store: S,
}

/// A folder check of a [`Guard`] that captures no value.
pub type FolderCheck = fn(&SessionRecord) -> Result<(), &'static str>;

/// A store check of a [`Guard`] that captures no value.
pub type StoreCheck = fn(&Store, &SessionRecord) -> Result<(), &'static str>;

/// A [`Guard`] whose checks all pass.
pub fn no_guard() -> Guard<FolderCheck, StoreCheck> {
    Guard {
        folder: |_| Ok(()),
        store: |_, _| Ok(()),
    }
}

/// Whether the note waits for a sync: its `transcript.md` or `summary.md`
/// does not match `session.md` (R7). This controls only the delete of
/// `recognition.jsonl` (KTD8). The removal of the audio does not wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteSync {
    Synced,
    Waiting,
}

/// Remove the audio of the saved note `session_id`: [`remove_audio_with`]
/// with a permanent delete.
pub fn remove_audio<F, S>(
    store: &SharedStore,
    claims: &Claims,
    session_id: &str,
    guard: Guard<F, S>,
    sync: NoteSync,
) -> Result<Removal, RemoveAudioError>
where
    F: FnOnce(&SessionRecord) -> Result<(), &'static str>,
    S: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
{
    remove_audio_with(store, claims, session_id, guard, sync, |path| {
        std::fs::remove_file(path)
    })
}

/// Remove the audio of the saved note `session_id`, and delete each audio
/// file with `remove`.
///
/// The step takes a delete claim for the time it runs, as Delete Note does,
/// so a summary regeneration, a Move and a folder rename wait or refuse.
/// It reads the row under a short store lock, and calls the folder check of
/// `guard` with no lock. Then, under the store lock, it brings the row up
/// to date from the home folder, as other writers do. It checks that the
/// note is saved and that its home folder did not change, and calls the
/// store check of `guard`. The step does not hold the store lock while it
/// writes markers and deletes files. When the audio of each segment is
/// removed, it also deletes `recognition.jsonl` with `remove`, unless
/// `sync` is [`NoteSync::Waiting`] (KTD8). At the end it sets the removed
/// flag of each segment that has its marker.
pub fn remove_audio_with<F, S, R>(
    store: &SharedStore,
    claims: &Claims,
    session_id: &str,
    guard: Guard<F, S>,
    sync: NoteSync,
    remove: R,
) -> Result<Removal, RemoveAudioError>
where
    F: FnOnce(&SessionRecord) -> Result<(), &'static str>,
    S: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
    R: FnMut(&Path) -> io::Result<()>,
{
    run_step(store, claims, session_id, guard, sync, remove, Scope::Every)
}

/// Finish the removal of the audio of the saved note `session_id` (KTD2):
/// delete the audio file of each segment that has its removal marker. The
/// step writes no marker, so a segment with no marker keeps its audio. The
/// removal pass does this whether the setting is on or off. The claim, the
/// check that the note is saved, `guard`, the name check, the delete of
/// `recognition.jsonl` and the flags are the same as in
/// [`remove_audio_with`]. The result has an entry only for each segment
/// that has its marker.
pub fn finish_removal<F, S>(
    store: &SharedStore,
    claims: &Claims,
    session_id: &str,
    guard: Guard<F, S>,
    sync: NoteSync,
) -> Result<Removal, RemoveAudioError>
where
    F: FnOnce(&SessionRecord) -> Result<(), &'static str>,
    S: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
{
    run_step(
        store,
        claims,
        session_id,
        guard,
        sync,
        |path| std::fs::remove_file(path),
        Scope::Marked,
    )
}

/// The segments that the step works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Each segment: write the marker if it is missing, then delete the
    /// audio file.
    Every,
    /// Only each segment that has its marker: delete the audio file.
    Marked,
}

/// The row of `session_id`. The note must be saved.
pub(crate) fn saved_row(
    store: &Store,
    session_id: &str,
) -> Result<SessionRecord, RemoveAudioError> {
    let record = store
        .session(session_id)?
        .ok_or_else(|| RemoveAudioError::Failed(format!("no session {session_id}")))?;
    if record.state != SessionState::Saved {
        return Err(RemoveAudioError::Busy(record.state.name()));
    }
    Ok(record)
}

/// Take the delete claim of `session_id`, as Delete Note does.
pub(crate) fn delete_claim(claims: &Claims, session_id: &str) -> Result<Claim, RemoveAudioError> {
    // A second delete claim is not refused by `Claims::delete`, so a
    // running Delete Note is checked first.
    if claims.is_regenerating_or_deleting(session_id) && !claims.is_regenerating(session_id) {
        return Err(RemoveAudioError::Busy("deleting"));
    }
    claims.delete(session_id).map_err(RemoveAudioError::Busy)
}

fn run_step<F, S, R>(
    store: &SharedStore,
    claims: &Claims,
    session_id: &str,
    guard: Guard<F, S>,
    sync: NoteSync,
    mut remove: R,
    scope: Scope,
) -> Result<Removal, RemoveAudioError>
where
    F: FnOnce(&SessionRecord) -> Result<(), &'static str>,
    S: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
    R: FnMut(&Path) -> io::Result<()>,
{
    // The claim comes before the home folder is read, as in
    // `session_core::jobs::delete_session`.
    let _claim = delete_claim(claims, session_id)?;

    // The folder check reads files, so it runs with no store lock.
    let checked = saved_row(&lock(store), session_id)?;
    let folder_check = (guard.folder)(&checked);

    let (home, segments, refreshed) = {
        let mut store = lock(store);
        let refreshed = crate::follow::follow_folder(&mut store, session_id, false)?;
        let record = saved_row(&store, session_id)?;
        // The claim stops a Move and a folder rename. If the home folder
        // changed all the same, the result of the folder check is not
        // valid.
        if record.home_path != checked.home_path {
            return Err(RemoveAudioError::Busy("moving"));
        }
        (guard.store)(&store, &record).map_err(RemoveAudioError::Busy)?;
        folder_check.map_err(RemoveAudioError::Busy)?;
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
        .filter(|segment| match scope {
            Scope::Every => true,
            // A name that is not safe is not read, not even for a marker.
            Scope::Marked => {
                safe_audio_name(&segment.audio_file)
                    && folder::audio_removed(&home, &segment.audio_file)
            }
        })
        .map(|segment| SegmentRemoval {
            audio_file: segment.audio_file.clone(),
            outcome: remove_segment(&home, &segment.audio_file, &mut remove),
        })
        .collect();

    // The removal is complete when the audio of each segment of the note is
    // removed. `recognition.jsonl` has no use without the audio (KTD8).
    let complete = !segments.is_empty()
        && results.len() == segments.len()
        && results.iter().all(|r| {
            matches!(
                r.outcome,
                SegmentOutcome::Removed | SegmentOutcome::AlreadyRemoved
            )
        });
    if complete {
        match sync {
            NoteSync::Synced => {
                remove_recognition(&home, &mut remove);
            }
            // A later pass deletes it.
            NoteSync::Waiting => log::debug!(
                "audio removal: {session_id} waits for a sync, so {ARTIFACT_FILE} stays for now"
            ),
        }
    }

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

/// What a delete of one file with a fixed name did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileDelete {
    Deleted,
    /// No entry has the name.
    Missing,
    /// The entry is not a regular file, or the delete failed. The entry
    /// stays, and the delete logs it.
    Kept,
}

/// Delete `recognition.jsonl` in the canonical home folder `home` with
/// `remove` when it is a regular file (R11). A folder or a link with that
/// name stays, and the step logs it. The check reads metadata only, so an
/// online-only file is not downloaded (R12). A failed delete does not
/// change the result of the removal.
pub(crate) fn remove_recognition<R>(home: &Path, remove: &mut R) -> FileDelete
where
    R: FnMut(&Path) -> io::Result<()>,
{
    remove_regular_file(&home.join(ARTIFACT_FILE), remove)
}

/// Delete `path` with `remove` when it is a regular file. A folder or a
/// link with that name stays, and the delete logs it. The check reads
/// metadata only (R12).
pub(crate) fn remove_regular_file<R>(path: &Path, remove: &mut R) -> FileDelete
where
    R: FnMut(&Path) -> io::Result<()>,
{
    match path.symlink_metadata() {
        Ok(meta) if meta.is_file() => match remove(path) {
            Ok(()) => FileDelete::Deleted,
            Err(e) if e.kind() == io::ErrorKind::NotFound => FileDelete::Missing,
            Err(e) => {
                log::warn!("audio removal: cannot delete {}: {e}", path.display());
                FileDelete::Kept
            }
        },
        Ok(_) => {
            log::warn!(
                "audio removal: {} is not a regular file, so it stays",
                path.display()
            );
            FileDelete::Kept
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => FileDelete::Missing,
        Err(e) => {
            log::warn!("audio removal: cannot read {}: {e}", path.display());
            FileDelete::Kept
        }
    }
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
    // The audio folder must be a real folder in the home folder. A link,
    // for example `audio -> .`, resolves to a different path.
    let dir = match dir.canonicalize() {
        Ok(dir) if dir == home.join(AUDIO_FOLDER) => dir,
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
    // Metadata only: an online-only file is not downloaded (R12).
    let entry = path.symlink_metadata().ok();
    // The step deletes files only, never a folder.
    if entry.as_ref().is_some_and(|meta| meta.is_dir()) {
        return SegmentOutcome::UnsafeName;
    }

    let mut wrote = false;
    if !dir.join(&marker).try_exists().unwrap_or(false) {
        if let Err(e) = home_write::write_atomic(&dir, &marker, REMOVAL_MARKER_TEXT.as_bytes()) {
            return SegmentOutcome::Failed(format!("cannot write the marker of {audio}: {e}"));
        }
        wrote = true;
    }
    if entry.is_some() {
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
