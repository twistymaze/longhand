//! The land stage (U11, KD4, KD13, R22, R22e, R22f): render the session's
//! markdown beside its audio, then move it into the destination through the
//! `landing` crate, recording each landing state before the next step so a
//! crash resumes from what is on disk.
//!
//! Landing claims the session first, under the store lock: from then on a
//! resume is refused, since the working directory is about to leave. The
//! claim is released if landing fails before anything moved.

use std::path::{Path, PathBuf};

use landing::{Fs, Job, LandState, LandingError, post_landing};
use session_core::folder::{self, NoteLine, SessionDoc, Summary, SummaryMeta};
use session_core::{Event, Segment, SessionState, Stage, Store};

use crate::summarize::{self, SUMMARY_FILE, SummaryEndpoint};
use crate::{Control, SharedStore, StageError, checkpoint, lock};

pub const SESSION_FILE: &str = "session.md";
pub const NOTES_FILE: &str = "notes.md";
/// The landing's progress, a [`LandState`]. Its presence means landing has
/// begun and the session can no longer be resumed.
pub const STATE: &str = "state";
/// Under the summarize stage: why it failed, for the landed stub (KD13).
pub const SUMMARY_FAILED: &str = "failed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Landed {
    pub path: PathBuf,
    /// The destination is on another volume: the session was copied and
    /// verified rather than moved in one step.
    pub copied: bool,
}

/// Whether landing has begun for `session_id` (a resume must refuse).
pub fn begun(store: &Store, session_id: &str) -> Result<bool, StageError> {
    Ok(store
        .checkpoints(session_id, Stage::Land)?
        .iter()
        .any(|(key, _)| key == STATE))
}

/// Record why summarization failed; the session lands with a stub.
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
) -> Result<Landed, StageError> {
    // Only a verified transcript lands (KD13).
    if !checkpoint::is_complete(store, session_id, Stage::Verify, segments.len())? {
        return Err(StageError::Failed(
            "the transcript has not been verified, so the session stays in the working location"
                .into(),
        ));
    }
    let destination = destination
        .ok_or_else(|| StageError::NoDestination("no destination folder is set".into()))?;
    let started = segments
        .first()
        .ok_or_else(|| StageError::Failed("the session has no audio".into()))?
        .wall_start;
    let folder_name = landing::naming::folder_name(started);
    let job = Job {
        session_id,
        working,
        destination,
        folder_name: &folder_name,
    };

    let mut state = match checkpoint::get::<LandState>(store, session_id, Stage::Land, STATE)? {
        Some(state) => state,
        None => {
            control.proceed()?;
            claim(store, session_id)?;
            LandState::Working
        }
    };
    if state == LandState::Working && working.exists() {
        render(store, session_id, working, segments)?;
    }

    let mut copied = false;
    while !matches!(state, LandState::Done { .. }) {
        let next = match landing::step(&state, &job, fs) {
            Ok(next) => next,
            Err(e) => {
                if state == LandState::Working && working.exists() {
                    release(store, session_id);
                }
                return Err(match e {
                    LandingError::Destination { .. } | LandingError::StagingTaken(_) => {
                        StageError::NoDestination(e.to_string())
                    }
                    other => StageError::Failed(other.to_string()),
                });
            }
        };
        if let LandState::Landed {
            path,
            working_retained,
        } = &next
        {
            copied = *working_retained;
            lock(store).set_landed_path(session_id, &path.to_string_lossy())?;
        }
        checkpoint::put(store, session_id, Stage::Land, STATE, &next)?;
        state = next;
    }
    let LandState::Done { path } = state else {
        unreachable!("the loop ends on Done");
    };

    // Reopen notes against the landed folder (R22e): anything edited while
    // landing is in the store, and is written here. Under the lock, so an
    // edit lands either before this render or after the session is Landed,
    // where it rewrites the file itself.
    let mut guard = lock(store);
    write_notes(&guard, session_id, &path)?;
    guard.put_checkpoint(
        session_id,
        Stage::Land,
        checkpoint::DONE,
        &serde_json::to_string(&checkpoint::StageDone {
            segments: segments.len(),
        })?,
    )?;
    guard.apply(session_id, &Event::Landed)?;
    Ok(Landed { path, copied })
}

/// Take the session for landing, unless a resume took it first.
fn claim(store: &SharedStore, session_id: &str) -> Result<(), StageError> {
    let guard = lock(store);
    let processing = matches!(
        guard.session(session_id)?.map(|r| r.state),
        Some(SessionState::Processing { .. })
    );
    if !processing {
        return Err(StageError::Cancelled);
    }
    guard.put_checkpoint(
        session_id,
        Stage::Land,
        STATE,
        &serde_json::to_string(&LandState::Working)?,
    )?;
    Ok(())
}

/// Nothing moved: the session may be resumed again.
fn release(store: &SharedStore, session_id: &str) {
    if let Err(e) = lock(store).delete_checkpoint(session_id, Stage::Land, STATE) {
        log::warn!("could not release the landing claim on {session_id}: {e}");
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
    let (speaker_count, notes) = {
        let guard = lock(store);
        let record = guard
            .session(session_id)?
            .ok_or_else(|| StageError::Failed(format!("no session {session_id}")))?;
        (record.speaker_count, render_notes(&guard, session_id)?)
    };
    let doc = SessionDoc::new(session_id, speaker_count, segments)
        .ok_or_else(|| StageError::Failed("the session has no audio".into()))?;
    write_working(working, SESSION_FILE, &doc.render())?;
    write_working(working, NOTES_FILE, &notes)?;
    if !checkpoint::is_complete(store, session_id, Stage::Summarize, segments.len())? {
        let why = checkpoint::get::<String>(store, session_id, Stage::Summarize, SUMMARY_FAILED)?
            .unwrap_or_else(|| "summarization did not run".into());
        write_working(working, SUMMARY_FILE, &stub(&why).render())?;
    }
    Ok(())
}

pub fn stub(why: &str) -> Summary {
    Summary {
        meta: SummaryMeta::Stub {
            failed_stage: "summarize".into(),
        },
        body: format!(
            "# Summary\n\nNo summary was produced: {why}\n\nThe transcript and notes are complete. Regenerate the summary from the session in Longhand."
        ),
    }
}

fn render_notes(store: &Store, session_id: &str) -> Result<String, StageError> {
    let notes: Vec<NoteLine> = store
        .notes(session_id)?
        .into_iter()
        .map(|n| NoteLine {
            anchor: n.anchor,
            text: n.text,
        })
        .collect();
    Ok(folder::render_notes(&notes))
}

fn write_notes(store: &Store, session_id: &str, landed: &Path) -> Result<(), StageError> {
    let notes = render_notes(store, session_id)?;
    post_landing::write_atomic(landed, NOTES_FILE, notes.as_bytes())?;
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

/// The landed folder of a session, if it has landed.
fn landed_path(store: &Store, session_id: &str) -> Result<Option<PathBuf>, StageError> {
    Ok(store
        .session(session_id)?
        .filter(|r| r.state == SessionState::Landed)
        .and_then(|r| r.landed_path)
        .map(PathBuf::from))
}

/// A note of a landed session changed: rewrite its `notes.md` whole
/// (R22f). Returns false when the session has not landed, so there is no
/// file to write yet.
pub fn rewrite_notes(store: &mut Store, session_id: &str) -> Result<bool, StageError> {
    let Some(path) = landed_path(store, session_id)? else {
        return Ok(false);
    };
    write_notes(store, session_id, &path)?;
    store.apply(session_id, &Event::LandedEdit)?;
    Ok(true)
}

/// Summarize a landed session again and replace its `summary.md` (R19,
/// AE17): a stub left by a failed summary, or a summary the user wants
/// redone.
pub fn regenerate_summary(
    store: &SharedStore,
    session_id: &str,
    endpoint: SummaryEndpoint,
    learn: &dyn Fn(u32),
    control: &dyn Control,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PathBuf, StageError> {
    let (path, segments) = {
        let guard = lock(store);
        let path = landed_path(&guard, session_id)?.ok_or_else(|| {
            StageError::Failed("only a landed session's summary can be regenerated".into())
        })?;
        (path, guard.segments(session_id)?)
    };
    // A stub resumes from the calls made before the failure; a complete
    // summary starts over.
    if checkpoint::is_complete(store, session_id, Stage::Summarize, segments.len())? {
        summarize::clear(store, session_id)?;
    }
    let summary = summarize::produce(
        store, session_id, &segments, endpoint, learn, control, progress,
    )?;
    post_landing::write_atomic(&path, SUMMARY_FILE, summary.render().as_bytes())?;
    checkpoint::mark_complete(store, session_id, Stage::Summarize, segments.len())?;
    let mut guard = lock(store);
    guard.delete_checkpoint(session_id, Stage::Summarize, SUMMARY_FAILED)?;
    guard.apply(session_id, &Event::LandedEdit)?;
    Ok(path.join(SUMMARY_FILE))
}
