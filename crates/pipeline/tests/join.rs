//! The join: ASR text onto speaker turns, in session time.

use asr::AsrSegment;
use chrono::{DateTime, FixedOffset};
use diarize::Turn;
use pipeline::diarize::Speakers;
use pipeline::join::{SILENCE_MARK_MS, build};
use pipeline::transcribe::WindowResult;
use session_core::Segment;
use session_core::folder::TranscriptLine;

fn wall() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-22T10:00:00+00:00").unwrap()
}

fn segment(index: u32, offset_ms: u64, duration_ms: u64) -> Segment {
    Segment {
        index,
        offset_ms,
        duration_ms,
        wall_start: wall(),
        wall_end: wall(),
        audio_file: format!("audio/segment-{:02}.opus", index + 1),
    }
}

/// A window whose ASR segments are (start, end, text) within the window.
fn window(segment: u32, start_ms: u64, pieces: &[(u64, u64, &str)]) -> WindowResult {
    let end_ms = start_ms + pieces.last().map_or(0, |p| p.1);
    WindowResult {
        segment,
        window: 0,
        start_ms,
        end_ms,
        text: pieces.iter().map(|p| p.2).collect::<Vec<_>>().join(" "),
        segments: pieces
            .iter()
            .map(|&(s, e, t)| AsrSegment {
                start_ms: s,
                end_ms: e,
                text: t.into(),
            })
            .collect(),
    }
}

fn turn(segment: u32, start_ms: u64, end_ms: u64, speaker: u32) -> Turn {
    Turn {
        segment,
        start_ms,
        end_ms,
        speaker,
    }
}

fn turns_of(lines: &[TranscriptLine]) -> Vec<(u64, Option<u32>, String)> {
    lines
        .iter()
        .filter_map(|l| match l {
            TranscriptLine::Turn {
                position_ms,
                speaker,
                text,
            } => Some((*position_ms, *speaker, text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn each_piece_goes_to_the_turn_it_overlaps_most_and_same_speaker_pieces_merge() {
    let segments = [segment(0, 0, 20_000)];
    let windows = [window(
        0,
        0,
        &[
            (0, 2_000, "one"),
            (2_000, 4_000, "two"),
            // Mostly inside speaker 2's turn.
            (4_500, 7_000, "three"),
            (7_000, 9_000, "four"),
        ],
    )];
    let speakers = Speakers::Turns(vec![turn(0, 0, 5_000, 1), turn(0, 5_000, 10_000, 2)]);
    let t = build(&segments, &windows, &speakers);
    assert!(t.has_speakers);
    assert_eq!(
        turns_of(&t.lines),
        vec![
            (0, Some(1), "one two".into()),
            (4_500, Some(2), "three four".into()),
        ]
    );
}

#[test]
fn a_piece_outside_every_turn_goes_to_the_nearest() {
    let segments = [segment(0, 0, 20_000)];
    let windows = [window(0, 0, &[(0, 1_000, "a"), (12_000, 13_000, "b")])];
    let speakers = Speakers::Turns(vec![turn(0, 0, 2_000, 1), turn(0, 9_000, 11_000, 2)]);
    let t = build(&segments, &windows, &speakers);
    assert_eq!(
        turns_of(&t.lines),
        vec![(0, Some(1), "a".into()), (12_000, Some(2), "b".into())]
    );
}

#[test]
fn times_move_to_session_positions_and_segments_are_separated_by_a_gap() {
    let segments = [segment(0, 0, 10_000), segment(1, 70_000, 10_000)];
    let windows = [
        window(0, 1_000, &[(0, 2_000, "before")]),
        window(1, 500, &[(1_000, 2_000, "after")]),
    ];
    let speakers = Speakers::Turns(vec![turn(0, 0, 10_000, 1), turn(1, 0, 10_000, 1)]);
    let t = build(&segments, &windows, &speakers);
    assert_eq!(
        t.lines,
        vec![
            TranscriptLine::Turn {
                position_ms: 1_000,
                speaker: Some(1),
                text: "before".into()
            },
            TranscriptLine::Gap {
                duration_ms: 60_000
            },
            TranscriptLine::Turn {
                position_ms: 71_500,
                speaker: Some(1),
                text: "after".into()
            },
        ]
    );
}

#[test]
fn a_long_silence_inside_a_segment_is_marked_and_splits_the_turn() {
    let segments = [segment(0, 0, 2_000 + SILENCE_MARK_MS)];
    let windows = [
        window(0, 0, &[(0, 1_000, "hello")]),
        window(0, 1_000 + SILENCE_MARK_MS, &[(0, 1_000, "again")]),
    ];
    let speakers = Speakers::Turns(vec![turn(0, 0, 200_000, 1)]);
    let t = build(&segments, &windows, &speakers);
    assert_eq!(t.lines.len(), 3);
    assert_eq!(
        t.lines[1],
        TranscriptLine::Silence {
            duration_ms: SILENCE_MARK_MS
        }
    );
    // A shorter pause is not marked.
    let windows = [
        window(0, 0, &[(0, 1_000, "hello")]),
        window(0, SILENCE_MARK_MS, &[(0, 1_000, "again")]),
    ];
    let t = build(&segments, &windows, &speakers);
    assert_eq!(turns_of(&t.lines), vec![(0, Some(1), "hello again".into())]);
}

#[test]
fn silence_at_a_segments_start_or_end_is_marked_too() {
    // Playback stopped a minute and a half in; the recording ran on (U25).
    let segments = [segment(0, 0, 284_000)];
    let windows = [window(0, 70_000, &[(0, 20_000, "the last line")])];
    let speakers = Speakers::Turns(vec![turn(0, 70_000, 90_000, 1)]);
    let t = build(&segments, &windows, &speakers);
    assert_eq!(
        t.lines,
        vec![
            TranscriptLine::Silence {
                duration_ms: 70_000
            },
            TranscriptLine::Turn {
                position_ms: 70_000,
                speaker: Some(1),
                text: "the last line".into()
            },
            TranscriptLine::Silence {
                duration_ms: 194_000
            },
        ]
    );
    // A segment with nothing said is one silence.
    let t = build(&[segment(0, 0, 90_000)], &[], &Speakers::Turns(vec![]));
    assert_eq!(
        t.lines,
        vec![TranscriptLine::Silence {
            duration_ms: 90_000
        }]
    );
}

#[test]
fn speakers_are_numbered_by_first_appearance_in_the_transcript() {
    let segments = [segment(0, 0, 20_000)];
    // Speaker 1 of the diarization said nothing transcribed.
    let windows = [window(0, 0, &[(6_000, 7_000, "x"), (11_000, 12_000, "y")])];
    let speakers = Speakers::Turns(vec![
        turn(0, 0, 5_000, 1),
        turn(0, 5_000, 10_000, 3),
        turn(0, 10_000, 15_000, 2),
    ]);
    let t = build(&segments, &windows, &speakers);
    assert_eq!(
        turns_of(&t.lines),
        vec![(6_000, Some(1), "x".into()), (11_000, Some(2), "y".into())]
    );
}

#[test]
fn without_speakers_the_text_still_arrives_unmarked() {
    let segments = [segment(0, 0, 10_000)];
    let windows = [window(
        0,
        0,
        &[(0, 1_000, "a"), (1_000, 2_000, " "), (2_000, 3_000, "b")],
    )];
    let t = build(&segments, &windows, &Speakers::Unavailable);
    assert!(!t.has_speakers);
    assert_eq!(turns_of(&t.lines), vec![(0, None, "a b".into())]);
    assert!(t.render().contains("no speaker marks"));
}

#[test]
fn a_window_without_timestamped_segments_uses_its_own_span() {
    let segments = [segment(0, 0, 10_000)];
    let windows = [WindowResult {
        segment: 0,
        window: 0,
        start_ms: 2_000,
        end_ms: 4_000,
        text: "whole".into(),
        segments: vec![],
    }];
    let speakers = Speakers::Turns(vec![turn(0, 0, 3_500, 1), turn(0, 3_500, 10_000, 2)]);
    let t = build(&segments, &windows, &speakers);
    assert_eq!(turns_of(&t.lines), vec![(2_000, Some(1), "whole".into())]);
}
