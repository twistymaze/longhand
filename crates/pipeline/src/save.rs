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
use library::scan::{READ_TIMEOUT, Read, read_timed};
use session_core::folder::{self, SessionDoc, Summary, SummaryMeta};
use session_core::{Event, Segment, SessionRecord, SessionState, Stage, Store, TitleSource};
use session_save::{Fs, Job, SaveError, SaveState, home_write};

use crate::folder_rename::{self, FolderRename};
use crate::join::TRANSCRIPT_FILE;
use crate::summarize::{self, Prompt, SUMMARY_FILE, SummaryEndpoint};
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
/// A replacement reached publication. Preserve its local files on failure.
/// Only the manifest proves that the replacement committed.
pub const HOME_CHANGED: &str = "home_changed";

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

/// Only owner-local records can cause recovery writes. Receivers have none.
pub fn recover_summary_publications(store: &SharedStore) -> Result<(), StageError> {
    let guard = lock(store);
    for record in guard.sessions()? {
        let Some(value) = guard.result_state(&record.id, "summary-publication")? else {
            continue;
        };
        let local: PathBuf = serde_json::from_value(value)?;
        if !local.join(session_save::result::JOURNAL).exists() {
            continue;
        }
        let publication = session_save::result::load(&local)?;
        if record.home_path.as_deref().map(Path::new) != Some(publication.home.as_path()) {
            log::warn!("summary publication folder changed for {}", record.id);
            continue;
        }
        if publication.committed() {
            let doc = session_core::generation::manifest(&publication.home)?;
            guard.set_result_state(&record.id, "verified-manifest", &serde_json::to_value(doc)?)?;
            guard.clear_summary_not_updated(&record.id)?;
            publication.acknowledge(&local)?;
            guard.clear_result_state(&record.id, "summary-publication")?;
            std::fs::remove_dir_all(local)?;
        } else if let Err(error) = publication.rollback() {
            log::warn!("summary publication conflict for {}: {error}", record.id);
        }
    }
    Ok(())
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
    let destination =
        destination.ok_or_else(|| StageError::NoDestination("no storage folder is set".into()))?;
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
    if working.join(session_save::result::JOURNAL).exists() {
        let publication = session_save::result::load(working)?;
        if publication.home != home {
            return Err(StageError::Failed(
                "the note folder moved during publication".into(),
            ));
        }
        if publication.committed() {
            checkpoint::put(
                store,
                session_id,
                Stage::Save,
                STATE,
                &SaveState::Done {
                    path: home.to_path_buf(),
                },
            )?;
        }
    }
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
    let mut home = home.to_path_buf();
    if !finished {
        checkpoint::put(store, session_id, Stage::Save, HOME_CHANGED, &true)?;
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
            let guard = lock(store);
            let now = run_home(&guard, session_id)?;
            if now == home {
                break guard;
            }
            drop(guard);
            home = now;
        };
        // Under the store lock: the documents go into the folder where the
        // note is now, and the notes written are the ones that the folder
        // and this computer hold now.
        follow_run(&mut guard, session_id)?;
        let summary_notes: Option<String> = guard
            .checkpoints(session_id, Stage::Summarize)?
            .into_iter()
            .find(|(key, _)| key == "source-notes")
            .map(|(_, value)| serde_json::from_str(&value))
            .transpose()?;
        if summary_notes.as_ref().is_some_and(|hash| {
            *hash != folder::fingerprint(guard.notes(session_id).unwrap_or_default().as_bytes())
        }) {
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
        let expected = match expected {
            Some((path, hash)) if path == home => hash,
            Some(_) => {
                return Err(StageError::Failed(
                    "the note folder moved during processing; the staged result was kept".into(),
                ));
            }
            None => folder::fingerprint(&std::fs::read(home.join(SESSION_FILE))?),
        };
        let doc = session_doc(&record, segments, working)?;
        let publication = session_save::result::prepare(&home, working, working, doc, &expected)?;
        // This marker prevents generic failure cleanup from discarding the
        // owner-local journal. It does not prove that commit occurred.
        guard.put_checkpoint(session_id, Stage::Save, HOME_CHANGED, "true")?;
        drop(guard);
        loop {
            if publication.committed() {
                break;
            }
            if let Err(error) = control.proceed() {
                publication.rollback()?;
                return Err(error);
            }
            if publication.step()? {
                break;
            }
        }
        let done = SaveState::Done { path: home.clone() };
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
    let published = session_core::generation::manifest(&home)?;
    guard.promote_result_snapshot(session_id, published.recognition_snapshot.as_ref())?;
    guard.set_speaker_count(session_id, published.speaker_count)?;
    guard.set_result_state(
        session_id,
        "verified-manifest",
        &serde_json::to_value(&published)?,
    )?;
    guard.apply(session_id, &Event::Saved)?;
    if working.join(session_save::result::JOURNAL).exists() {
        session_save::result::load(working)?.acknowledge(working)?;
    }
    if working.exists() {
        std::fs::remove_dir_all(working)?;
    }
    let path = reconcile_after_done(&guard, session_id, home.to_path_buf(), fs);
    Ok(Saved {
        path,
        copied: false,
    })
}

/// Copy each segment's audio that `home` does not hold yet from the working
/// copy. The old audio is never rewritten.
fn add_audio(home: &Path, working: &Path, segments: &[Segment]) -> Result<(), StageError> {
    if !home.is_dir() {
        return Err(StageError::NoDestination(format!(
            "the note's folder {} is missing; put it back to finish",
            home.display()
        )));
    }
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
        write_working(working, SUMMARY_FILE, &stub(&why).render())?;
    }
    // `session.md` last, with the fingerprints of the files written above.
    let mut doc = session_doc(&record, segments, working)?;
    doc.result_generation = Some(session_save::result::stage(working, working)?);
    write_working(working, SESSION_FILE, &doc.render())?;
    Ok(())
}

/// The contents of `session.md`: the session's record, and the fingerprints
/// of the `transcript.md` and `summary.md` that are in `folder` now (KTD2).
/// Write `session.md` after those files, so that the fingerprints match.
pub fn session_doc(
    record: &SessionRecord,
    segments: &[Segment],
    folder: &Path,
) -> Result<SessionDoc, StageError> {
    let mut doc = SessionDoc::new(&record.id, record.speaker_count, segments)
        .ok_or_else(|| StageError::Failed("the note has no audio".into()))?;
    doc.title = record.title.as_ref().map(|t| t.text.clone());
    doc.title_source = record.title.as_ref().map(|t| t.source);
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
    if let Ok(previous) = session_core::generation::manifest(folder)
        && previous.transcript_fingerprint == doc.transcript_fingerprint
        && previous.summary_fingerprint == doc.summary_fingerprint
    {
        doc.result_generation = previous.result_generation;
    }
    let artifact = folder.join(session_core::recognition::ARTIFACT_FILE);
    if artifact.exists()
        && session_core::generation::manifest(folder)
            .map_or(true, |doc| doc.recognition_artifact.is_some())
    {
        let reference = session_core::recognition::ArtifactReference {
            version: session_core::recognition::ARTIFACT_VERSION,
            filename: session_core::recognition::ARTIFACT_FILE.into(),
            sha256: session_core::recognition::file_digest(&artifact)?,
        };
        let header = session_core::recognition::read_artifact(&artifact, &reference, |_| Ok(()))
            .map_err(|e| StageError::Failed(e.to_string()))?;
        if doc.transcript_fingerprint.as_ref() != Some(&header.transcript) {
            return Err(StageError::Failed(
                "recognition artifact does not match the transcript".into(),
            ));
        }
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
    if let Ok(doc) = session_core::generation::manifest(folder) {
        store.set_result_state(session_id, "verified-manifest", &serde_json::to_value(doc)?)?;
    }
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

/// Summarize a saved session again and replace its `summary.md` (R19,
/// AE17): a stub left by a failed summary, or a summary the user wants
/// redone. A rename that waited for the regeneration then moves the
/// folder, whether the regeneration succeeded or not (KTD8). The returned
/// path is in the folder under its name after that rename.
#[allow(clippy::too_many_arguments)]
pub fn regenerate_summary(
    store: &SharedStore,
    session_id: &str,
    working_root: &Path,
    endpoint: SummaryEndpoint,
    active: Prompt,
    learn: &dyn Fn(u32),
    control: &dyn Control,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PathBuf, StageError> {
    let regenerated = summarize_again(
        store,
        session_id,
        working_root,
        endpoint,
        active,
        learn,
        control,
        progress,
    );
    // The regeneration has ended, and the caller holds the one processing
    // permit, so no other regeneration runs.
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
    active: Prompt,
    learn: &dyn Fn(u32),
    control: &dyn Control,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PathBuf, StageError> {
    let not_saved = || {
        StageError::Failed(
            "a summary can be regenerated only once the note has finished processing".into(),
        )
    };
    // The row follows its folder first (KTD3): the summary uses the notes
    // and the title that another computer wrote. The caller holds the
    // regeneration claim.
    let (home, segments) = {
        let mut guard = lock(store);
        home_path(&guard, session_id)?.ok_or_else(not_saved)?;
        crate::follow::follow_folder(&mut guard, session_id, false)?;
        let home = home_path(&guard, session_id)?.ok_or_else(not_saved)?;
        (home, guard.segments(session_id)?)
    };
    // Regenerate Summary explicitly accepts the current transcript text.
    // An external edit removes the reusable recognition reference.
    let mut current = session_core::generation::manifest(&home)?;
    let canonical = std::fs::read(home.join(TRANSCRIPT_FILE))
        .map_err(|e| unreadable(&home.join(TRANSCRIPT_FILE), &e.to_string()))?;
    let current_hash = folder::fingerprint(&canonical);
    if current.transcript_fingerprint.as_ref() != Some(&current_hash)
        || current
            .result_generation
            .as_ref()
            .is_some_and(|g| g.files.get(TRANSCRIPT_FILE) != Some(&current_hash))
    {
        current.transcript_fingerprint = Some(current_hash);
        current.summary_fingerprint = fingerprint_of(&home, SUMMARY_FILE)?;
        current.recognition_artifact = None;
        current.recognition_audio = None;
        current.result_generation = Some(session_save::result::stage(&home, &home)?);
        home_write::write_atomic(&home, SESSION_FILE, current.render().as_bytes())?;
    }
    // A stub resumes from the calls made before the failure, but only when
    // they were made with the active prompt's text (KTD5), and from the
    // transcript and the notes that the folder holds now
    // ([`summarize::INPUT`]). A complete
    // summary, or a stub made with another prompt or with no saved prompt,
    // starts over. The title call always runs again, so a new proposal
    // follows (R31). A title that the user set stays.
    let complete = checkpoint::is_complete(store, session_id, Stage::Summarize, segments.len())?;
    let saved = checkpoint::get::<Prompt>(store, session_id, Stage::Summarize, summarize::PROMPT)?;
    let same_prompt = saved.is_some_and(|p| p.id == active.id && p.text == active.text);
    if complete || !same_prompt {
        summarize::clear(store, session_id)?;
    } else {
        lock(store).delete_checkpoint(session_id, Stage::Summarize, summarize::TITLE)?;
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
    let expected = folder::fingerprint(&std::fs::read(home.join(SESSION_FILE))?);
    let source_notes = folder::fingerprint(lock(store).notes(session_id)?.as_bytes());
    let summary = summarize::produce(
        store, session_id, &home, endpoint, active, learn, control, progress,
    )?;
    // The home folder is read again under the lock, just before the write,
    // and the lock is held through the write.
    let path = {
        let guard = lock(store);
        let path = home_path(&guard, session_id)?.ok_or_else(not_saved)?;
        // The model calls can take long. A user title that another
        // computer wrote in the meantime goes into the row first, so the
        // `session.md` below keeps it.
        if path != home || source_notes != folder::fingerprint(guard.notes(session_id)?.as_bytes())
        {
            return Err(StageError::Failed(
                "the note changed while its summary was being made; the previous result was kept"
                    .into(),
            ));
        }
        keep_folder_user_title(&guard, session_id, &path)?;
        let local = tempfile::Builder::new()
            .prefix(".longhand-summary-")
            .tempdir_in(working_root)?
            .keep();
        guard.set_result_state(
            session_id,
            "summary-publication",
            &serde_json::to_value(&local)?,
        )?;
        let previous = session_core::generation::manifest(&path)?;
        for name in [TRANSCRIPT_FILE, session_core::recognition::ARTIFACT_FILE] {
            if name == session_core::recognition::ARTIFACT_FILE
                && previous.recognition_artifact.is_none()
            {
                continue;
            }
            let bytes = session_core::generation::read(&path, &previous, name)?;
            home_write::write_atomic(&local, name, &bytes)?;
        }
        home_write::write_atomic(&local, SUMMARY_FILE, summary.render().as_bytes())?;
        let record = guard.session(session_id)?.ok_or_else(not_saved)?;
        let mut doc = session_doc(&record, &segments, &local)?;
        doc.recognition_snapshot = previous.recognition_snapshot;
        let publication = session_save::result::prepare(&path, &local, &local, doc, &expected)?;
        drop(guard);
        while !publication.committed() {
            if let Err(e) = control.proceed() {
                publication.rollback()?;
                return Err(e);
            }
            publication.step()?;
        }
        let guard = lock(store);
        let published = session_core::generation::manifest(&path)?;
        guard.set_result_state(
            session_id,
            "verified-manifest",
            &serde_json::to_value(&published)?,
        )?;
        publication.acknowledge(&local)?;
        guard.clear_result_state(session_id, "summary-publication")?;
        std::fs::remove_dir_all(local)?;
        path
    };
    checkpoint::mark_complete(store, session_id, Stage::Summarize, segments.len())?;
    let mut guard = lock(store);
    guard.delete_checkpoint(session_id, Stage::Summarize, SUMMARY_FAILED)?;
    guard.clear_summary_not_updated(session_id)?;
    guard.apply(session_id, &Event::SavedEdit)?;
    Ok(path.join(SUMMARY_FILE))
}

/// Store the user title of the `session.md` in `folder` as the row's
/// title, when the row has another title. A title that the folder gives
/// with no source is the user's when it is not the stored one (R5). Only a
/// user title is taken: a proposed title in the folder is older than the
/// proposal that the regeneration stored. A `session.md` that cannot be
/// read or parsed changes nothing. The caller holds the store lock.
fn keep_folder_user_title(
    store: &Store,
    session_id: &str,
    folder: &Path,
) -> Result<(), StageError> {
    let Read::Bytes(bytes) = read_timed(folder.join(SESSION_FILE), READ_TIMEOUT) else {
        return Ok(());
    };
    let Some(doc) = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| SessionDoc::parse(text).ok())
    else {
        return Ok(());
    };
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
