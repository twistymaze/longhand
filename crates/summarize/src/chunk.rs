//! Chunking a transcript that does not fit one call (KTD16). Whole turns
//! are packed under the allowance with about 10% overlap by turns. The
//! notes document goes whole with every call (KTD6), so the caller takes
//! its size from the allowance first. A turn longer than the allowance on
//! its own is split at sentence boundaries, each part keeping the speaker
//! label and start time.

use serde::{Deserialize, Serialize};

use crate::tokens::estimate;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    /// On the session timeline.
    pub position_ms: u64,
    /// `None` when diarization failed (R20a).
    pub speaker: Option<u32>,
    pub text: String,
}

/// The share of a chunk's turns repeated at the start of the next.
pub const OVERLAP: f64 = 0.10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub turns: Vec<Turn>,
    /// `turns[..first_new]` repeat the end of the previous chunk.
    pub first_new: usize,
}

pub fn format_clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

pub fn render_turn(turn: &Turn) -> String {
    match turn.speaker {
        Some(n) => format!(
            "[{}] Speaker {n}: {}",
            format_clock(turn.position_ms),
            turn.text
        ),
        None => format!("[{}] {}", format_clock(turn.position_ms), turn.text),
    }
}

/// A line's cost inside a prompt: its text and its newline.
fn line_cost(line: &str) -> usize {
    estimate(line) + 1
}

fn turn_cost(turn: &Turn) -> usize {
    line_cost(&render_turn(turn))
}

/// Split turns that cannot fit a chunk on their own.
fn split_long(turns: &[Turn], allowance: usize) -> Vec<Turn> {
    let mut out = Vec::with_capacity(turns.len());
    for turn in turns {
        if turn_cost(turn) <= allowance {
            out.push(turn.clone());
            continue;
        }
        let empty = Turn {
            text: String::new(),
            ..turn.clone()
        };
        let mut part = empty.clone();
        for piece in pieces(&turn.text, allowance.saturating_sub(turn_cost(&empty))) {
            let candidate = if part.text.is_empty() {
                piece.to_string()
            } else {
                format!("{} {piece}", part.text)
            };
            let fits = turn_cost(&Turn {
                text: candidate.clone(),
                ..empty.clone()
            }) <= allowance;
            if fits || part.text.is_empty() {
                part.text = candidate;
            } else {
                out.push(std::mem::replace(&mut part, empty.clone()));
                part.text = piece.to_string();
            }
        }
        if !part.text.is_empty() {
            out.push(part);
        }
    }
    out
}

/// Sentences, and a sentence that alone exceeds `room` cut into words, and a
/// word that alone exceeds it cut into characters.
fn pieces(text: &str, room: usize) -> Vec<String> {
    let room = room.max(1);
    let mut out = Vec::new();
    for sentence in sentences(text) {
        if estimate(&sentence) <= room {
            out.push(sentence);
            continue;
        }
        for word in sentence.split_whitespace() {
            if estimate(word) <= room {
                out.push(word.to_string());
                continue;
            }
            let per = (room as f64 * crate::tokens::BYTES_PER_TOKEN)
                .floor()
                .max(1.0) as usize;
            let chars: Vec<char> = word.chars().collect();
            for c in chars.chunks(per) {
                out.push(c.iter().collect());
            }
        }
    }
    out
}

fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        if matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|n| n.is_whitespace()) {
            out.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out.retain(|s| !s.is_empty());
    out
}

/// Pack `turns` into chunks whose rendered turns each cost at most
/// `allowance` tokens.
pub fn chunk(turns: &[Turn], allowance: usize) -> Vec<Chunk> {
    let turns = split_long(turns, allowance);
    if turns.is_empty() {
        return Vec::new();
    }
    let weight: Vec<usize> = turns.iter().map(turn_cost).collect();

    let mut chunks = Vec::new();
    let mut start = 0; // first turn this chunk owns
    let mut overlap_from = 0; // first turn it repeats
    while start < turns.len() {
        let mut cost: usize = (overlap_from..start).map(|i| turn_cost(&turns[i])).sum();
        let mut end = start;
        while end < turns.len() && (end == start || cost + weight[end] <= allowance) {
            // The first owned turn always goes in, so the packing advances.
            cost += weight[end];
            end += 1;
        }
        chunks.push(Chunk {
            turns: turns[overlap_from..end].to_vec(),
            first_new: start - overlap_from,
        });
        // Repeat the last ~10% of this chunk's turns, but never so many that
        // the next chunk could not own at least one turn of its own.
        let span = end - overlap_from;
        let mut repeat = ((span as f64 * OVERLAP).round() as usize).min(end - start);
        while repeat > 0
            && end < turns.len()
            && (end - repeat..end)
                .map(|i| turn_cost(&turns[i]))
                .sum::<usize>()
                + weight[end]
                > allowance
        {
            repeat -= 1;
        }
        overlap_from = end - repeat;
        start = end;
    }
    chunks
}

/// Group consecutive summaries for one merge call: as many as fit
/// `allowance`, and never fewer than two while two remain, so each layer
/// shrinks. A lone summary left at a layer's end passes up unchanged.
pub fn merge_groups(costs: &[usize], allowance: usize) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    while start < costs.len() {
        let mut end = start;
        let mut cost = 0;
        while end < costs.len() && (end - start < 2 || cost + costs[end] <= allowance) {
            cost += costs[end];
            end += 1;
        }
        groups.push(start..end);
        start = end;
    }
    groups
}
