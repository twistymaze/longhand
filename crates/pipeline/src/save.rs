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

use session_core::folder::{self, SessionDoc, Summary, SummaryMeta};
use session_core::{Event, Segment, SessionRecord, SessionState, Stage, Store};
use session_save::{Fs, Job, SaveError, SaveState, home_write};

use crate::folder_rename::{self, FolderRename};
use crate::join::TRANSCRIPT_FILE;
use crate::summarize::{self, SUMMARY_FILE, SummaryEndpoint};
use crate::{Control, SharedStore, StageError, checkpoint, lock};

pub const SESSION_FILE: &str = "session.md";
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

/// Whether a completed session's folder still holds every segment's audio,
/// which is what resuming it needs (R6d). False for a session that has not
/// completed.
pub fn home_intact(store: &Store, session_id: &str) -> Result<bool, StageError> {
    let Some(home) = home_path(store, session_id)? else {
        return Ok(false);
    };
    let segments = store.segments(session_id)?;
    Ok(!segments.is_empty() && segments.iter().all(|s| home.join(&s.audio_file).is_file()))
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
    let home = lock(store)
        .session(session_id)?
        .and_then(|record| record.home_path)
        .map(PathBuf::from);
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
    let destination = destination
        .ok_or_else(|| StageError::NoDestination("no destination folder is set".into()))?;
    let started = segments
        .first()
        .ok_or_else(|| StageError::Failed("the note has no audio".into()))?
        .wall_start;
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

    // Reopen notes against the home folder (R22e): anything edited while
    // saving is in the store, and is written here. Under the lock, so an
    // edit comes either before this render or after the session is Saved,
    // where it rewrites the file itself.
    let mut guard = lock(store);
    write_notes(&guard, session_id, &path)?;
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
            "the destination folder {} has {:.1} GB free and the note needs about {:.1} GB",
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
/// the documents are replaced, `session.md` last because the folder parses
/// back from it. Every step can run again, so a crash reruns the update;
/// the working directory goes only once the folder is complete (R22b).
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
    let finished = match checkpoint::get::<SaveState>(store, session_id, Stage::Save, STATE)? {
        Some(state) => matches!(state, SaveState::Done { .. }),
        None => {
            control.proceed()?;
            claim(store, session_id)?;
            false
        }
    };
    if !finished {
        if !working.exists() {
            return Err(StageError::Failed(format!(
                "the working copy at {} is gone before {} was updated",
                working.display(),
                home.display()
            )));
        }
        render(store, session_id, working, segments)?;
        for segment in segments {
            let target = home.join(&segment.audio_file);
            if target.exists() {
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
        for name in [TRANSCRIPT_FILE, SUMMARY_FILE, NOTES_FILE, SESSION_FILE] {
            if name == SUMMARY_FILE && summary_error.is_some() {
                continue;
            }
            let contents = std::fs::read(working.join(name))?;
            home_write::write_atomic(home, name, &contents)?;
        }
        let done = SaveState::Done {
            path: home.to_path_buf(),
        };
        checkpoint::put(store, session_id, Stage::Save, STATE, &done)?;
    }
    if working.exists() {
        std::fs::remove_dir_all(working)?;
    }

    let mut guard = lock(store);
    write_notes(&guard, session_id, home)?;
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
    guard.apply(session_id, &Event::Saved)?;
    let path = reconcile_after_done(&guard, session_id, home.to_path_buf(), fs);
    Ok(Saved {
        path,
        copied: false,
    })
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

/// `session.md`, `notes.md`, and the summary stub when summarization
/// failed, beside the audio, the transcript and any summary.
fn render(
    store: &SharedStore,
    session_id: &str,
    working: &Path,
    segments: &[Segment],
) -> Result<(), StageError> {
    let (record, notes) = {
        let guard = lock(store);
        let record = guard
            .session(session_id)?
            .ok_or_else(|| StageError::Failed(format!("no session {session_id}")))?;
        let notes = render_notes(&guard, session_id)?;
        (record, notes)
    };
    let doc = session_doc(&record, segments)?;
    write_working(working, SESSION_FILE, &doc.render())?;
    write_working(working, NOTES_FILE, &notes)?;
    if !checkpoint::is_complete(store, session_id, Stage::Summarize, segments.len())? {
        let why = checkpoint::get::<String>(store, session_id, Stage::Summarize, SUMMARY_FAILED)?
            .unwrap_or_else(|| "summarization did not run".into());
        write_working(working, SUMMARY_FILE, &stub(&why).render())?;
    }
    Ok(())
}

/// The contents of `session.md`, with the session's title.
pub fn session_doc(record: &SessionRecord, segments: &[Segment]) -> Result<SessionDoc, StageError> {
    let mut doc = SessionDoc::new(&record.id, record.speaker_count, segments)
        .ok_or_else(|| StageError::Failed("the note has no audio".into()))?;
    doc.title = record.title.as_ref().map(|t| t.text.clone());
    Ok(doc)
}

/// After the save records `Done`: the save used the folder name it
/// recorded when it began (KTD7). A title that changed since then renames
/// the folder now (KTD8). Returns the folder's path. A failed rename does
/// not fail the save: the reconcile at the next launch tries again. No
/// regeneration runs, because the stage holds the one processing permit.
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
        meta: SummaryMeta::Stub {
            failed_stage: "summarize".into(),
        },
        body: format!(
            "# Summary\n\nNo summary was produced: {why}\n\nThe transcript and notes are complete. Regenerate the summary from the note in Longhand."
        ),
    }
}

fn render_notes(store: &Store, session_id: &str) -> Result<String, StageError> {
    Ok(folder::render_notes(&store.notes(session_id)?))
}

fn write_notes(store: &Store, session_id: &str, folder: &Path) -> Result<(), StageError> {
    let notes = render_notes(store, session_id)?;
    home_write::write_atomic(folder, NOTES_FILE, notes.as_bytes())?;
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

/// The home folder of a session, if it is saved.
fn home_path(store: &Store, session_id: &str) -> Result<Option<PathBuf>, StageError> {
    Ok(store
        .session(session_id)?
        .filter(|r| r.state == SessionState::Saved)
        .and_then(|r| r.home_path)
        .map(PathBuf::from))
}

/// Replace a session's notes document (KTD5, R20). On a saved session the
/// home folder's `notes.md` is rewritten whole in the same call (R22f).
/// The caller holds the store lock for the whole call, which is the lock
/// the save stage takes to write `notes.md` after `Done`: an edit comes
/// either before that write or after the session is saved, so no edit is
/// lost. Returns false when the session is not saved, so there is no file
/// to write yet.
pub fn set_notes(store: &mut Store, session_id: &str, document: &str) -> Result<bool, StageError> {
    store.set_notes(session_id, document)?;
    rewrite_notes(store, session_id)
}

fn rewrite_notes(store: &mut Store, session_id: &str) -> Result<bool, StageError> {
    let Some(path) = home_path(store, session_id)? else {
        return Ok(false);
    };
    write_notes(store, session_id, &path)?;
    store.apply(session_id, &Event::SavedEdit)?;
    Ok(true)
}

/// Summarize a saved session again and replace its `summary.md` (R19,
/// AE17): a stub left by a failed summary, or a summary the user wants
/// redone. A rename that waited for the regeneration then moves the
/// folder, whether the regeneration succeeded or not (KTD8). The returned
/// path is in the folder under its name after that rename.
pub fn regenerate_summary(
    store: &SharedStore,
    session_id: &str,
    endpoint: SummaryEndpoint,
    learn: &dyn Fn(u32),
    control: &dyn Control,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PathBuf, StageError> {
    let regenerated = summarize_again(store, session_id, endpoint, learn, control, progress);
    // The regeneration has ended, and the caller holds the one processing
    // permit, so no other regeneration runs.
    let renamed = folder_rename::reconcile_shared(store, session_id, false, &session_save::RealFs);
    let path = regenerated?;
    Ok(moved_to(session_id, renamed).map_or(path, |folder| folder.join(SUMMARY_FILE)))
}

fn summarize_again(
    store: &SharedStore,
    session_id: &str,
    endpoint: SummaryEndpoint,
    learn: &dyn Fn(u32),
    control: &dyn Control,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PathBuf, StageError> {
    let not_saved = || {
        StageError::Failed(
            "a summary can be regenerated only once the note has finished processing".into(),
        )
    };
    let segments = {
        let guard = lock(store);
        home_path(&guard, session_id)?.ok_or_else(not_saved)?;
        guard.segments(session_id)?
    };
    // A stub resumes from the calls made before the failure; a complete
    // summary starts over. The title call always runs again, so a new
    // proposal follows (R31). A title that the user set stays.
    if checkpoint::is_complete(store, session_id, Stage::Summarize, segments.len())? {
        summarize::clear(store, session_id)?;
    } else {
        lock(store).delete_checkpoint(session_id, Stage::Summarize, summarize::TITLE)?;
    }
    let summary = summarize::produce(
        store, session_id, &segments, endpoint, learn, control, progress,
    )?;
    // The home folder is read again under the lock, just before the write,
    // and the lock is held through the write.
    let path = {
        let guard = lock(store);
        let path = home_path(&guard, session_id)?.ok_or_else(not_saved)?;
        home_write::write_atomic(&path, SUMMARY_FILE, summary.render().as_bytes())?;
        path
    };
    checkpoint::mark_complete(store, session_id, Stage::Summarize, segments.len())?;
    let mut guard = lock(store);
    guard.delete_checkpoint(session_id, Stage::Summarize, SUMMARY_FAILED)?;
    guard.clear_summary_not_updated(session_id)?;
    guard.apply(session_id, &Event::SavedEdit)?;
    Ok(path.join(SUMMARY_FILE))
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
