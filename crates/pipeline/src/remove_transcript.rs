//! Transcript removal (U4 of the transcript removal plan): the step that
//! permanently deletes the transcript of one saved note and any audio that
//! is left, and the eligibility check that the removal pass and the count
//! before a setting change share.
//!
//! The step keeps this order (KTD3):
//!
//! 1. It removes the audio of each segment with the audio step.
//! 2. Only when no segment has audio left, it writes `transcript.removed`.
//!    Otherwise it stops, and the transcript stays.
//! 3. It deletes the files of KTD4: `transcript.md`, `recognition.jsonl`,
//!    the sync conflict copies of both, and the `.longhand-results` copies.
//! 4. Under the store lock, it removes the transcript fields from
//!    `longhand.json`.
//! 5. It clears the transcript data in the store, sets the store flag, and
//!    then runs a WAL checkpoint (KTD5).
//!
//! The marker makes the transcript removed. A crash after (2) leaves a
//! state that the next run on any computer finishes with
//! [`finish_transcript_removal`]. A repeated run changes nothing. The step
//! deletes only regular files with a fixed name, and it reads no name from
//! a synced file. The delete does not move a file to the Trash (R8).

use std::io;
use std::path::{Path, PathBuf};

use chrono::TimeDelta;
use session_core::folder::{self, SummaryMeta, TRANSCRIPT_REMOVED_MARKER, TRANSCRIPT_REMOVED_TEXT};
use session_core::jobs::Claims;
use session_core::note_folder::{Layout, NoteFiles, READ_TIMEOUT};
use session_core::note_record::NoteRecord;
use session_core::recognition::ARTIFACT_FILE;
use session_core::{SessionRecord, SessionState, Stage, Store, StoreError};
use session_save::{home_write, note_files};

use crate::join::TRANSCRIPT_FILE;
use crate::remove_audio::{
    EligibilityInputs, FileDelete, Guard, Guarded, RemoveAudioError, Scope, SegmentOutcome,
    SegmentRemoval, delete_claim, guarded_note, record_flags, remove_regular_file, remove_segments,
};
use crate::remove_copies::{
    Cleanup, clean_results_in, delete_in_results, files_in_results, reachable, write_leftover,
};
use crate::summarize::SUMMARY_FILE;
use crate::{SharedStore, lock};

/// The result of the step for one note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptRemoval {
    /// What the step did to the audio of each segment. Empty when the
    /// marker was there before the step.
    pub audio: Vec<SegmentRemoval>,
    /// The files and folders that the step deleted, and the entries with a
    /// fixed name that it did not delete.
    pub files: Cleanup,
    /// The note has the transcript marker at the end of the step.
    pub removed: bool,
    /// The row of the note changed: the caller sends a row change.
    pub row_changed: bool,
}

/// Why the step stopped.
#[derive(Debug, thiserror::Error)]
pub enum RemoveTranscriptError {
    /// The note is not saved, other work holds a claim on it, or the guard
    /// of the caller refused. The text names the reason. The step changed
    /// no file.
    #[error("the note is busy: {0}")]
    Busy(&'static str),
    /// The note has no home folder, or its home folder or storage folder is
    /// not available.
    #[error("the note's folder is not available")]
    NoFolder,
    /// The audio of a segment is not removed. The step wrote no transcript
    /// marker, and the transcript stays (R4). Each entry tells what the
    /// audio step did.
    #[error("the audio of the note could not be removed")]
    AudioLeft(Vec<SegmentRemoval>),
    #[error("{0}")]
    Failed(String),
}

impl From<RemoveAudioError> for RemoveTranscriptError {
    fn from(e: RemoveAudioError) -> Self {
        match e {
            RemoveAudioError::Busy(reason) => RemoveTranscriptError::Busy(reason),
            RemoveAudioError::NoFolder => RemoveTranscriptError::NoFolder,
            RemoveAudioError::Failed(text) => RemoveTranscriptError::Failed(text),
        }
    }
}

impl From<StoreError> for RemoveTranscriptError {
    fn from(e: StoreError) -> Self {
        RemoveTranscriptError::Failed(e.to_string())
    }
}

/// Remove the transcript of the saved note `note_id` and any audio that is
/// left: [`remove_transcript_with`] with a permanent delete.
pub fn remove_transcript<F, S>(
    store: &SharedStore,
    claims: &Claims,
    note_id: &str,
    guard: Guard<F, S>,
) -> Result<TranscriptRemoval, RemoveTranscriptError>
where
    F: FnOnce(&SessionRecord) -> Result<(), &'static str>,
    S: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
{
    remove_transcript_with(store, claims, note_id, guard, |path| {
        std::fs::remove_file(path)
    })
}

/// Remove the transcript of the saved note `note_id` and any audio that is
/// left, and delete each file with `remove`. See the module documentation.
///
/// The claim, the check that the note is saved, `guard` and the audio
/// deletes are the same as in
/// [`crate::remove_audio::remove_audio_with`]. A note in the old layout
/// waits for its conversion: the step changes nothing (KTD6 of the note
/// layout plan).
pub fn remove_transcript_with<F, S, R>(
    store: &SharedStore,
    claims: &Claims,
    note_id: &str,
    guard: Guard<F, S>,
    remove: R,
) -> Result<TranscriptRemoval, RemoveTranscriptError>
where
    F: FnOnce(&SessionRecord) -> Result<(), &'static str>,
    S: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
    R: FnMut(&Path) -> io::Result<()>,
{
    run_step(store, claims, note_id, guard, remove, Mode::Remove)
}

/// Finish the removal of the transcript of the saved note `note_id`
/// (KTD3): when the note has the transcript marker, do steps (3) to (5).
/// The step removes no audio and writes no marker, so a note with no
/// marker keeps its transcript. The removal pass does this whether the
/// setting is on or off. The claim, the check that the note is saved and
/// `guard` are the same as in [`remove_transcript_with`].
pub fn finish_transcript_removal<F, S>(
    store: &SharedStore,
    claims: &Claims,
    note_id: &str,
    guard: Guard<F, S>,
) -> Result<TranscriptRemoval, RemoveTranscriptError>
where
    F: FnOnce(&SessionRecord) -> Result<(), &'static str>,
    S: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
{
    run_step(
        store,
        claims,
        note_id,
        guard,
        |path| std::fs::remove_file(path),
        Mode::Finish,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Remove the audio, write the marker, then finish.
    Remove,
    /// Finish only when the marker is there.
    Finish,
}

fn run_step<F, S, R>(
    store: &SharedStore,
    claims: &Claims,
    note_id: &str,
    guard: Guard<F, S>,
    mut remove: R,
    mode: Mode,
) -> Result<TranscriptRemoval, RemoveTranscriptError>
where
    F: FnOnce(&SessionRecord) -> Result<(), &'static str>,
    S: FnOnce(&Store, &SessionRecord) -> Result<(), &'static str>,
    R: FnMut(&Path) -> io::Result<()>,
{
    // The claim comes before the home folder is read, as in
    // `session_core::jobs::delete_session`.
    let _claim = delete_claim(claims, note_id)?;
    let Guarded {
        home,
        segments,
        refreshed,
    } = guarded_note(store, note_id, guard)?;
    let mut result = TranscriptRemoval {
        row_changed: refreshed,
        ..TranscriptRemoval::default()
    };

    if !folder::transcript_removed(&home) {
        if mode == Mode::Finish {
            return Ok(result);
        }
        // The records of a note in the old layout are in `session.md`, so
        // the step cannot clear them. It changes nothing before the
        // conversion.
        if read_files(&home)?.layout == Layout::Old {
            return Err(RemoveTranscriptError::Busy("converting"));
        }
        // (1) A note never keeps audio after its transcript is gone (R4).
        result.audio = remove_segments(&home, &segments, Scope::Every, &mut remove);
        result.row_changed |= record_flags(&lock(store), note_id, &home, &result.audio)?;
        let audio_gone = result.audio.iter().all(|s| {
            matches!(
                s.outcome,
                SegmentOutcome::Removed | SegmentOutcome::AlreadyRemoved
            )
        });
        if !audio_gone {
            return Err(RemoveTranscriptError::AudioLeft(result.audio));
        }
        // (2) The marker makes the removal. It comes before the first
        // delete of a transcript file.
        reachable(&home)?;
        home_write::write_atomic(
            &home,
            TRANSCRIPT_REMOVED_MARKER,
            TRANSCRIPT_REMOVED_TEXT.as_bytes(),
        )
        .map_err(|e| {
            RemoveTranscriptError::Failed(format!("cannot write the transcript marker: {e}"))
        })?;
    }
    result.removed = true;

    // (3) The files with the transcript text, also the copies in the
    // results folder of older builds. The other copies go only when
    // `summary.md` is there, because a copy can hold the only summary (R14).
    delete_files(&home, &mut remove, &mut result.files)?;
    delete_in_results(&home, &mut remove, &mut result.files, transcript_file)?;
    if home
        .join(SUMMARY_FILE)
        .metadata()
        .is_ok_and(|meta| meta.is_file())
    {
        clean_results_in(&home, &mut remove, &mut result.files)?;
    }

    // (4) `longhand.json` last, with the store lock held from the read to
    // the write, so that other writers on this computer do not undo it.
    let mut store = lock(store);
    clear_record(&home)?;
    // (5) The store keeps no transcript data. The flag comes after the
    // clears, so that the flag is set only on a row with no transcript
    // data.
    store.clear_recognition_data(note_id)?;
    store.clear_transcript_index(note_id)?;
    result.row_changed |= store.set_transcript_removed(note_id, true)?;
    if !store.checkpoint_wal()? {
        log::warn!(
            "transcript removal: the store WAL can still hold the removed transcript of {note_id}"
        );
    }
    Ok(result)
}

/// The note files in the canonical home folder `home`.
fn read_files(home: &Path) -> Result<NoteFiles, RemoveTranscriptError> {
    NoteFiles::read_timed(home, READ_TIMEOUT)
        .map_err(|e| RemoveTranscriptError::Failed(format!("cannot read the note's records: {e}")))
}

/// Whether `name` is a file that holds transcript text (KTD4):
/// `transcript.md`, `recognition.jsonl`, a sync conflict copy of one of
/// them, or a write leftover of one of them. The names compare with their
/// case.
fn transcript_file(name: &str) -> bool {
    [TRANSCRIPT_FILE, ARTIFACT_FILE].into_iter().any(|file| {
        name == file || library::conflict::copy_of(name, file) || write_leftover(name, file)
    })
}

/// The regular files in the canonical home folder `home` that
/// [`transcript_file`] names. This reads metadata only.
fn transcript_files(home: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(home)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if transcript_file(&name) {
            files.push(home.join(name));
        }
    }
    files.sort();
    Ok(files)
}

/// Delete the files of [`transcript_files`] in the canonical home folder
/// `home` with `remove`. An entry that is not a regular file stays, and the
/// step logs it.
fn delete_files<R>(
    home: &Path,
    remove: &mut R,
    cleanup: &mut Cleanup,
) -> Result<(), RemoveTranscriptError>
where
    R: FnMut(&Path) -> io::Result<()>,
{
    let files = transcript_files(home).map_err(|e| {
        RemoveTranscriptError::Failed(format!("cannot list {}: {e}", home.display()))
    })?;
    for path in files {
        reachable(home)?;
        match remove_regular_file(&path, remove) {
            FileDelete::Deleted => cleanup.deleted.push(path),
            FileDelete::Kept => cleanup.kept.push(path),
            FileDelete::Missing => {}
        }
    }
    Ok(())
}

/// Whether `record` has a field of KTD4.
fn has_transcript_fields(record: &NoteRecord) -> bool {
    record.transcript_fingerprint.is_some()
        || record.recognition_snapshot.is_some()
        || record.recognition_artifact.is_some()
        || record.recognition_audio.is_some()
}

/// Remove the fields of KTD4 from `longhand.json` in the canonical home
/// folder `home`. The found speaker count stays. A file with no such field
/// is not written. The caller holds the store lock.
fn clear_record(home: &Path) -> Result<(), RemoveTranscriptError> {
    let files = read_files(home)?;
    if files.layout == Layout::Old {
        // The conversion writes the records. The next pass then clears
        // them.
        log::info!(
            "transcript removal: {} waits for its conversion",
            home.display()
        );
        return Ok(());
    }
    let mut record = files.record;
    if !has_transcript_fields(&record) {
        return Ok(());
    }
    record.transcript_fingerprint = None;
    record.recognition_snapshot = None;
    record.recognition_artifact = None;
    record.recognition_audio = None;
    note_files::write_record(home, &record).map_err(|e| {
        RemoveTranscriptError::Failed(format!("cannot write the note's records: {e}"))
    })?;
    Ok(())
}

/// Whether the note in the home folder `home` needs
/// [`finish_transcript_removal`] (KTD3): it has the transcript marker, and
/// a file of KTD4 is still there, or `flag` (the store flag of this
/// computer) is not set, or `longhand.json` still has a field of KTD4. The
/// check reads metadata first, and opens `longhand.json` only for a note
/// with the marker.
pub fn finish_due(home: &Path, flag: bool) -> bool {
    if !folder::transcript_removed(home) {
        return false;
    }
    if !flag || !files_in_results(home, transcript_file).is_empty() {
        return true;
    }
    let file_left = transcript_files(home).is_ok_and(|files| {
        files
            .iter()
            .any(|path| path.symlink_metadata().is_ok_and(|meta| meta.is_file()))
    });
    file_left
        || NoteFiles::read_timed(home, READ_TIMEOUT)
            .is_ok_and(|files| files.layout == Layout::New && has_transcript_fields(&files.record))
}

/// Whether `longhand.json` in the home folder `home` records the summary
/// stub that a failed summary leaves (R28). A note that another computer
/// saved has no failed-summary row in this computer's store, so a caller
/// can add this check to the folder check of its [`Guard`]. A file that
/// cannot be read counts as no stub.
pub fn summary_stub_in(home: &Path) -> bool {
    NoteFiles::read_timed(home, READ_TIMEOUT)
        .is_ok_and(|files| matches!(files.record.summary, Some(SummaryMeta::Stub { .. })))
}

// ---------------------------------------------------------------- eligibility

/// The result of the transcript eligibility check for one note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptEligibility {
    /// The removal of the note's transcript can start now.
    Eligible,
    /// The note is not saved, or has no home folder (R10).
    NotSaved,
    /// Other work holds a claim on the note.
    Claimed,
    /// A run on this computer or another one resumes or processes the note.
    InUse,
    /// The note has no segment, so it has no last recording end (R6).
    NoRecording,
    /// The transcript of the note is already removed.
    NoTranscript,
    /// The delay has passed, and the summary of the note is the stub that
    /// a failed summary leaves (R28). The transcript is the only record of
    /// the meeting.
    SummaryFailed,
    /// The delay has not passed. The removal can start at `due`.
    NotYet {
        due: chrono::DateTime<chrono::FixedOffset>,
    },
}

/// Whether the removal of the transcript of the note `note_id` can start
/// (R6, R10, R28). `inputs.delay_days` is the effective delay of KTD7. The
/// last recording end is the latest segment `wall_end`. The check uses the
/// store only and opens no file. The step checks the state again after it
/// takes its claim.
///
/// The failed-summary row is only in the store of the computer that saved
/// the note, so a note that another computer saved shows no stub here. See
/// [`summary_stub_in`].
pub fn transcript_eligibility(
    store: &Store,
    note_id: &str,
    inputs: &EligibilityInputs,
) -> Result<TranscriptEligibility, StoreError> {
    let record = store
        .session(note_id)?
        .ok_or_else(|| StoreError::NoSuchSession(note_id.to_string()))?;
    if record.state != SessionState::Saved || record.home_path.is_none() {
        return Ok(TranscriptEligibility::NotSaved);
    }
    if inputs.claimed {
        return Ok(TranscriptEligibility::Claimed);
    }
    if inputs.in_use {
        return Ok(TranscriptEligibility::InUse);
    }
    let segments = store.segments(note_id)?;
    let Some(end) = segments.iter().map(|s| s.wall_end).max() else {
        return Ok(TranscriptEligibility::NoRecording);
    };
    if store.transcript_removed(note_id)? {
        return Ok(TranscriptEligibility::NoTranscript);
    }
    let due = end + TimeDelta::days(i64::from(inputs.delay_days));
    if due > inputs.now {
        return Ok(TranscriptEligibility::NotYet { due });
    }
    if store
        .checkpoint_value(note_id, Stage::Summarize, crate::save::SUMMARY_FAILED)?
        .is_some()
    {
        return Ok(TranscriptEligibility::SummaryFailed);
    }
    Ok(TranscriptEligibility::Eligible)
}
