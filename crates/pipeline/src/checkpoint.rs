//! Checkpoint keys and payloads (R14, R20). Every payload is JSON in the
//! `checkpoints` table, so a run can be inspected with any SQLite tool.
//!
//! A stage is complete when its `done` checkpoint names as many segments as
//! the session has: a session resumed after processing gains a segment, and
//! only that segment's work is then outstanding.

use serde::{Deserialize, Serialize};
use session_core::Stage;

use crate::{SharedStore, StageError, lock};

pub const DONE: &str = "done";

pub fn segment_key(index: u32) -> String {
    format!("seg-{index:04}")
}

/// The echo spans of a stereo segment (KTD8), under `Stage::Assemble`.
pub fn echo_key(index: u32) -> String {
    format!("seg-{index:04}-echo")
}

pub fn plan_key(index: u32) -> String {
    format!("seg-{index:04}-plan")
}

pub fn window_key(index: u32, window: usize) -> String {
    format!("seg-{index:04}-win-{window:05}")
}

pub fn window_prefix(index: u32) -> String {
    format!("seg-{index:04}-win-")
}

/// A part of the live transcript (KTD7). Live keys do not start with
/// [`window_prefix`], so a live row never counts as a finished window.
pub fn live_key(index: u32, part: u32) -> String {
    format!("seg-{index:04}-live-{part:04}")
}

pub fn live_prefix(index: u32) -> String {
    format!("seg-{index:04}-live-")
}

pub fn segment_done_key(index: u32) -> String {
    format!("seg-{index:04}-done")
}

/// The segment index of a `seg-NNNN-...` key. Other keys give `None`. An
/// index of 10000 or more has more than four digits.
pub fn segment_of(key: &str) -> Option<u32> {
    let rest = key.strip_prefix("seg-")?;
    let digits = rest.split('-').next()?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageDone {
    pub segments: usize,
}

pub fn get<T: for<'de> Deserialize<'de>>(
    store: &SharedStore,
    note_id: &str,
    stage: Stage,
    key: &str,
) -> Result<Option<T>, StageError> {
    lock(store)
        .checkpoint_value(note_id, stage, key)?
        .map(|payload| serde_json::from_str(&payload).map_err(StageError::from))
        .transpose()
}

pub fn put<T: Serialize>(
    store: &SharedStore,
    note_id: &str,
    stage: Stage,
    key: &str,
    value: &T,
) -> Result<(), StageError> {
    let payload = serde_json::to_string(value)?;
    lock(store).put_checkpoint(note_id, stage, key, &payload)?;
    Ok(())
}

/// Every checkpoint of a stage, key-ordered.
pub fn all(
    store: &SharedStore,
    note_id: &str,
    stage: Stage,
) -> Result<Vec<(String, String)>, StageError> {
    Ok(lock(store).checkpoints(note_id, stage)?)
}

pub fn is_complete(
    store: &SharedStore,
    note_id: &str,
    stage: Stage,
    segments: usize,
) -> Result<bool, StageError> {
    Ok(get::<StageDone>(store, note_id, stage, DONE)?.is_some_and(|d| d.segments == segments))
}

pub fn mark_complete(
    store: &SharedStore,
    note_id: &str,
    stage: Stage,
    segments: usize,
) -> Result<(), StageError> {
    put(store, note_id, stage, DONE, &StageDone { segments })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The keys of segment 10000 and after have more than four digits.
    #[test]
    fn segment_of_reads_the_index_of_each_segment_key() {
        assert_eq!(segment_of(&segment_key(3)), Some(3));
        assert_eq!(segment_of(&window_key(7, 3)), Some(7));
        assert_eq!(segment_of(&plan_key(12_345)), Some(12_345));
        assert_eq!(segment_of(&window_key(10_000, 0)), Some(10_000));
        assert_eq!(segment_of(&segment_done_key(42)), Some(42));
        assert_eq!(segment_of("recognition-generation-v1"), None);
        assert_eq!(segment_of("seg-"), None);
        assert_eq!(segment_of("seg-+12-done"), None);
    }
}
