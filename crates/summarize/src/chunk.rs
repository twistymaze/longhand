//! Chunking a transcript that does not fit one call (KTD16). Whole turns
//! are packed under the allowance with about 10% overlap by turns. The
//! notes document goes whole with every call (KTD6), so the caller takes
//! its size from the allowance first. A turn longer than the allowance on
//! its own is split at sentence boundaries, each part keeping the speaker
//! label and start time. A pause line goes into the same chunk as the line
//! after it.

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

/// One line of the summary input: a turn, or a pause in the note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Line {
    Turn(Turn),
    /// The user paused the note for `duration_ms`. Nothing was recorded in
    /// that time.
    Pause {
        duration_ms: u64,
    },
}

impl From<Turn> for Line {
    fn from(turn: Turn) -> Self {
        Self::Turn(turn)
    }
}

/// The share of a chunk's lines repeated at the start of the next.
pub const OVERLAP: f64 = 0.10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub lines: Vec<Line>,
    /// `lines[..first_new]` repeat the end of the previous chunk.
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

pub fn render_line(line: &Line) -> String {
    match line {
        Line::Turn(turn) => render_turn(turn),
        Line::Pause { duration_ms } => format!("--- pause {} ---", format_clock(*duration_ms)),
    }
}

/// A line's cost inside a prompt: its text and its newline.
fn line_cost(line: &str) -> usize {
    estimate(line) + 1
}

fn turn_cost(turn: &Turn) -> usize {
    line_cost(&render_turn(turn))
}

/// The cost of `line` in a chunk. A pause line also costs the pause rule,
/// which a part with a pause line carries once.
fn chunk_line_cost(line: &Line) -> usize {
    match line {
        Line::Turn(turn) => turn_cost(turn),
        Line::Pause { .. } => line_cost(&render_line(line)) + line_cost(crate::prompts::PAUSE_RULE),
    }
}

/// Split turns that cannot fit a chunk on their own. A turn after pause
/// lines goes into the same chunk as them, so it gets less room.
fn split_long(lines: &[Line], allowance: usize) -> Vec<Line> {
    let mut out = Vec::with_capacity(lines.len());
    let mut pause_cost = 0;
    for line in lines {
        let Line::Turn(turn) = line else {
            pause_cost += chunk_line_cost(line);
            out.push(line.clone());
            continue;
        };
        let allowance = allowance.saturating_sub(std::mem::take(&mut pause_cost));
        if turn_cost(turn) <= allowance {
            out.push(line.clone());
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
                out.push(std::mem::replace(&mut part, empty.clone()).into());
                part.text = piece.to_string();
            }
        }
        if !part.text.is_empty() {
            out.push(part.into());
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
            // Cost by bytes: a character can be more than one byte. A piece
            // always takes at least one character, so the loop moves on.
            let mut piece = String::new();
            for c in word.chars() {
                if !piece.is_empty()
                    && crate::tokens::estimate_bytes(piece.len() + c.len_utf8()) > room
                {
                    out.push(std::mem::take(&mut piece));
                }
                piece.push(c);
            }
            out.push(piece);
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

/// Pack `lines` into chunks whose rendered lines each cost at most
/// `allowance` tokens.
pub fn chunk(lines: &[Line], allowance: usize) -> Vec<Chunk> {
    let lines = split_long(lines, allowance);
    if lines.is_empty() {
        return Vec::new();
    }
    let weight: Vec<usize> = lines.iter().map(chunk_line_cost).collect();
    // The end of the group that starts at line `i`: pause lines go with the
    // line after them, so a chunk never ends with a pause line.
    let group_end = |i: usize| {
        let mut j = i;
        while j < lines.len() && matches!(lines[j], Line::Pause { .. }) {
            j += 1;
        }
        (j + 1).min(lines.len())
    };

    let mut chunks = Vec::new();
    let mut start = 0; // first line this chunk owns
    let mut overlap_from = 0; // first line it repeats
    while start < lines.len() {
        let mut cost: usize = weight[overlap_from..start].iter().sum();
        let mut end = start;
        while end < lines.len() {
            let next = group_end(end);
            let add: usize = weight[end..next].iter().sum();
            // The first owned group always goes in, so the packing advances.
            if end > start && cost + add > allowance {
                break;
            }
            cost += add;
            end = next;
        }
        chunks.push(Chunk {
            lines: lines[overlap_from..end].to_vec(),
            first_new: start - overlap_from,
        });
        // Repeat the last ~10% of this chunk's lines, but never so many that
        // the next chunk could not own at least one group of its own.
        let span = end - overlap_from;
        let mut repeat = ((span as f64 * OVERLAP).round() as usize).min(end - start);
        while repeat > 0
            && end < lines.len()
            && weight[end - repeat..group_end(end)].iter().sum::<usize>() > allowance
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
