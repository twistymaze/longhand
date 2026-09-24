//! Note anchors (R12, R12a, R13).
//!
//! A note is anchored at its first keystroke's position on the session
//! timeline, minus a lookback so it lands on the subject it refers to. The
//! anchor is fixed when resolved and never moves when the text is edited.

use serde::{Deserialize, Serialize};

use crate::timeline::{Location, Timeline};

/// Provisional lookback; tuned once real sessions exist (U5).
pub const DEFAULT_LOOKBACK_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Anchor {
    /// Typed before the first segment, after the session stopped, or while no
    /// session was open.
    Unanchored,
    At {
        position_ms: u64,
        /// The first keystroke fell in a gap, so the anchor is the end of the
        /// preceding segment rather than a moment that was recorded.
        gap_anchored: bool,
    },
}

/// Resolve the anchor for a keystroke at `keystroke_ms` on the session
/// timeline. `None` means no session position exists (nothing recorded yet,
/// or the session has stopped).
pub fn resolve(timeline: &Timeline, keystroke_ms: Option<u64>, lookback_ms: u64) -> Anchor {
    let Some(keystroke_ms) = keystroke_ms else {
        return Anchor::Unanchored;
    };
    match timeline.locate(keystroke_ms) {
        Location::BeforeStart => Anchor::Unanchored,
        Location::InGap { after } => match timeline.segment_end(after) {
            Ok(end) => Anchor::At {
                position_ms: end,
                gap_anchored: true,
            },
            Err(_) => Anchor::Unanchored,
        },
        Location::InSegment { .. } => {
            let target = keystroke_ms.saturating_sub(lookback_ms);
            let position_ms = match timeline.locate(target) {
                // A lookback that reaches back into a gap stops at the end of
                // the segment before it (R12a).
                Location::InGap { after } => timeline.segment_end(after).unwrap_or(target),
                // Or before the recording began: the start is as far back as
                // there is anything to point at.
                Location::BeforeStart => timeline
                    .segments()
                    .first()
                    .map(|s| s.offset_ms)
                    .or_else(|| timeline.open_segment().map(|o| o.offset_ms))
                    .unwrap_or(0),
                Location::InSegment { .. } => target,
            };
            Anchor::At {
                position_ms,
                gap_anchored: false,
            }
        }
    }
}
