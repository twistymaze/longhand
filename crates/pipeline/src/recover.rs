//! Launch recovery of sessions a dead process left recording (U16, R8).
//!
//! The store still says Capturing, and the segment that was open has no
//! row: its chunks on disk, named by segment and timeline position, are the
//! record of it. The sweep closes any `.part` from its length, rebuilds the
//! segment from the chunk names, and interrupts the session with a crash
//! reason as of when its audio ends — the wall-clock moment the auto-queue
//! wait (R5e) is measured from, where the timeline position could not say
//! how long the process was dead.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, FixedOffset, TimeZone};
use session_core::{Event, InterruptReason, MIN_SEGMENT_MS, Segment, SessionState};

use crate::{RECOVERY_DIR, SharedStore, StageError, lock};

/// Where a segment's archive goes in the session folder.
pub fn segment_audio_file(index: u32) -> String {
    format!("audio/segment-{:02}.opus", index + 1)
}

/// A session the sweep found recording under a dead process.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovered {
    pub note_id: String,
    pub state: SessionState,
}

/// A session that the sweep could not recover. It keeps its files.
#[derive(Debug)]
pub struct Failed {
    pub note_id: String,
    pub error: StageError,
}

/// What [`recover_crashed`] did at one launch.
#[derive(Debug, Default)]
pub struct Recovery {
    /// The sessions that the sweep recovered.
    pub recovered: Vec<Recovered>,
    /// The sessions that the sweep could not recover, each with its error.
    pub failed: Vec<Failed>,
}

/// Recover every session left Capturing. Run at launch, before any writer
/// starts, while nothing in this process is recording. Then register the
/// chunks that an Interrupted session keeps with no segment row, and delete
/// the old audio copies that ended sessions left
/// ([`crate::old_audio::sweep`]).
///
/// An error in one session does not stop the others or the sweep. That
/// session keeps its files and becomes Interrupted(crash) when the store
/// can write it, so the failed-capture alert shows at launch. Only an error
/// that reads the list of sessions is returned.
pub fn recover_crashed(store: &SharedStore, working_root: &Path) -> Result<Recovery, StageError> {
    let capturing = lock(store).sessions_in("capturing")?;
    let interrupted = lock(store).sessions_in("interrupted")?;
    let mut report = Recovery::default();
    for record in capturing {
        let id = record.id;
        let session_dir = working_root.join(&id);
        let state = match recover_session(store, &id, &session_dir, record.created_at) {
            Ok(state) => state,
            Err(error) => {
                log::error!("could not recover session {id} after a crash: {error}");
                interrupt_failed(store, &id, record.created_at);
                report.failed.push(Failed { note_id: id, error });
                continue;
            }
        };
        log::info!(
            "session {id} was recording when Longhand stopped; now {}",
            state.name()
        );
        report.recovered.push(Recovered { note_id: id, state });
    }
    // A failed store write or an earlier failed recovery can leave chunks
    // with no segment row in an Interrupted session. Their rows are added,
    // and the state does not change (KTD3 of the capture recovery plan).
    for record in interrupted {
        let session_dir = working_root.join(&record.id);
        if let Err(e) = register_chunks(store, &record.id, &session_dir, None) {
            log::error!(
                "could not register the audio of interrupted session {}: {e}",
                record.id
            );
        }
    }
    // A session that ended while Longhand was stopped keeps no copy of its
    // old audio (KTD8 of the audio removal plan).
    if let Err(e) = crate::old_audio::sweep(store, working_root) {
        log::error!("the old audio sweep at launch failed: {e}");
    }
    Ok(report)
}

fn recover_session(
    store: &SharedStore,
    id: &str,
    session_dir: &Path,
    created_at: DateTime<FixedOffset>,
) -> Result<SessionState, StageError> {
    if crate::early_home::home_deleted(store, id)? {
        // Another computer deleted the note while this one recorded into
        // it: a person's Delete wins, and the recording goes (R23). No
        // folder is made again. The check for unregistered audio does not
        // apply: a person chose to delete it.
        log::warn!("the folder of {id} was deleted on another computer");
        let state = lock(store).apply(
            id,
            &Event::UserStop {
                at: created_at,
                retained_audio: false,
            },
        )?;
        remove_working_folder(&state, session_dir);
        return Ok(state);
    }
    recover_one(store, id, session_dir, created_at)
}

/// A discarded recording and a note with no audio keep no working folder.
fn remove_working_folder(state: &SessionState, session_dir: &Path) {
    if matches!(state, SessionState::Discarded | SessionState::Saved) {
        let _ = std::fs::remove_dir_all(session_dir);
    }
}

/// The wall-clock end of the segment with the highest index, or `None` when
/// there is no segment.
pub fn last_segment_end(segments: &[Segment]) -> Option<DateTime<FixedOffset>> {
    segments.iter().max_by_key(|s| s.index).map(|s| s.wall_end)
}

/// Make a session whose recovery failed Interrupted(crash), as of the end
/// of its last stored segment, or its creation when it has none. When the
/// store cannot write it, the session stays Capturing, and the next launch
/// tries to recover it again.
fn interrupt_failed(store: &SharedStore, id: &str, created_at: DateTime<FixedOffset>) {
    let mut store = lock(store);
    let at = store
        .segments(id)
        .ok()
        .and_then(|segments| last_segment_end(&segments))
        .unwrap_or(created_at);
    let event = Event::CaptureLost {
        reason: InterruptReason::Crash,
        at,
    };
    if let Err(e) = store.apply(id, &event) {
        log::error!("could not interrupt session {id} after its recovery failed: {e}");
    }
}

fn recover_one(
    store: &SharedStore,
    id: &str,
    session_dir: &Path,
    created_at: DateTime<FixedOffset>,
) -> Result<SessionState, StageError> {
    register_chunks(store, id, session_dir, None)?;
    let mut guard = lock(store);
    let segments = guard.segments(id)?;
    // A `.part` that no repair could read has no row. The session keeps it,
    // and a later launch tries it again (R6).
    let unregistered = unregistered_audio(session_dir, &segments);
    // The last segment is the one that was recording.
    let ended = last_segment_end(&segments);
    if ended.is_none() && !unregistered {
        // Nothing survived R35: the note is a note with no audio again, as
        // after a Stop with no segment (R19). A note that existed before
        // the recording or has notes text stays. The removal of empty notes
        // owns an empty note (U6).
        let state = guard.apply(id, &Event::RecordingDropped)?;
        drop(guard);
        remove_working_folder(&state, session_dir);
        return Ok(state);
    }
    Ok(guard.apply(
        id,
        &Event::CaptureLost {
            reason: InterruptReason::Crash,
            at: ended.unwrap_or(created_at),
        },
    )?)
}

/// Register as segments the closed chunks in the session's
/// `audio-recovery/` folder that have no segment row (KTD2 of the capture
/// recovery plan). First close each `.part` that a writer left. A segment
/// under a second is removed with its chunks (R35), except the segment
/// `keep`. A rebuilt segment does not end in a pause.
///
/// Each step locks the store on its own, so the caller must hold no store
/// guard. Call only while no writer runs on the folder. A `.part` that
/// cannot be repaired stays in place: [`unregistered_audio`] finds it.
pub fn register_chunks(
    store: &SharedStore,
    id: &str,
    session_dir: &Path,
    keep: Option<u32>,
) -> Result<(), StageError> {
    let dir = session_dir.join(RECOVERY_DIR);
    let partial = recorder::recover_partial_chunks(&dir)?;
    if !partial.left.is_empty() {
        log::warn!(
            "{} partial chunks of {id} could not be closed and stay in place",
            partial.left.len()
        );
    }
    // The segments of the `.part` files that stay in place.
    let left: Vec<u32> = partial
        .left
        .iter()
        .filter_map(|path| recorder::chunk_segment_of(&path.file_name()?.to_string_lossy()))
        .collect();
    let chunks = recorder::list_chunks(&dir)?;
    let known: Vec<u32> = lock(store).segments(id)?.iter().map(|s| s.index).collect();

    let mut open: BTreeMap<u32, Vec<&recorder::ChunkInfo>> = BTreeMap::new();
    for chunk in chunks.iter().filter(|c| !known.contains(&c.segment)) {
        open.entry(chunk.segment).or_default().push(chunk);
    }
    for (index, mut of) in open {
        of.sort_by_key(|c| c.start_ms);
        let (Some(first), Some(last)) = (of.first(), of.last()) else {
            continue;
        };
        let offset_ms = first.start_ms;
        let duration_ms = last.end_ms.saturating_sub(offset_ms);
        if duration_ms < MIN_SEGMENT_MS && keep != Some(index) {
            // The removal deletes every file of the segment, also a `.part`
            // that the repair left. Its audio can be longer than a second,
            // so the segment and its files stay, and a later launch tries
            // the `.part` again.
            if left.contains(&index) {
                log::warn!(
                    "segment {index} of {id} has a partial chunk that could not be closed; its files stay"
                );
                continue;
            }
            recorder::remove_segment(&dir, index)?;
            crate::live::delete_segment(store, id, index)?;
            log::info!("segment {index} of {id} was under a second and was dropped");
            continue;
        }
        // The segment's audio is its time: the wall clock at its start plus
        // its length is when the last sample was written.
        let wall_start = match wall(first.capture_started_at) {
            Some(at) => at,
            None => session_created_at(store, id)?,
        };
        // The open segment has no row, so its channel count comes from the
        // first chunk's WAV header (KTD6 of the system audio plan).
        let channels = recorder::chunk_channels(first)?;
        let segment = Segment {
            index,
            offset_ms,
            duration_ms,
            wall_start,
            wall_end: wall_start + chrono::TimeDelta::milliseconds(duration_ms as i64),
            audio_file: segment_audio_file(index),
            channels,
            ends_in_pause: false,
        };
        let store = lock(store);
        store.put_segment(id, &segment)?;
        // The first stereo segment that is kept sets the stereo mark (KTD2).
        if channels == 2 && store.mark_stereo_from(id, index)? {
            log::info!("segment {index} of {id} is the note's first stereo segment");
        }
    }
    Ok(())
}

/// Whether the session's `audio-recovery/` folder holds a `.part`, or a
/// closed chunk whose segment index has no row in `segments` (KTD1 of the
/// capture recovery plan). A `.part` counts whatever its index, because no
/// row includes its audio. Call only while no writer runs on the folder:
/// the open chunk of a running writer is a `.part`. A working folder that
/// can follow a capture is deleted only when this is false. When the
/// folder cannot be read, the result is true, so the folder stays.
pub fn unregistered_audio(session_dir: &Path, segments: &[Segment]) -> bool {
    let dir = session_dir.join(RECOVERY_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(e) => {
            log::warn!("could not read {} ({e}); its audio stays", dir.display());
            return true;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                log::warn!(
                    "could not read an entry of {} ({e}); its audio stays",
                    dir.display()
                );
                return true;
            }
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(index) = recorder::chunk_segment_of(&name) else {
            continue;
        };
        // No row names the audio of a `.part`, also when its segment has a
        // row: a row covers only the closed chunks.
        if name.ends_with(".part") || !segments.iter().any(|s| s.index == index) {
            return true;
        }
    }
    false
}

/// When the session was created.
fn session_created_at(store: &SharedStore, id: &str) -> Result<DateTime<FixedOffset>, StageError> {
    lock(store)
        .session(id)?
        .map(|record| record.created_at)
        .ok_or_else(|| StageError::Failed(format!("no session {id}")))
}

/// A chunk name's capture start, milliseconds since the epoch, as local time.
fn wall(millis: u64) -> Option<DateTime<FixedOffset>> {
    chrono::Local
        .timestamp_millis_opt(i64::try_from(millis).ok()?)
        .single()
        .map(|t| t.fixed_offset())
}
