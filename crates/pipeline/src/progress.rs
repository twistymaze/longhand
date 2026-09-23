//! Where a session's processing stands, read from the store alone (U12,
//! AE4): a window shown mid-stage rebuilds its view from this, then follows
//! the runner's events.

use serde::{Deserialize, Serialize};
use session_core::Stage;

use crate::runner::STAGES;
use crate::{SharedStore, StageError, checkpoint, lock};

/// Under a stage: the message it last failed or was blocked with.
pub const ERROR: &str = "error";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Stages whose work is complete, in order.
    pub done: Vec<Stage>,
    /// The stage started last, if processing has begun.
    pub current: Option<Stage>,
    /// Transcription windows done and planned, once any are planned.
    pub windows: Option<(usize, usize)>,
}

pub fn snapshot(store: &SharedStore, session_id: &str) -> Result<Snapshot, StageError> {
    let segments = lock(store).segments(session_id)?.len();
    let mut done = Vec::new();
    for &stage in STAGES {
        if checkpoint::is_complete(store, session_id, stage, segments)? {
            done.push(stage);
        }
    }
    let current = lock(store).current_stage(session_id)?;
    let rows = checkpoint::all(store, session_id, Stage::Transcribe)?;
    let mut planned = 0;
    let mut finished = 0;
    for (key, payload) in &rows {
        if key.ends_with("-plan") {
            let plan: crate::transcribe::WindowPlan = serde_json::from_str(payload)?;
            planned += plan.windows.len();
        } else if key.contains("-win-") {
            finished += 1;
        }
    }
    Ok(Snapshot {
        done,
        current,
        windows: (planned > 0).then_some((finished.min(planned), planned)),
    })
}

/// Remember why `stage` stopped, for a view opened later.
pub fn record_error(
    store: &SharedStore,
    session_id: &str,
    stage: Stage,
    message: &str,
) -> Result<(), StageError> {
    checkpoint::put(store, session_id, stage, ERROR, &message)
}

pub fn error(
    store: &SharedStore,
    session_id: &str,
    stage: Stage,
) -> Result<Option<String>, StageError> {
    checkpoint::get(store, session_id, stage, ERROR)
}
