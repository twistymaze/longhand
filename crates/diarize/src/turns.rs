//! Turn smoothing (KTD7): the stack's raw speaker segments become the turns
//! the transcript is built on. Written for Longhand.
//!
//! Within each recorded segment, and never across a gap:
//! - same-speaker segments less than [`MERGE_GAP_MS`] apart are merged;
//! - a turn shorter than [`MIN_TURN_MS`] is absorbed into the neighbour it
//!   overlaps more.
//!
//! Then speakers are numbered `1..N` in order of first appearance across the
//! whole session.

use serde::{Deserialize, Serialize};

use crate::SpeakerSegment;

pub const MERGE_GAP_MS: u64 = 300;
pub const MIN_TURN_MS: u64 = 500;

/// A speaker turn inside one recorded segment, in ms within that segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub segment: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    /// 1-based, in first-appearance order across the session.
    pub speaker: u32,
}

impl Turn {
    fn duration(&self) -> u64 {
        self.end_ms - self.start_ms
    }

    fn overlap(&self, other: &Turn) -> u64 {
        self.end_ms
            .min(other.end_ms)
            .saturating_sub(self.start_ms.max(other.start_ms))
    }

    fn gap_to(&self, other: &Turn) -> u64 {
        if other.start_ms >= self.end_ms {
            other.start_ms - self.end_ms
        } else {
            self.start_ms.saturating_sub(other.end_ms)
        }
    }
}

/// Smooth each segment's raw speaker segments (seconds within the segment,
/// session-wide speaker indices) and number the speakers.
pub fn smooth(per_segment: &[(u32, Vec<SpeakerSegment>)]) -> Vec<Turn> {
    let mut all = Vec::new();
    for (segment, raw) in per_segment {
        let mut turns: Vec<Turn> = raw
            .iter()
            .map(|s| Turn {
                segment: *segment,
                start_ms: (s.start * 1000.0).round().max(0.0) as u64,
                end_ms: (s.end * 1000.0).round().max(0.0) as u64,
                // Raw index + 1 for now; renumbered below.
                speaker: s.speaker as u32 + 1,
            })
            .filter(|t| t.end_ms > t.start_ms)
            .collect();
        turns.sort_by_key(|t| (t.start_ms, t.speaker));
        merge_same_speaker(&mut turns);
        absorb_short(&mut turns);
        all.extend(turns);
    }
    renumber(&mut all);
    all
}

/// Merge same-speaker turns less than [`MERGE_GAP_MS`] apart, including
/// ones that overlap, keeping start order.
fn merge_same_speaker(turns: &mut Vec<Turn>) {
    let mut out: Vec<Turn> = Vec::with_capacity(turns.len());
    for turn in turns.drain(..) {
        // The latest earlier turn of the same speaker is the only candidate.
        if let Some(prev) = out.iter_mut().rev().find(|t| t.speaker == turn.speaker)
            && turn.start_ms <= prev.end_ms + MERGE_GAP_MS
        {
            prev.end_ms = prev.end_ms.max(turn.end_ms);
            continue;
        }
        out.push(turn);
    }
    out.sort_by_key(|t| (t.start_ms, t.speaker));
    *turns = out;
}

fn absorb_short(turns: &mut Vec<Turn>) {
    loop {
        let Some(i) = (0..turns.len()).find(|&i| turns[i].duration() < MIN_TURN_MS) else {
            return;
        };
        let short = turns[i];
        // Neighbours in time order that belong to someone else.
        let before = (0..i).rev().find(|&j| turns[j].speaker != short.speaker);
        let after = (i + 1..turns.len()).find(|&j| turns[j].speaker != short.speaker);
        let choice = match (before, after) {
            (Some(b), Some(a)) => {
                let (ob, oa) = (turns[b].overlap(&short), turns[a].overlap(&short));
                if ob != oa {
                    if ob > oa { b } else { a }
                } else if turns[b].gap_to(&short) <= turns[a].gap_to(&short) {
                    b
                } else {
                    a
                }
            }
            (Some(b), None) => b,
            (None, Some(a)) => a,
            // The only speaker in the segment: a short turn is still speech.
            (None, None) => return,
        };
        let target = &mut turns[choice];
        target.start_ms = target.start_ms.min(short.start_ms);
        target.end_ms = target.end_ms.max(short.end_ms);
        turns.remove(i);
        turns.sort_by_key(|t| (t.start_ms, t.speaker));
        merge_same_speaker(turns);
    }
}

fn renumber(turns: &mut [Turn]) {
    let mut order: Vec<u32> = Vec::new();
    for turn in turns.iter() {
        if !order.contains(&turn.speaker) {
            order.push(turn.speaker);
        }
    }
    for turn in turns.iter_mut() {
        turn.speaker = order.iter().position(|&s| s == turn.speaker).expect("seen") as u32 + 1;
    }
}

/// Speaker-change positions within `segment`, in ms, for the windowing stage.
pub fn turn_cuts(turns: &[Turn], segment: u32) -> Vec<u64> {
    let mut cuts = Vec::new();
    let mut previous: Option<&Turn> = None;
    for turn in turns.iter().filter(|t| t.segment == segment) {
        if let Some(p) = previous
            && p.speaker != turn.speaker
        {
            cuts.push(turn.start_ms);
        }
        previous = Some(turn);
    }
    cuts
}
