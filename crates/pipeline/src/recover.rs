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

/// Where a segment's archive lands in the session folder.
pub fn segment_audio_file(index: u32) -> String {
    format!("audio/segment-{:02}.opus", index + 1)
}

/// A session the sweep found recording under a dead process.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovered {
    pub session_id: String,
    pub state: SessionState,
}

/// Recover every session left Capturing. Run at launch, before any writer
/// starts, while nothing in this process is recording.
pub fn recover_crashed(
    store: &SharedStore,
    working_root: &Path,
) -> Result<Vec<Recovered>, StageError> {
    let capturing = lock(store).sessions_in("capturing")?;
    let mut recovered = Vec::new();
    for record in capturing {
        let id = record.id;
        let session_dir = working_root.join(&id);
        let state = recover_one(store, &id, &session_dir, record.created_at)?;
        if state == SessionState::Discarded {
            let _ = std::fs::remove_dir_all(&session_dir);
        }
        log::info!(
            "session {id} was recording when Longhand stopped; now {}",
            state.name()
        );
        recovered.push(Recovered {
            session_id: id,
            state,
        });
    }
    Ok(recovered)
}

fn recover_one(
    store: &SharedStore,
    id: &str,
    session_dir: &Path,
    created_at: DateTime<FixedOffset>,
) -> Result<SessionState, StageError> {
    let dir = session_dir.join(RECOVERY_DIR);
    recorder::recover_partial_chunks(&dir)?;
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
        if duration_ms < MIN_SEGMENT_MS {
            recorder::remove_segment(&dir, index)?;
            log::info!("segment {index} of {id} was under a second and was dropped");
            continue;
        }
        // The segment's audio is its time: the wall clock at its start plus
        // its length is when the last sample was written.
        let wall_start = wall(first.capture_started_at).unwrap_or(created_at);
        let segment = Segment {
            index,
            offset_ms,
            duration_ms,
            wall_start,
            wall_end: wall_start + chrono::TimeDelta::milliseconds(duration_ms as i64),
            audio_file: segment_audio_file(index),
        };
        lock(store).put_segment(id, &segment)?;
    }

    let mut store = lock(store);
    let segments = store.segments(id)?;
    // The last segment is the one that was recording.
    let Some(ended) = segments.iter().max_by_key(|s| s.index).map(|s| s.wall_end) else {
        // Nothing survived R35: the session never held audio.
        return Ok(store.apply(
            id,
            &Event::UserStop {
                at: created_at,
                retained_audio: false,
            },
        )?);
    };
    Ok(store.apply(
        id,
        &Event::CaptureLost {
            reason: InterruptReason::Crash,
            at: ended,
        },
    )?)
}

/// A chunk name's capture start, milliseconds since the epoch, as local time.
fn wall(millis: u64) -> Option<DateTime<FixedOffset>> {
    chrono::Local
        .timestamp_millis_opt(i64::try_from(millis).ok()?)
        .single()
        .map(|t| t.fixed_offset())
}
