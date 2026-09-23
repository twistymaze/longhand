//! The join stage (KTD7, R16, R20a): each ASR segment goes to the speaker
//! turn it overlaps most, consecutive same-speaker segments merge into one
//! transcript turn, and times move from within-segment to session positions.
//! The result is `transcript.md` in the session's working directory.

use std::path::Path;

use diarize::Turn;
use session_core::folder::{Transcript, TranscriptLine};
use session_core::{Segment, Stage};

use crate::diarize::{Speakers, speakers};
use crate::transcribe::{WindowResult, results};
use crate::{SharedStore, StageError, checkpoint};

/// A within-segment stretch with nothing said at least this long is marked
/// in the transcript (provisional).
pub const SILENCE_MARK_MS: u64 = 60_000;
pub const TRANSCRIPT_FILE: &str = "transcript.md";

pub fn run(
    store: &SharedStore,
    session_id: &str,
    session_dir: &Path,
    segments: &[Segment],
) -> Result<Transcript, StageError> {
    let transcript = build(
        segments,
        &results(store, session_id)?,
        &speakers(store, session_id)?,
    );
    let path = session_dir.join(TRANSCRIPT_FILE);
    let temp = session_dir.join(format!("{TRANSCRIPT_FILE}.tmp"));
    std::fs::write(&temp, transcript.render())?;
    crate::sync_file(&temp)?;
    std::fs::rename(&temp, &path)?;
    checkpoint::mark_complete(store, session_id, Stage::Join, segments.len())?;
    Ok(transcript)
}

/// One ASR segment placed within its recorded segment.
struct Piece<'a> {
    start_ms: u64,
    end_ms: u64,
    text: &'a str,
}

pub fn build(segments: &[Segment], windows: &[WindowResult], speakers: &Speakers) -> Transcript {
    let turns: &[Turn] = match speakers {
        Speakers::Turns(turns) => turns,
        Speakers::Unavailable => &[],
    };
    let has_speakers = matches!(speakers, Speakers::Turns(_));

    let mut ordered: Vec<&Segment> = segments.iter().collect();
    ordered.sort_by_key(|s| s.offset_ms);

    let mut lines = Vec::new();
    let mut previous_end: Option<u64> = None;
    for segment in ordered {
        if let Some(end) = previous_end {
            lines.push(TranscriptLine::Gap {
                duration_ms: segment.offset_ms.saturating_sub(end),
            });
        }
        previous_end = Some(segment.end_ms());

        let mut pieces: Vec<Piece> = Vec::new();
        for window in windows.iter().filter(|w| w.segment == segment.index) {
            if window.segments.is_empty() && !window.text.is_empty() {
                pieces.push(Piece {
                    start_ms: window.start_ms,
                    end_ms: window.end_ms,
                    text: &window.text,
                });
            }
            for s in &window.segments {
                pieces.push(Piece {
                    start_ms: window.start_ms + s.start_ms,
                    end_ms: window.start_ms + s.end_ms,
                    text: &s.text,
                });
            }
        }
        pieces.sort_by_key(|p| p.start_ms);
        let own: Vec<&Turn> = turns
            .iter()
            .filter(|t| t.segment == segment.index)
            .collect();

        // (speaker, position, text) of the turn being built.
        let mut open: Option<(Option<u32>, u64, String)> = None;
        let mut last_end: Option<u64> = None;
        for piece in pieces.iter().filter(|p| !p.text.trim().is_empty()) {
            if let Some(end) = last_end
                && piece.start_ms.saturating_sub(end) >= SILENCE_MARK_MS
            {
                flush(&mut open, &mut lines);
                lines.push(TranscriptLine::Silence {
                    duration_ms: piece.start_ms - end,
                });
            }
            last_end = Some(last_end.map_or(piece.end_ms, |e| e.max(piece.end_ms)));
            let speaker = if has_speakers {
                speaker_for(piece, &own)
            } else {
                None
            };
            match &mut open {
                Some((current, _, text)) if *current == speaker => {
                    text.push(' ');
                    text.push_str(piece.text.trim());
                }
                _ => {
                    flush(&mut open, &mut lines);
                    open = Some((
                        speaker,
                        segment.offset_ms + piece.start_ms,
                        piece.text.trim().to_string(),
                    ));
                }
            }
        }
        flush(&mut open, &mut lines);
    }

    renumber(&mut lines);
    Transcript {
        has_speakers,
        lines,
    }
}

fn flush(open: &mut Option<(Option<u32>, u64, String)>, lines: &mut Vec<TranscriptLine>) {
    if let Some((speaker, position_ms, text)) = open.take() {
        lines.push(TranscriptLine::Turn {
            position_ms,
            speaker,
            text,
        });
    }
}

/// The turn overlapping `piece` most; failing any overlap, the nearest.
fn speaker_for(piece: &Piece, turns: &[&Turn]) -> Option<u32> {
    let mut best: Option<(u64, u32)> = None;
    let mut nearest: Option<(u64, u32)> = None;
    for turn in turns {
        let overlap = turn
            .end_ms
            .min(piece.end_ms)
            .saturating_sub(turn.start_ms.max(piece.start_ms));
        if overlap > 0 {
            if best.is_none_or(|(o, _)| overlap > o) {
                best = Some((overlap, turn.speaker));
            }
        } else {
            let distance = if turn.end_ms <= piece.start_ms {
                piece.start_ms - turn.end_ms
            } else {
                turn.start_ms.saturating_sub(piece.end_ms)
            };
            if nearest.is_none_or(|(d, _)| distance < d) {
                nearest = Some((distance, turn.speaker));
            }
        }
    }
    best.or(nearest).map(|(_, speaker)| speaker)
}

/// Number speakers 1..N by first appearance in the transcript, so a speaker
/// the stack found but who said nothing transcribed leaves no hole.
fn renumber(lines: &mut [TranscriptLine]) {
    let mut order: Vec<u32> = Vec::new();
    for line in lines.iter() {
        if let TranscriptLine::Turn {
            speaker: Some(s), ..
        } = line
            && !order.contains(s)
        {
            order.push(*s);
        }
    }
    for line in lines.iter_mut() {
        if let TranscriptLine::Turn {
            speaker: Some(s), ..
        } = line
        {
            *s = order.iter().position(|o| o == s).expect("seen") as u32 + 1;
        }
    }
}
