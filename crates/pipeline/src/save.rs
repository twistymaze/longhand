//! The save stage (U11, KD4, KD13, R22, R22e, R22f): render the session's
//! markdown beside its audio, then move it into the destination through the
//! `session_save` crate, recording each save state before the next step so a
//! crash resumes from what is on disk.
//!
//! Saving claims the session first, under the store lock: from then on a
//! resume is refused, since the working directory is about to leave. The
//! claim is released if saving fails before anything moved.

use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};

use library::refresh::Scope;
use library::scan::{READ_TIMEOUT, Read, read_note_files, read_timed};
pub use session_core::folder::AudioState;
use session_core::folder::{self, ResultFlag, SessionDoc, Summary, SummaryMeta};
use session_core::note_folder::{Layout, NoteFiles, SESSION_FILE};
use session_core::note_record::{NOTE_RECORD_FILE, NoteRecord};
use session_core::{Event, Segment, SessionRecord, SessionState, Stage, Store, TitleSource};
pub use session_save::publication::OLD_LAYOUT;
use session_save::publication::{Outcome, Publication, WriteSet};
use session_save::{Fs, Job, SaveError, SaveState, home_write, note_files};

use crate::folder_rename::{self, FolderRename};
use crate::join::TRANSCRIPT_FILE;
use crate::summarize::{self, Prompt, SUMMARY_FILE, SummaryEndpoint};
use crate::{Control, SharedStore, StageError, checkpoint, lock};

pub const NOTES_FILE: &str = "notes.md";
/// The save's progress, a [`SaveState`]. Its presence means saving has
/// begun and the session can no longer be resumed.
pub const STATE: &str = session_core::jobs::SAVE_STATE;
/// The destination folder name, a string, recorded with the claim so that
/// every later run of the save uses the same name (KTD7). A title that
/// changes during the save cannot then send the final rename, or the scan
/// for a folder whose rename was not recorded, to another name.
pub const FOLDER_NAME: &str = "folder_name";
/// Under the summarize stage: why it failed, for the saved stub (KD13).
pub const SUMMARY_FAILED: &str = "failed";
/// A replacement reached publication. Preserve its local files on failure.
/// Only the result fields of `session.md` prove that the replacement
/// committed.
pub const HOME_CHANGED: &str = "home_changed";
/// Under `result_state`: the local staging folder of a summary
/// regeneration. Its publication record is in that folder (KTD1).
const SUMMARY_PUBLICATION: &str = "summary-publication";
/// The error of a publication that another computer's result replaced
/// before this computer finished it (KTD4).
pub const CHANGED_ELSEWHERE: &str =
    "This note changed on another computer. Longhand kept that version.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: PathBuf,
    /// The destination is on another volume: the session was copied and
    /// verified rather than moved in one step.
    pub copied: bool,
}

/// Whether saving has begun for `session_id` (a resume must refuse).
pub fn begun(store: &Store, session_id: &str) -> Result<bool, StageError> {
    Ok(store
        .checkpoints(session_id, Stage::Save)?
        .iter()
        .any(|(key, _)| key == STATE))
}

/// Whether this save has begun to replace the documents in the session's
/// home folder.
pub fn home_changed(store: &Store, session_id: &str) -> Result<bool, StageError> {
    Ok(store
        .checkpoints(session_id, Stage::Save)?
        .iter()
        .any(|(key, _)| key == HOME_CHANGED))
}

/// Finish each summary regeneration that a crash stopped (R4). Only the
/// owner has the local records, so only the owner writes. A publication
/// that wrote a file rolls forward from its staging (KTD3). One that
/// another computer's result replaced is dropped (KTD4).
pub fn recover_summary_publications(store: &SharedStore) -> Result<(), StageError> {
    let mut guard = lock(store);
    for record in guard.sessions()? {
        if let Err(error) = recover_summary_publication(&mut guard, &record.id) {
            log::warn!(
                "summary publication recovery failed for {}: {error}",
                record.id
            );
        }
    }
    Ok(())
}

/// Finish or drop the summary publication of `session_id`, if it has one.
/// The staging and its row go when the publication ends. A staging with no
/// record wrote nothing into the note folder. The caller holds the store
/// lock.
fn recover_summary_publication(store: &mut Store, session_id: &str) -> Result<(), StageError> {
    let Some(value) = store.result_state(session_id, SUMMARY_PUBLICATION)? else {
        return Ok(());
    };
    let local: PathBuf = serde_json::from_value(value)?;
    if let Some(publication) = Publication::load(&local)? {
        let home = run_home(store, session_id)?;
        match publication.publish(&home, &local)? {
            Outcome::Committed => {
                log::info!("finished the summary publication of {session_id}");
                let segments = store.segments(session_id)?.len();
                summary_published(store, session_id, segments)?;
            }
            Outcome::Conflict => {
                log::warn!(
                    "{session_id}: {CHANGED_ELSEWHERE} The regenerated summary was dropped."
                );
            }
        }
    }
    drop_summary_staging(store, session_id, &local)
}

/// Delete the record and the staging of a summary regeneration, then its
/// row. The caller holds the store lock.
fn drop_summary_staging(store: &Store, session_id: &str, local: &Path) -> Result<(), StageError> {
    if local.exists() {
        Publication::remove(local)?;
        std::fs::remove_dir_all(local)?;
    }
    store.clear_result_state(session_id, SUMMARY_PUBLICATION)?;
    Ok(())
}

/// The bookkeeping after a regenerated summary is in the note folder.
fn summary_published(
    store: &mut Store,
    session_id: &str,
    segments: usize,
) -> Result<(), StageError> {
    store.put_checkpoint(
        session_id,
        Stage::Summarize,
        checkpoint::DONE,
        &serde_json::to_string(&checkpoint::StageDone { segments })?,
    )?;
    store.delete_checkpoint(session_id, Stage::Summarize, SUMMARY_FAILED)?;
    store.clear_summary_not_updated(session_id)?;
    store.apply(session_id, &Event::SavedEdit)?;
    Ok(())
}

/// The audio of a completed session, from its folder (KTD9 of the audio
/// removal plan). Resume and Transcribe Again need
/// [`AudioState::Present`] (R6d). A session that has not completed, or has
/// no segment, has no audio in a folder: its state is
/// [`AudioState::Waiting`].
pub fn audio_state(store: &Store, session_id: &str) -> Result<AudioState, StageError> {
    let Some(home) = home_path(store, session_id)? else {
        return Ok(AudioState::Waiting);
    };
    let segments = store.segments(session_id)?;
    if segments.is_empty() {
        return Ok(AudioState::Waiting);
    }
    Ok(folder::audio_state(
        &home,
        segments.iter().map(|s| s.audio_file.as_str()),
    ))
}

/// Record why summarization failed; the session is saved with a stub.
pub fn record_summary_failure(
    store: &SharedStore,
    session_id: &str,
    message: &str,
) -> Result<(), StageError> {
    checkpoint::put(
        store,
        session_id,
        Stage::Summarize,
        SUMMARY_FAILED,
        &message,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    store: &SharedStore,
    session_id: &str,
    working: &Path,
    segments: &[Segment],
    destination: Option<&Path>,
    fs: &dyn Fs,
    control: &dyn Control,
) -> Result<Saved, StageError> {
    // Only a verified transcript is saved (KD13).
    if !checkpoint::is_complete(store, session_id, Stage::Verify, segments.len())? {
        return Err(StageError::Failed(
            "the transcript has not been verified, so the note stays in the working location"
                .into(),
        ));
    }
    // The staged copy leaves `audio-recovery/` behind, and the save then
    // deletes the working folder. Audio that has no segment row is not in
    // the note, so the folder stays (KTD1 of the capture recovery plan).
    if working.exists() && crate::recover::unregistered_audio(working, segments) {
        return Err(StageError::Failed(
            "some recorded audio is not part of the note yet, so the note stays in the working location"
                .into(),
        ));
    }
    // The note guard keeps the flag updates of the early write out of the
    // save: an update either completes before the claim, or sees the claim
    // and writes nothing (KTD4).
    let _guard = crate::early_home::guard(session_id);
    // A note whose early write is not done waits for that one folder, and
    // never gets a second one here (R5). The authoritative state is the
    // early write state, not the home path, which its last step records
    // before that state.
    if crate::early_home::unfinished(&lock(store), session_id)? {
        return Err(StageError::NoDestination(
            "the note's folder is not in the storage folder yet; the note is saved there when the folder is written"
                .into(),
        ));
    }
    let record = lock(store)
        .session(session_id)?
        .ok_or_else(|| StageError::Failed(format!("no session {session_id}")))?;
    let home = record.home_path.clone().map(PathBuf::from);
    let recorded = checkpoint::get::<SaveState>(store, session_id, Stage::Save, STATE)?;
    // `home_path` is recorded with `Saved`, before the step loop reaches
    // `Done`: a first save still in progress finishes through the loop. A
    // resumed session has no save state, since resuming clears it.
    let first_save_in_progress = matches!(
        recorded,
        Some(SaveState::Copying { .. } | SaveState::Staged { .. } | SaveState::Saved { .. })
    );
    if let Some(home) = home
        && !first_save_in_progress
    {
        return update_home(store, session_id, working, segments, &home, fs, control);
    }
    let destination =
        destination.ok_or_else(|| StageError::NoDestination("no storage folder is set".into()))?;
    // The note's start time names the folder (R16, KTD9).
    let started = note_started(&record, segments);
    let (mut state, folder_name) = match recorded {
        Some(state) => {
            // A save that began before names were recorded used the
            // no-title name.
            let name = checkpoint::get::<String>(store, session_id, Stage::Save, FOLDER_NAME)?
                .unwrap_or_else(|| session_save::naming::folder_name(started, None));
            (state, name)
        }
        None => {
            control.proceed()?;
            (SaveState::Working, claim_named(store, session_id, started)?)
        }
    };
    let job = Job {
        session_id,
        working,
        destination,
        folder_name: &folder_name,
    };
    if state == SaveState::Working && working.exists() {
        render(store, session_id, working, segments)?;
        if let Err(e) = check_room(working, destination) {
            release(store, session_id);
            return Err(e);
        }
    }

    let mut copied = matches!(
        state,
        SaveState::Saved {
            working_retained: true,
            ..
        }
    );
    while !matches!(state, SaveState::Done { .. }) {
        let next = match session_save::step(&state, &job, fs) {
            Ok(next) => next,
            Err(e) => {
                // A failed copy removed its partial staging and left the
                // working copy whole: start over from it, so a retry after
                // choosing another folder goes there.
                if matches!(state, SaveState::Copying { .. }) && working.exists() {
                    checkpoint::put(store, session_id, Stage::Save, STATE, &SaveState::Working)?;
                    state = SaveState::Working;
                }
                if state == SaveState::Working && working.exists() {
                    release(store, session_id);
                }
                // Every failure but a lost copy waits for the destination
                // to be fixed or changed (R22b), naming what went wrong.
                return Err(match e {
                    SaveError::Lost(_) => StageError::Failed(e.to_string()),
                    SaveError::Io(io) => StageError::NoDestination(format!(
                        "saving to {} failed: {io}",
                        destination.display()
                    )),
                    other => StageError::NoDestination(other.to_string()),
                });
            }
        };
        if let SaveState::Saved {
            path,
            working_retained,
        } = &next
        {
            copied = *working_retained;
            lock(store).set_home_path(session_id, &path.to_string_lossy())?;
        }
        checkpoint::put(store, session_id, Stage::Save, STATE, &next)?;
        state = next;
    }
    let SaveState::Done { path } = state else {
        unreachable!("the loop ends on Done");
    };
    crate::follow::mark_destination(destination);

    // Reopen notes against the home folder (R22e): anything edited while
    // saving is in the store, and is written here. Under the lock, so an
    // edit comes either before this render or after the session is Saved,
    // where it rewrites the file itself.
    let mut guard = lock(store);
    write_notes(&guard, session_id, &path)?;
    record_transcript_fingerprint(&guard, session_id, &path)?;
    guard.put_checkpoint(
        session_id,
        Stage::Save,
        checkpoint::DONE,
        &serde_json::to_string(&checkpoint::StageDone {
            segments: segments.len(),
        })?,
    )?;
    guard.finish_transcribe_again(session_id, None)?;
    guard.apply(session_id, &Event::Saved)?;
    let path = reconcile_after_done(&guard, session_id, path, fs);
    Ok(Saved { path, copied })
}

/// Room for the session in a destination on another volume, where saving
/// copies: what leaves the working location, plus a margin (U18). Checked
/// before the copy starts, so a copy it just validated cannot run out of
/// space part-way. A same-volume save is a rename and needs none.
fn check_room(working: &Path, destination: &Path) -> Result<(), StageError> {
    const MARGIN: u64 = 64 * 1024 * 1024;
    if same_volume(working, destination).unwrap_or(false) {
        return Ok(());
    }
    let needed = tree_size(working, session_save::LEFT_BEHIND)? + MARGIN;
    match recorder::storage::available_space(destination) {
        Some(free) if free < needed => Err(StageError::NoDestination(format!(
            "the storage folder {} has {:.1} GB free and the note needs about {:.1} GB",
            destination.display(),
            free as f64 / 1e9,
            needed as f64 / 1e9
        ))),
        _ => Ok(()),
    }
}

#[cfg(unix)]
fn same_volume(a: &Path, b: &Path) -> Option<bool> {
    use std::os::unix::fs::MetadataExt;
    Some(std::fs::metadata(a).ok()?.dev() == std::fs::metadata(b).ok()?.dev())
}

#[cfg(windows)]
fn same_volume(a: &Path, b: &Path) -> Option<bool> {
    let root = |p: &Path| {
        let canonical = p.canonicalize().ok()?;
        canonical
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
    };
    Some(root(a)? == root(b)?)
}

/// Bytes under `dir`, not counting the top-level entries named in `skip`.
fn tree_size(dir: &Path, skip: &[&str]) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if skip.iter().any(|s| entry.file_name() == *s) {
            continue;
        }
        let kind = entry.file_type()?;
        total += if kind.is_dir() {
            tree_size(&entry.path(), &[])?
        } else {
            entry.metadata()?.len()
        };
    }
    Ok(total)
}

/// A resumed session completes into the folder it completed into before
/// (R6d): new audio is added beside the old, which is never rewritten, then
/// the new result is published, `longhand.json` last because the folder
/// parses back from it (KTD5). The working directory is the local staging
/// of the publication, and holds its record (KTD1). Every step can run
/// again, so a crash reruns the update: with a record, the rerun only rolls
/// forward (KTD3). The working directory goes only once the folder is
/// complete (R22b).
fn update_home(
    store: &SharedStore,
    session_id: &str,
    working: &Path,
    segments: &[Segment],
    home: &Path,
    fs: &dyn Fs,
    control: &dyn Control,
) -> Result<Saved, StageError> {
    if !home.is_dir() {
        return Err(StageError::NoDestination(format!(
            "the note's folder {} is missing; put it back to finish",
            home.display()
        )));
    }
    // A Transcribe Again run whose summary failed keeps the previous
    // `summary.md`, not the stub (R19, KTD14).
    let again = lock(store)
        .transcribe_again_marks(session_id)?
        .model
        .is_some();
    let summary_error = if again
        && !checkpoint::is_complete(store, session_id, Stage::Summarize, segments.len())?
    {
        Some(
            checkpoint::get::<String>(store, session_id, Stage::Summarize, SUMMARY_FAILED)?
                .unwrap_or_else(|| "summarization did not run".into()),
        )
    } else {
        None
    };
    let recorded = Publication::load(working)?;
    let finished = match checkpoint::get::<SaveState>(store, session_id, Stage::Save, STATE)? {
        Some(state) => matches!(state, SaveState::Done { .. }),
        None => {
            control.proceed()?;
            claim(store, session_id)?;
            false
        }
    };
    if !finished {
        let publication = match recorded {
            Some(publication) => publication,
            None => stage_publication(store, session_id, working, segments, control)?,
        };
        // The library reconcile moves the home path when a sync delivers a
        // renamed folder (KTD6), so the publication reads it again.
        let home = run_home(&lock(store), session_id)?;
        if publication.publish(&home, working)? == Outcome::Conflict {
            return Err(drop_publication(store, session_id, working));
        }
        let done = SaveState::Done { path: home };
        checkpoint::put(store, session_id, Stage::Save, STATE, &done)?;
    }

    let mut guard = lock(store);
    let home = run_home(&guard, session_id)?;
    follow_run(&mut guard, session_id)?;
    write_notes(&guard, session_id, &home)?;
    record_transcript_fingerprint(&guard, session_id, &home)?;
    guard.put_checkpoint(
        session_id,
        Stage::Save,
        checkpoint::DONE,
        &serde_json::to_string(&checkpoint::StageDone {
            segments: segments.len(),
        })?,
    )?;
    // After `Done`: a crash before this line runs the bookkeeping again.
    guard.finish_transcribe_again(session_id, summary_error.as_deref())?;
    let published = NoteFiles::read(&home)?.doc;
    guard.promote_result_snapshot(session_id, published.recognition_snapshot.as_ref())?;
    guard.set_speaker_count(session_id, published.speaker_count)?;
    guard.apply(session_id, &Event::Saved)?;
    // The record goes with the working directory, after the bookkeeping.
    if working.exists() {
        std::fs::remove_dir_all(working)?;
    }
    let path = reconcile_after_done(&guard, session_id, home.to_path_buf(), fs);
    Ok(Saved {
        path,
        copied: false,
    })
}

/// Stage the result of a run that updates its folder, check the folder,
/// and record the publication (KTD1). Nothing in the note folder changes
/// here but the new audio of a Resume. A failed check writes no record, so
/// the previous result stays (R3, R5).
fn stage_publication(
    store: &SharedStore,
    session_id: &str,
    working: &Path,
    segments: &[Segment],
    control: &dyn Control,
) -> Result<Publication, StageError> {
    checkpoint::put(store, session_id, Stage::Save, HOME_CHANGED, &true)?;
    let mut home = run_home(&lock(store), session_id)?;
    if !working.exists() {
        return Err(StageError::Failed(format!(
            "the working copy at {} is gone before {} was updated",
            working.display(),
            home.display()
        )));
    }
    // Another computer can rename the note or edit its notes during the
    // run: the row takes them now, and the run's transcript and summary
    // replace the folder's (KTD3).
    follow_run(&mut lock(store), session_id)?;
    render(store, session_id, working, segments)?;
    // The library reconcile moves the home path when a sync delivers a
    // renamed folder (KTD6), so each write reads it again. The audio is
    // copied without the store lock; when the home path moved during
    // the copy, the copy runs again into the new folder.
    home = run_home(&lock(store), session_id)?;
    let mut guard = loop {
        add_audio(&home, working, segments)?;
        // The last point where a cancel stops the run: the note folder
        // then has only the new audio.
        control.proceed()?;
        let guard = lock(store);
        let now = run_home(&guard, session_id)?;
        if now == home {
            break guard;
        }
        drop(guard);
        home = now;
    };
    // Under the store lock: the checks see the folder and the notes that
    // this computer holds now. They run once, before the record (KTD3).
    follow_run(&mut guard, session_id)?;
    let summary_notes: Option<String> = guard
        .checkpoints(session_id, Stage::Summarize)?
        .into_iter()
        .find(|(key, _)| key == "source-notes")
        .map(|(_, value)| serde_json::from_str(&value))
        .transpose()?;
    let current_notes = folder::fingerprint(guard.notes(session_id)?.as_bytes());
    if summary_notes
        .as_ref()
        .is_some_and(|hash| *hash != current_notes)
    {
        // Retry must produce a summary from the current notes. Keep the
        // recognition checkpoints and the previous result.
        guard.clear_checkpoints(session_id, Stage::Summarize)?;
        guard.delete_checkpoint(session_id, Stage::Save, STATE)?;
        return Err(StageError::Failed(
            "notes changed after the summary started; the previous result was kept".into(),
        ));
    }
    let mut record = guard
        .session(session_id)?
        .ok_or_else(|| StageError::Failed(format!("no session {session_id}")))?;
    if let Some(pending) = guard.result_state(session_id, "pending-snapshot")? {
        record.speaker_count =
            crate::reprocess::speaker_count(&serde_json::from_value(pending.clone())?);
        record.recognition_snapshot = Some(pending);
    }
    let expected: Option<(PathBuf, String)> = guard
        .result_state(session_id, "replacement-base")?
        .map(serde_json::from_value)
        .transpose()?;
    // Only the records count (KTD4): an edit of a `.md` file, a title
    // proposal or a type move does not make the run refuse (R3).
    let changed = match expected {
        Some((path, hash)) if path == home => {
            note_files::new_layout_record(&home)?.run_fingerprint() != hash
        }
        Some(_) => {
            return Err(StageError::Failed(
                "the note folder moved during processing; the previous result was kept".into(),
            ));
        }
        None => false,
    };
    if changed {
        return Err(StageError::Failed(
            "the note changed during processing; the previous result was kept".into(),
        ));
    }
    let staged = with_summary(
        &session_doc(&record, segments, working)?,
        summary_meta(&guard, session_id, segments)?,
    );
    let publication = Publication::new(&home, staged, WriteSet::Result)?;
    publication.record(working)?;
    // This marker prevents generic failure cleanup from discarding the
    // record. It does not prove that the publication committed.
    guard.put_checkpoint(session_id, Stage::Save, HOME_CHANGED, "true")?;
    Ok(publication)
}

/// Another computer's result replaced the note's result before this
/// publication finished (KTD4): delete the record and forget the run, and
/// write nothing. A Transcribe Again then fails and keeps the folder, and
/// its working directory goes. Returns the error that ends the run.
fn drop_publication(store: &SharedStore, session_id: &str, working: &Path) -> StageError {
    log::warn!("{session_id}: {CHANGED_ELSEWHERE} The result of this run was dropped.");
    let dropped = Publication::remove(working)
        .map_err(StageError::from)
        .and_then(|()| {
            let guard = lock(store);
            guard.clear_checkpoints(session_id, Stage::Summarize)?;
            guard.delete_checkpoint(session_id, Stage::Save, STATE)?;
            guard.delete_checkpoint(session_id, Stage::Save, HOME_CHANGED)?;
            Ok(())
        });
    if let Err(e) = dropped {
        log::warn!("could not drop the publication of {session_id}: {e}");
    }
    StageError::Failed(CHANGED_ELSEWHERE.into())
}

/// Copy each segment's audio that `home` does not hold yet from the working
/// copy. The old audio is never rewritten. Only the segments that the
/// Resume recorded are copied: the audio of a segment that the note had
/// when the Resume started, or that has a removal marker, is never put
/// back, in any order in which a sync delivers a removal (KTD8 of the
/// audio removal plan).
fn add_audio(home: &Path, working: &Path, segments: &[Segment]) -> Result<(), StageError> {
    if !home.is_dir() {
        return Err(StageError::NoDestination(format!(
            "the note's folder {} is missing; put it back to finish",
            home.display()
        )));
    }
    let old = crate::old_audio::old_segments(working)?.unwrap_or_default();
    for segment in segments {
        let target = home.join(&segment.audio_file);
        if target.exists()
            || old.iter().any(|o| o.audio_file == segment.audio_file)
            || folder::audio_removed(home, &segment.audio_file)
        {
            continue;
        }
        let (Some(dir), Some(name)) =
            (target.parent(), target.file_name().and_then(|n| n.to_str()))
        else {
            return Err(StageError::Failed(format!(
                "bad audio name {}",
                segment.audio_file
            )));
        };
        std::fs::create_dir_all(dir)?;
        home_write::copy_atomic(dir, name, &working.join(&segment.audio_file))?;
    }
    Ok(())
}

/// The home folder of a session in a run that updates its folder, whatever
/// the session's state.
fn run_home(store: &Store, session_id: &str) -> Result<PathBuf, StageError> {
    store
        .session(session_id)?
        .and_then(|record| record.home_path)
        .map(PathBuf::from)
        .ok_or_else(|| StageError::Failed(format!("the note {session_id} has no folder")))
}

/// Inside a run that updates the note's folder: take the title and the
/// notes from the folder, and keep the run's recognition results (KTD3).
fn follow_run(store: &mut Store, session_id: &str) -> Result<(), StageError> {
    let home = run_home(store, session_id)?;
    crate::follow::follow_scoped(store, session_id, &home, Scope::TitleAndNotes)?;
    Ok(())
}

/// Take the session for saving, unless a resume took it first.
fn claim(store: &SharedStore, session_id: &str) -> Result<(), StageError> {
    let guard = lock(store);
    processing(&guard, session_id)?;
    put_working(&guard, session_id)
}

/// Take the session for a first save and record its folder name, from the
/// title it has now. The name is recorded before the claim, under the same
/// lock, so a recorded claim always has its name.
fn claim_named(
    store: &SharedStore,
    session_id: &str,
    started: DateTime<FixedOffset>,
) -> Result<String, StageError> {
    let guard = lock(store);
    let record = processing(&guard, session_id)?;
    let title = record.title.map(|t| t.text);
    let name = session_save::naming::folder_name(started, title.as_deref());
    guard.put_checkpoint(
        session_id,
        Stage::Save,
        FOLDER_NAME,
        &serde_json::to_string(&name)?,
    )?;
    put_working(&guard, session_id)?;
    Ok(name)
}

/// The session's record, if it is still processing; a resume took it
/// otherwise.
fn processing(store: &Store, session_id: &str) -> Result<SessionRecord, StageError> {
    match store.session(session_id)? {
        Some(record) if matches!(record.state, SessionState::Processing { .. }) => Ok(record),
        _ => Err(StageError::Cancelled),
    }
}

fn put_working(store: &Store, session_id: &str) -> Result<(), StageError> {
    store.put_checkpoint(
        session_id,
        Stage::Save,
        STATE,
        &serde_json::to_string(&SaveState::Working)?,
    )?;
    Ok(())
}

/// Nothing moved: the session may be resumed again, and the next claim
/// records a name from the title it has then.
fn release(store: &SharedStore, session_id: &str) {
    let guard = lock(store);
    for key in [STATE, FOLDER_NAME] {
        if let Err(e) = guard.delete_checkpoint(session_id, Stage::Save, key) {
            log::warn!("could not release the save claim on {session_id}: {e}");
        }
    }
}

/// Write these files in this order beside the audio, the transcript and
/// any summary: `notes.md`, the summary stub when summarization failed,
/// `session.md`, then `longhand.json`.
fn render(
    store: &SharedStore,
    session_id: &str,
    working: &Path,
    segments: &[Segment],
) -> Result<(), StageError> {
    let (record, notes) = {
        let guard = lock(store);
        let mut record = guard
            .session(session_id)?
            .ok_or_else(|| StageError::Failed(format!("no session {session_id}")))?;
        if let Some(pending) = guard.result_state(session_id, "pending-snapshot")? {
            record.speaker_count =
                crate::reprocess::speaker_count(&serde_json::from_value(pending.clone())?);
            record.recognition_snapshot = Some(pending);
        }
        let notes = render_notes(&guard, session_id)?;
        (record, notes)
    };
    write_working(working, NOTES_FILE, &notes)?;
    if !checkpoint::is_complete(store, session_id, Stage::Summarize, segments.len())? {
        let why = checkpoint::get::<String>(store, session_id, Stage::Summarize, SUMMARY_FAILED)?
            .unwrap_or_else(|| "summarization did not run".into());
        write_working(working, SUMMARY_FILE, &stub(&why).render_body())?;
    }
    // `session.md` with the title and the tags of the row, then
    // `longhand.json` last, with the fingerprints of the files written
    // above (KTD2).
    let doc = session_doc(&record, segments, working)?;
    write_working(
        working,
        SESSION_FILE,
        &note_files::new_session_md(doc.title.as_deref(), &doc.tags),
    )?;
    let staged = with_summary(&doc, summary_meta(&lock(store), session_id, segments)?);
    write_working(working, NOTE_RECORD_FILE, &staged.render())?;
    Ok(())
}

/// The records of `doc` with the summary data `summary`.
fn with_summary(doc: &SessionDoc, summary: Option<SummaryMeta>) -> NoteRecord {
    NoteRecord {
        summary,
        ..NoteRecord::from_doc(doc)
    }
}

/// The data of the summary in the working folder of `session_id`: the data
/// that the Summarize stage recorded, or the stub's data when the stage did
/// not complete. The caller holds the store lock.
fn summary_meta(
    store: &Store,
    session_id: &str,
    segments: &[Segment],
) -> Result<Option<SummaryMeta>, StageError> {
    let value = |key: &str| store.checkpoint_value(session_id, Stage::Summarize, key);
    let done = value(checkpoint::DONE)?
        .map(|text| serde_json::from_str::<checkpoint::StageDone>(&text))
        .transpose()?
        .is_some_and(|done| done.segments == segments.len());
    if !done {
        return Ok(Some(SummaryMeta::stub()));
    }
    Ok(value(summarize::META)?
        .map(|text| serde_json::from_str(&text))
        .transpose()?)
}

/// Whether the note folder `home` is in the old layout (KTD6): the actions
/// that change its records wait for the conversion. A folder that cannot
/// be read now is not.
pub fn old_layout(home: &Path) -> bool {
    read_note_files(home, READ_TIMEOUT).is_some_and(|files| files.layout == Layout::Old)
}

/// The start time of the note of `record` (R16, KTD9): the time it opened,
/// else its first segment, else the time of its row.
pub fn note_started(record: &SessionRecord, segments: &[Segment]) -> DateTime<FixedOffset> {
    record.start(segments)
}

/// The note as one document, with its records and its user fields: from
/// the session's record, with the fingerprints of the `transcript.md` and
/// `summary.md` that are in `folder` now (KTD2). Write `longhand.json`
/// after those files, so that the fingerprints match.
pub fn session_doc(
    record: &SessionRecord,
    segments: &[Segment],
    folder: &Path,
) -> Result<SessionDoc, StageError> {
    let started = note_started(record, segments);
    let mut doc = SessionDoc::at(&record.id, started, record.speaker_count, segments);
    // A note with no segment never recorded: it has no result (R6). A
    // writer that keeps the present `session.md` keeps its flag.
    if segments.is_empty() {
        doc.result = Some(ResultFlag::NoRecording);
    }
    doc.title = record.title.as_ref().map(|t| t.text.clone());
    doc.title_source = record.title.as_ref().map(|t| t.source);
    // The user fields of the row (KTD2). A writer that replaces a
    // `session.md` takes them from that file instead.
    doc.tags = record.tags.clone();
    doc.pending_type = record.pending_type.clone();
    // A Transcribe Again run saves its transcript before the store records
    // its model as `asr_model`: the run's model made the transcript.
    doc.asr_model = record
        .transcribe_again_model
        .clone()
        .or_else(|| record.asr_model.clone());
    doc.found_speakers = record.found_speakers;
    doc.recognition_snapshot = record.recognition_snapshot.clone();
    doc.transcript_fingerprint = fingerprint_of(folder, TRANSCRIPT_FILE)?;
    doc.summary_fingerprint = fingerprint_of(folder, SUMMARY_FILE)?;
    // A missing `recognition.jsonl`, or one for another transcript, gives
    // no reference: the note then has no timed text to use again (KTD8).
    let artifact = folder.join(session_core::recognition::ARTIFACT_FILE);
    if artifact.is_file()
        && NoteFiles::read(folder).map_or(true, |files| files.record.recognition_artifact.is_some())
        && let Some((reference, header)) = read_artifact_header(&artifact)?
        && doc.transcript_fingerprint.as_ref() == Some(&header.transcript)
    {
        doc.recognition_artifact = Some(reference);
        let mut audio = std::collections::BTreeMap::new();
        for segment in segments {
            let source = if folder.join(&segment.audio_file).is_file() {
                folder.join(&segment.audio_file)
            } else {
                record
                    .home_path
                    .as_deref()
                    .map(Path::new)
                    .unwrap_or(folder)
                    .join(&segment.audio_file)
            };
            if source.is_file() {
                audio.insert(
                    segment.audio_file.clone(),
                    session_core::recognition::file_digest(&source)?,
                );
            }
        }
        if audio.len() == segments.len() {
            doc.recognition_audio = Some(audio);
        }
    }
    Ok(doc)
}

/// The reference to the `recognition.jsonl` at `path` and its header.
/// `None` when the file is missing.
fn read_artifact_header(
    path: &Path,
) -> Result<
    Option<(
        session_core::recognition::ArtifactReference,
        session_core::recognition::ArtifactHeader,
    )>,
    StageError,
> {
    use session_core::recognition::{self as recognition, ArtifactError};
    let missing = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
    let sha256 = match recognition::file_digest(path) {
        Ok(sha256) => sha256,
        Err(e) if missing(&e) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let reference = recognition::ArtifactReference {
        version: recognition::ARTIFACT_VERSION,
        filename: recognition::ARTIFACT_FILE.into(),
        sha256,
    };
    match recognition::read_artifact(path, &reference, |_| Ok(())) {
        Ok(header) => Ok(Some((reference, header))),
        Err(ArtifactError::Io(e)) if missing(&e) => Ok(None),
        Err(e) => Err(StageError::Failed(e.to_string())),
    }
}

/// Record the fingerprint of the `transcript.md` that this save wrote into
/// `folder` (KTD12). A later refresh then knows the file as this
/// computer's, and keeps the stored recognition results.
fn record_transcript_fingerprint(
    store: &Store,
    session_id: &str,
    folder: &Path,
) -> Result<(), StageError> {
    let written = fingerprint_of(folder, TRANSCRIPT_FILE)?;
    store.set_transcript_fingerprint(session_id, written.as_deref())?;
    Ok(())
}

/// The fingerprint of the file `name` in `folder`, or `None` when the file
/// is not there. The read has a timeout, because a synced folder can hold
/// a file that does not download.
fn fingerprint_of(folder: &Path, name: &str) -> Result<Option<String>, StageError> {
    let path = folder.join(name);
    match read_timed(path.clone(), READ_TIMEOUT) {
        Read::Bytes(bytes) => Ok(Some(folder::fingerprint(&bytes))),
        Read::Missing => Ok(None),
        Read::Failed(why) => Err(unreadable(&path, &why)),
    }
}

/// The error for a file in the storage folder that cannot be read.
fn unreadable(path: &Path, why: &str) -> StageError {
    StageError::Failed(format!(
        "the storage folder is not available: {} cannot be read: {why}",
        path.display()
    ))
}

/// After the save records `Done`: the save used the folder name it
/// recorded when it began (KTD7). A title that changed since then renames
/// the folder now (KTD8). Returns the folder's path. A failed rename does
/// not fail the save: the reconcile at the next launch tries again. No
/// regeneration and no title job runs, because the stage holds the turn
/// of the summary queue.
fn reconcile_after_done(store: &Store, session_id: &str, path: PathBuf, fs: &dyn Fs) -> PathBuf {
    moved_to(
        session_id,
        folder_rename::reconcile(store, session_id, false, true, fs),
    )
    .unwrap_or(path)
}

/// The folder's new path when a reconcile moved it. A reconcile that
/// failed is logged, and the folder keeps its path.
fn moved_to(session_id: &str, reconciled: Result<FolderRename, StageError>) -> Option<PathBuf> {
    match reconciled {
        Ok(FolderRename::Moved(folder)) => Some(folder),
        Ok(FolderRename::Kept(why)) => {
            log::warn!("the folder of {session_id} keeps its name: {why}");
            None
        }
        Ok(FolderRename::Unchanged | FolderRename::Later) => None,
        Err(e) => {
            log::warn!("could not reconcile the folder of {session_id}: {e}");
            None
        }
    }
}

pub fn stub(why: &str) -> Summary {
    Summary {
        meta: SummaryMeta::stub(),
        body: format!(
            "# Summary\n\nNo summary was produced: {why}\n\nThe transcript and notes are complete. Regenerate the summary from the note in Longhand."
        ),
    }
}

fn render_notes(store: &Store, session_id: &str) -> Result<String, StageError> {
    Ok(folder::render_notes(&store.notes(session_id)?))
}

/// Write the notes of the store into `folder`, and record the fingerprint
/// of the file as the `notes.md` that this computer last wrote (KTD8).
fn write_notes(store: &Store, session_id: &str, folder: &Path) -> Result<(), StageError> {
    let notes = render_notes(store, session_id)?;
    home_write::write_atomic(folder, NOTES_FILE, notes.as_bytes())?;
    store
        .set_written_notes_fingerprint(session_id, Some(&folder::fingerprint(notes.as_bytes())))?;
    Ok(())
}

/// Inside the app's own working location, a fixed temporary name is safe.
fn write_working(dir: &Path, name: &str, contents: &str) -> Result<(), StageError> {
    let temp = dir.join(format!("{name}.tmp"));
    std::fs::write(&temp, contents)?;
    crate::sync_file(&temp)?;
    std::fs::rename(&temp, dir.join(name))?;
    Ok(())
}

/// The error of a regeneration of a note that is not saved.
fn not_saved() -> StageError {
    StageError::Failed(
        "a summary can be regenerated only once the note has finished processing".into(),
    )
}

/// Refuse a regeneration of a note that is not saved or has no home folder
/// (R10). [`regenerate_summary`] does the same check again.
pub fn require_saved(store: &Store, session_id: &str) -> Result<(), StageError> {
    home_path(store, session_id)?.ok_or_else(not_saved)?;
    Ok(())
}

/// The home folder of a session, if it is saved.
fn home_path(store: &Store, session_id: &str) -> Result<Option<PathBuf>, StageError> {
    Ok(store
        .session(session_id)?
        .filter(|r| r.state == SessionState::Saved)
        .and_then(|r| r.home_path)
        .map(PathBuf::from))
}

/// What a notes save did (KTD10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotesSave {
    /// The store holds the document. `written` when the note folder's
    /// `notes.md` holds it too.
    Saved { written: bool },
    /// `notes.md` changed after the editor loaded it: another computer
    /// wrote it. The file and the row keep that text, and the document is
    /// in the draft file at `draft`, beside `notes.md`.
    Conflict { draft: PathBuf },
    /// The note has no audio, and its home folder is not at its home path:
    /// another computer can have removed it (R24). The store holds the
    /// document, and no folder is made. [`rescue_notes`] finds out what
    /// happened to the folder.
    FolderGone,
}

/// The fingerprint of the `notes.md` that holds `document`. The editor
/// sends it back with each save, as the fingerprint of the file that it
/// loaded (KTD10).
pub fn notes_fingerprint(document: &str) -> String {
    folder::fingerprint(folder::render_notes(document).as_bytes())
}

/// Replace a session's notes document (KTD5, R20), with no check of what
/// the editor loaded. See [`save_notes`]. Returns whether the note
/// folder's `notes.md` holds the document.
pub fn set_notes(store: &mut Store, session_id: &str, document: &str) -> Result<bool, StageError> {
    match save_notes(store, session_id, document, None)? {
        NotesSave::Saved { written } => Ok(written),
        NotesSave::FolderGone => Ok(false),
        NotesSave::Conflict { .. } => unreachable!("a save with no loaded fingerprint"),
    }
}

/// Replace a session's notes document (KTD5, KTD10, R20). The row first
/// follows its folder (KTD3). A session with a home folder then gets its
/// `notes.md` rewritten whole in the same call (R22f), also while a Resume
/// or a Transcribe Again runs on it, so that the folder and the row agree
/// and a refresh in that run keeps the edit.
///
/// `loaded` is the fingerprint of the `notes.md` that the editor loaded
/// ([`notes_fingerprint`]). When the file in the folder has another
/// fingerprint, another computer changed it: the file stays as it is, and
/// the document goes into a draft beside it.
///
/// The caller holds the store lock for the whole call, which is the lock
/// the save stage takes to write `notes.md`: an edit comes either before
/// that write or after it, so no edit is lost.
pub fn save_notes(
    store: &mut Store,
    session_id: &str,
    document: &str,
    loaded: Option<&str>,
) -> Result<NotesSave, StageError> {
    let record = store
        .session(session_id)?
        .ok_or_else(|| session_core::StoreError::NoSuchSession(session_id.to_string()))?;
    crate::follow::follow_folder(store, session_id, false)?;
    let saved = record.state == SessionState::Saved;
    // A session in a run keeps the edit in the store when its folder is
    // not there; the save of the run writes it. A saved session reports
    // the folder that is gone.
    let home = record
        .home_path
        .map(PathBuf::from)
        .filter(|home| saved || home.is_dir());
    // A saved note with no audio whose folder is gone keeps the edit in the
    // store and makes no folder (R24).
    if saved
        && home.as_ref().is_some_and(|home| !home.is_dir())
        && store.segments(session_id)?.is_empty()
    {
        store.set_notes(session_id, document)?;
        return Ok(NotesSave::FolderGone);
    }
    // A `notes.md` that cannot be read in time is not replaced: this
    // computer cannot tell what it holds, so the edit goes into a draft.
    if let (Some(home), Some(loaded)) = (&home, loaded)
        && match read_timed(home.join(NOTES_FILE), READ_TIMEOUT) {
            Read::Bytes(bytes) => folder::fingerprint(&bytes) != loaded,
            Read::Missing => false,
            Read::Failed(why) => {
                log::warn!("cannot read the notes of {session_id}: {why}");
                true
            }
        }
    {
        let draft = write_draft(home, document)?;
        log::info!(
            "the notes of {session_id} changed on another computer; the edit is in {}",
            draft.display()
        );
        return Ok(NotesSave::Conflict { draft });
    }
    store.set_notes(session_id, document)?;
    let Some(home) = home else {
        return Ok(NotesSave::Saved { written: false });
    };
    write_notes(store, session_id, &home)?;
    if saved {
        store.apply(session_id, &Event::SavedEdit)?;
    }
    Ok(NotesSave::Saved { written: true })
}

/// Write `document` beside `notes.md` in `dir` as a conflict draft,
/// `notes (Longhand conflict YYYY-MM-DD HHMM).md`, a name of the conflict
/// form that the library scan finds (KTD5, KTD10). A draft of the same
/// minute gets a number, so no draft replaces another.
fn write_draft(dir: &Path, document: &str) -> Result<PathBuf, StageError> {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M").to_string();
    let mut name = format!("notes (Longhand conflict {stamp}).md");
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("notes (Longhand conflict {stamp}-{n}).md");
        n += 1;
    }
    home_write::write_atomic(dir, &name, folder::render_notes(document).as_bytes())?;
    Ok(dir.join(name))
}

/// What [`rescue_notes`] did (R24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesRescue {
    /// The folder is at another name in the storage folder, and the row
    /// now has that home path. The notes save can run again.
    Moved,
    /// The storage folder is not available, or a note folder in it cannot
    /// be read. Nothing changed, and the store keeps the notes.
    Unavailable,
    /// The folder is gone. The notes and the title are in the new note
    /// with no audio, and the row of the old note is forgotten.
    NewNote,
}

/// Keep the notes of the note `id` after a notes save found its home
/// folder gone ([`NotesSave::FolderGone`], R24). When no folder in the
/// storage folder holds the note, the store makes the note `new_id`, which
/// opened at `now` with no audio, with the notes and the title of `id`,
/// and then forgets `id`. The new note has its early write begun: the
/// caller starts its job. The caller stops the early write job of `id`
/// first. The text is never lost.
pub fn rescue_notes(
    store: &SharedStore,
    id: &str,
    new_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<NotesRescue, StageError> {
    let Some(home) = lock(store)
        .session(id)?
        .and_then(|r| r.home_path)
        .map(PathBuf::from)
    else {
        return Ok(NotesRescue::Unavailable);
    };
    match crate::early_home::locate(store, id, &home)? {
        crate::early_home::Location::Moved | crate::early_home::Location::There => {
            return Ok(NotesRescue::Moved);
        }
        crate::early_home::Location::Unknown => return Ok(NotesRescue::Unavailable),
        crate::early_home::Location::Gone => {}
    }
    let mut guard = lock(store);
    // The row can change while the scan runs: only the same note with no
    // audio and the same home path moves into a new note.
    let Some(record) = guard.session(id)? else {
        return Ok(NotesRescue::Unavailable);
    };
    if record.state != SessionState::Saved
        || record.home_path.as_deref().map(Path::new) != Some(home.as_path())
        || !guard.segments(id)?.is_empty()
    {
        return Ok(NotesRescue::Unavailable);
    }
    let notes = guard.notes(id)?;
    guard.create_notes_only(new_id, now, None)?;
    crate::early_home::begin(&guard, new_id)?;
    guard.set_notes(new_id, &notes)?;
    if let Some(title) = &record.title {
        guard.set_title(new_id, &title.text, title.source)?;
    }
    if !guard.forget(id)? {
        // A rename or a Move still needs the row: the library forgets it
        // later. The new note holds the text.
        log::warn!("the row of {id} stays until its rename or Move ends");
    }
    log::info!("the folder of {id} is gone; its notes are in the new note {new_id}");
    Ok(NotesRescue::NewNote)
}

/// Summarize a saved session again and replace its `summary.md` (R19,
/// AE17): a stub left by a failed summary, or a summary the user wants
/// redone. A rename that waited for the regeneration then moves the
/// folder, whether the regeneration succeeded or not (KTD8). The returned
/// path is in the folder under its name after that rename. `resolve` gives
/// the summary prompt of a note type. The regeneration calls it after the
/// row follows its folder, so a type change from another computer counts
/// (R3, KTD3). `queued` is the time that the regeneration waited for its
/// turn in the summary queue, for the request log.
#[allow(clippy::too_many_arguments)]
pub fn regenerate_summary(
    store: &SharedStore,
    session_id: &str,
    working_root: &Path,
    endpoint: SummaryEndpoint,
    resolve: &dyn Fn(Option<&str>) -> Prompt,
    control: &dyn Control,
    queued: std::time::Duration,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PathBuf, StageError> {
    let regenerated = summarize_again(
        store,
        session_id,
        working_root,
        endpoint,
        resolve,
        control,
        queued,
        progress,
    );
    // A failed regeneration publishes no new summary, so the title request
    // of that summary is not sent (KTD13). The note keeps its title.
    if regenerated.is_err()
        && let Err(e) = summarize::delete_title_request(store, session_id)
    {
        log::warn!("could not delete the title request of {session_id}: {e}");
    }
    // The regeneration has ended, and the caller holds the turn of the
    // summary queue, so no other regeneration, Save or title job runs.
    let renamed = folder_rename::reconcile_shared(store, session_id, false, &session_save::RealFs);
    let path = regenerated?;
    Ok(moved_to(session_id, renamed).map_or(path, |folder| folder.join(SUMMARY_FILE)))
}

#[allow(clippy::too_many_arguments)]
fn summarize_again(
    store: &SharedStore,
    session_id: &str,
    working_root: &Path,
    endpoint: SummaryEndpoint,
    resolve: &dyn Fn(Option<&str>) -> Prompt,
    control: &dyn Control,
    queued: std::time::Duration,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PathBuf, StageError> {
    // The row follows its folder first (KTD3): the summary uses the notes
    // and the title that another computer wrote. The caller holds the
    // regeneration claim. A publication that an earlier regeneration left
    // is finished or dropped first, so that two publications of the note
    // never overlap (KTD4).
    let (home, segments, note_type) = {
        let mut guard = lock(store);
        home_path(&guard, session_id)?.ok_or_else(not_saved)?;
        recover_summary_publication(&mut guard, session_id)?;
        crate::follow::follow_folder(&mut guard, session_id, false)?;
        let home = home_path(&guard, session_id)?.ok_or_else(not_saved)?;
        let record = guard.session(session_id)?.ok_or_else(not_saved)?;
        (
            home,
            guard.segments(session_id)?,
            crate::type_move::shown_type(&record).0,
        )
    };
    // The type that the note shows after the follow chooses the prompt
    // (R3, KTD3).
    let active = resolve(note_type.as_deref());
    // A note in the old layout waits for the conversion: nothing is
    // written (KTD6).
    let mut current = note_files::new_layout_record(&home)?;
    // Regenerate Summary explicitly accepts the current transcript text.
    // An external edit removes the reusable recognition reference.
    // A missing transcript counts as a hand edit after the wait for the
    // sync (R8), but there is nothing to summarize.
    let canonical = std::fs::read(home.join(TRANSCRIPT_FILE)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StageError::Failed("The transcript is missing.".into())
        } else {
            unreadable(&home.join(TRANSCRIPT_FILE), &e.to_string())
        }
    })?;
    let current_hash = folder::fingerprint(&canonical);
    let changed = || {
        StageError::Failed(
            "the note changed while its summary was being made; the previous result was kept"
                .into(),
        )
    };
    if current.transcript_fingerprint.as_ref() != Some(&current_hash) {
        let summary_fingerprint = fingerprint_of(&home, SUMMARY_FILE)?;
        // A result of another computer can reach `longhand.json` after the
        // read above. Read it again just before the write, and refuse when
        // its records changed, as the publication does.
        let mut now = note_files::new_layout_record(&home)?;
        if now.run_fingerprint() != current.run_fingerprint() {
            return Err(changed());
        }
        now.transcript_fingerprint = Some(current_hash.clone());
        now.summary_fingerprint = summary_fingerprint;
        now.recognition_artifact = None;
        now.recognition_audio = None;
        note_files::write_record(&home, &now)?;
        current = now;
    }
    // A stub resumes from the calls made before the failure, but only when
    // they were made with the active prompt's text (KTD5) and the current
    // Background, compared after trimming (KTD6), and from the
    // transcript and the notes that the folder holds now
    // ([`summarize::INPUT`]). A complete
    // summary, or a stub made with another prompt or with no saved prompt,
    // starts over. The title call always runs again, so a new proposal
    // follows (R31). A title that the user set stays.
    let complete = checkpoint::is_complete(store, session_id, Stage::Summarize, segments.len())?;
    let saved = checkpoint::get::<Prompt>(store, session_id, Stage::Summarize, summarize::PROMPT)?;
    let same_prompt = saved.is_some_and(|p| {
        p.id == active.id
            && p.text == active.text
            && p.background.trim() == active.background.trim()
    });
    if complete || !same_prompt {
        summarize::clear(store, session_id)?;
    } else {
        {
            let guard = lock(store);
            guard.delete_checkpoint(session_id, Stage::Summarize, summarize::TITLE)?;
            // A title request of an earlier summary is not sent (KTD9).
            guard.delete_checkpoint(session_id, Stage::Summarize, summarize::TITLE_REQUEST)?;
        }
        // The text is the same, so the cached calls stand. The name can
        // be new after a rename, and the summary records the name now.
        checkpoint::put(
            store,
            session_id,
            Stage::Summarize,
            summarize::PROMPT,
            &active,
        )?;
    }
    // The transcript comes from `transcript.md`, so a computer with no
    // recognition results for the note summarizes the same text (KTD4).
    // Only the records count (KTD4): a tag edit, a title edit, a title
    // proposal or a type move in the meantime does not make the run refuse.
    let expected = current.run_fingerprint();
    let source_notes = folder::fingerprint(lock(store).notes(session_id)?.as_bytes());
    let summary = summarize::produce(
        store, session_id, &home, endpoint, active, control, queued, progress,
    )?;
    // The last point where a cancel stops the regeneration: after the
    // record, the publication only rolls forward.
    control.proceed()?;
    // The home folder is read again under the lock, just before the write.
    // The checks run once, before the record (KTD3).
    let (path, local, publication) = {
        let guard = lock(store);
        let path = home_path(&guard, session_id)?.ok_or_else(not_saved)?;
        // The model calls can take long. A user title that another
        // computer wrote in the meantime goes into the row first, so the
        // `session.md` below keeps it.
        if path != home || source_notes != folder::fingerprint(guard.notes(session_id)?.as_bytes())
        {
            return Err(changed());
        }
        keep_folder_user_title(&guard, session_id, &path)?;
        let previous = note_files::new_layout_record(&path)?;
        if previous.run_fingerprint() != expected {
            return Err(changed());
        }
        // A computer that has not recorded has no working folder yet.
        std::fs::create_dir_all(working_root)?;
        let local = tempfile::Builder::new()
            .prefix(".longhand-summary-")
            .tempdir_in(working_root)?
            .keep();
        guard.set_result_state(
            session_id,
            SUMMARY_PUBLICATION,
            &serde_json::to_value(&local)?,
        )?;
        let staged = (|| -> Result<Publication, StageError> {
            // `summary.md` has no frontmatter: its data goes into
            // `longhand.json` (R1).
            home_write::write_atomic(&local, SUMMARY_FILE, summary.render_body().as_bytes())?;
            let record = guard.session(session_id)?.ok_or_else(not_saved)?;
            let mut staged = with_summary(
                &session_doc(&record, &segments, &local)?,
                Some(summary.meta.clone()),
            );
            // The summary is made from the transcript that the folder
            // holds. `transcript.md` and `recognition.jsonl` are not
            // written, so their references stay as they are (KTD5).
            staged.transcript_fingerprint = Some(current_hash);
            staged.recognition_snapshot = previous.recognition_snapshot;
            staged.recognition_artifact = previous.recognition_artifact;
            staged.recognition_audio = previous.recognition_audio;
            Ok(Publication::new(&path, staged, WriteSet::Summary)?)
        })();
        // A failure before the record changes nothing in the note folder
        // (R3).
        let publication = match staged {
            Ok(publication) => publication,
            Err(error) => {
                drop_summary_staging(&guard, session_id, &local)?;
                return Err(error);
            }
        };
        publication.record(&local)?;
        (path, local, publication)
    };
    // From here on the publication only rolls forward: after an error, the
    // record stays, and the recovery at the next launch or the next
    // regeneration finishes it.
    let outcome = publication.publish(&path, &local)?;
    let mut guard = lock(store);
    if outcome == Outcome::Conflict {
        log::warn!("{session_id}: {CHANGED_ELSEWHERE} The regenerated summary was dropped.");
        drop_summary_staging(&guard, session_id, &local)?;
        return Err(StageError::Failed(CHANGED_ELSEWHERE.into()));
    }
    summary_published(&mut guard, session_id, segments.len())?;
    drop_summary_staging(&guard, session_id, &local)?;
    Ok(path.join(SUMMARY_FILE))
}

/// Store the user title of the `session.md` in `folder` as the row's
/// title, when the row has another title. Only a user title is taken: a
/// proposed title in the folder is older than the proposal that the
/// regeneration stored. Files that cannot be read or parsed change
/// nothing. The caller holds the store lock.
///
/// In the new layout, the title is a user title when Longhand wrote it as
/// one. A title that differs from the title that Longhand last wrote
/// becomes a user title only after about one minute (KTD5): the library
/// reconcile takes it then. The regeneration does not write `session.md`,
/// so the title stays in the folder. In the old layout, a title with no
/// source is the user's when it is not the stored one (R5).
fn keep_folder_user_title(
    store: &Store,
    session_id: &str,
    folder: &Path,
) -> Result<(), StageError> {
    let Some(files) = read_note_files(folder, READ_TIMEOUT) else {
        return Ok(());
    };
    if files.title_changed_outside() {
        return Ok(());
    }
    let doc = files.doc;
    let Some(text) = doc
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return Ok(());
    };
    let stored = store.title(session_id)?;
    if stored.as_ref().is_some_and(|t| t.text == text) {
        return Ok(());
    }
    if matches!(doc.title_source, None | Some(TitleSource::User)) {
        store.set_title(session_id, text, TitleSource::User)?;
    }
    Ok(())
}

#[cfg(test)]
mod room_tests {
    use super::*;

    #[test]
    fn the_room_a_copy_needs_leaves_out_what_stays_behind() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("audio")).unwrap();
        std::fs::create_dir_all(dir.path().join("proc")).unwrap();
        std::fs::write(dir.path().join("audio/segment-01.opus"), vec![0u8; 1_000]).unwrap();
        std::fs::write(dir.path().join("session.md"), vec![0u8; 24]).unwrap();
        std::fs::write(dir.path().join("proc/segment-01.wav"), vec![0u8; 50_000]).unwrap();
        assert_eq!(
            tree_size(dir.path(), session_save::LEFT_BEHIND).unwrap(),
            1_024
        );
    }

    #[test]
    fn a_folder_on_the_same_volume_needs_no_room() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert_eq!(same_volume(&a, &b), Some(true));
        assert!(check_room(&a, &b).is_ok());
    }
}
