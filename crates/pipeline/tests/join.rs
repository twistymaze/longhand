//! The join: ASR text onto speaker turns, in session time.

use asr::{AsrSegment, LiveWord};
use chrono::{DateTime, FixedOffset};
use diarize::Turn;
use pipeline::diarize::Speakers;
use pipeline::join::{SILENCE_MARK_MS, build, found_speakers};
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
        channels: 1,
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
        words: None,
        live: false,
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
    // A shorter pause is not marked. It still starts a new paragraph (KTD10).
    let windows = [
        window(0, 0, &[(0, 1_000, "hello")]),
        window(0, SILENCE_MARK_MS, &[(0, 1_000, "again")]),
    ];
    let t = build(&segments, &windows, &speakers);
    assert!(
        !t.lines
            .iter()
            .any(|l| matches!(l, TranscriptLine::Silence { .. }))
    );
    assert_eq!(
        turns_of(&t.lines),
        vec![
            (0, Some(1), "hello".into()),
            (SILENCE_MARK_MS, Some(1), "again".into())
        ]
    );
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
        words: None,
        live: false,
    }];
    let speakers = Speakers::Turns(vec![turn(0, 0, 3_500, 1), turn(0, 3_500, 10_000, 2)]);
    let t = build(&segments, &windows, &speakers);
    assert_eq!(turns_of(&t.lines), vec![(2_000, Some(1), "whole".into())]);
}

/// A sentence of `len` characters that ends with a full stop.
fn sentence(n: usize, len: usize) -> String {
    let head = format!("Sentence {n} says");
    let mut s = head.clone();
    while s.len() < len - 1 {
        s.push_str(" more");
    }
    s.truncate(len - 1);
    s.push('.');
    s
}

#[test]
fn a_long_turn_splits_into_paragraphs_at_sentence_starts() {
    let sentences: Vec<String> = (0..6).map(|n| sentence(n, 200)).collect();
    // Six sentences of 200 characters, 5 s each, with no pause between them.
    let pieces: Vec<(u64, u64, &str)> = sentences
        .iter()
        .enumerate()
        .map(|(i, s)| (i as u64 * 5_000, (i as u64 + 1) * 5_000, s.as_str()))
        .collect();
    let segments = [segment(0, 10_000, 60_000)];
    let windows = [window(0, 0, &pieces)];
    let speakers = Speakers::Turns(vec![turn(0, 0, 60_000, 1)]);
    let turns = turns_of(&build(&segments, &windows, &speakers).lines);
    assert!(turns.len() >= 3, "{turns:?}");
    for (position_ms, speaker, text) in &turns {
        assert_eq!(*speaker, Some(1));
        let i = sentences
            .iter()
            .position(|s| text.starts_with(s.as_str()))
            .expect("each turn starts at a sentence start");
        assert_eq!(*position_ms, 10_000 + i as u64 * 5_000);
        assert!(text.ends_with('.'), "{text}");
    }
    let joined: Vec<String> = turns.into_iter().map(|t| t.2).collect();
    assert_eq!(joined.join(" "), sentences.join(" "));
}

#[test]
fn a_long_turn_does_not_split_in_the_middle_of_a_sentence() {
    let long = sentence(0, 450);
    let long = long.trim_end_matches('.');
    let windows = [window(
        0,
        0,
        &[(0, 5_000, long), (5_000, 6_000, "and the end.")],
    )];
    let speakers = Speakers::Turns(vec![turn(0, 0, 10_000, 1)]);
    let turns = turns_of(&build(&[segment(0, 0, 10_000)], &windows, &speakers).lines);
    assert_eq!(turns.len(), 1, "{turns:?}");
}

#[test]
fn a_pause_of_three_seconds_starts_a_new_turn() {
    let windows = [window(0, 0, &[(0, 1_000, "one"), (4_000, 5_000, "two")])];
    let speakers = Speakers::Turns(vec![turn(0, 0, 10_000, 1)]);
    let turns = turns_of(&build(&[segment(0, 0, 10_000)], &windows, &speakers).lines);
    assert_eq!(
        turns,
        vec![(0, Some(1), "one".into()), (4_000, Some(1), "two".into())]
    );
    // Without speakers the pause splits the text too.
    let turns = turns_of(&build(&[segment(0, 0, 10_000)], &windows, &Speakers::Unavailable).lines);
    assert_eq!(turns.len(), 2);
}

#[test]
fn a_short_same_speaker_run_stays_one_turn() {
    let windows = [window(
        0,
        0,
        &[
            (0, 1_000, "One."),
            (2_500, 3_000, "Two."),
            (4_000, 5_000, "Three."),
        ],
    )];
    let speakers = Speakers::Turns(vec![turn(0, 0, 10_000, 1)]);
    let turns = turns_of(&build(&[segment(0, 0, 10_000)], &windows, &speakers).lines);
    assert_eq!(turns, vec![(0, Some(1), "One. Two. Three.".into())]);
}

#[test]
fn the_found_speaker_count_is_the_number_of_distinct_speakers() {
    let segments = [segment(0, 0, 20_000)];
    let windows = [window(
        0,
        0,
        &[
            (0, 1_000, "a"),
            (5_000, 6_000, "b"),
            (10_000, 11_000, "c"),
            (15_000, 16_000, "d"),
        ],
    )];
    let speakers = Speakers::Turns(vec![
        turn(0, 0, 5_000, 7),
        turn(0, 5_000, 10_000, 4),
        turn(0, 10_000, 15_000, 9),
        turn(0, 15_000, 20_000, 7),
    ]);
    let t = build(&segments, &windows, &speakers);
    assert_eq!(found_speakers(&t), Some(3));
    // No speaker marks (R20a): no count.
    let t = build(&segments, &windows, &Speakers::Unavailable);
    assert_eq!(found_speakers(&t), None);
    // Speakers found, but nothing said: no marks, so no count.
    let t = build(&segments, &[], &speakers);
    assert_eq!(found_speakers(&t), None);
}

fn stored(windows: &[WindowResult], speakers: &Speakers) -> pipeline::SharedStore {
    let mut store = session_core::Store::open_in_memory().unwrap();
    store.create_session("s", wall(), None).unwrap();
    let store = std::sync::Arc::new(std::sync::Mutex::new(store));
    for w in windows {
        pipeline::checkpoint::put(
            &store,
            "s",
            session_core::Stage::Transcribe,
            &pipeline::checkpoint::window_key(w.segment, w.window),
            w,
        )
        .unwrap();
    }
    match speakers {
        Speakers::Turns(turns) => pipeline::checkpoint::put(
            &store,
            "s",
            session_core::Stage::Diarize,
            pipeline::diarize::TURNS,
            turns,
        )
        .unwrap(),
        Speakers::Unavailable => {
            pipeline::diarize::record_failure(&store, "s", 1, "no speakers").unwrap()
        }
    }
    store
}

#[test]
fn the_join_stage_stores_the_found_speaker_count() {
    let segments = [segment(0, 0, 20_000)];
    let windows = [window(
        0,
        0,
        &[(0, 1_000, "a"), (5_000, 6_000, "b"), (10_000, 11_000, "c")],
    )];
    let speakers = Speakers::Turns(vec![
        turn(0, 0, 5_000, 2),
        turn(0, 5_000, 10_000, 1),
        turn(0, 10_000, 15_000, 3),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let store = stored(&windows, &speakers);
    pipeline::join::run(&store, "s", dir.path(), &segments).unwrap();
    assert_eq!(store.lock().unwrap().found_speakers("s").unwrap(), Some(3));

    // A run without speaker marks records none, and replaces an earlier count.
    let store = stored(&windows, &Speakers::Unavailable);
    store
        .lock()
        .unwrap()
        .set_found_speakers("s", Some(2))
        .unwrap();
    pipeline::join::run(&store, "s", dir.path(), &segments).unwrap();
    assert_eq!(store.lock().unwrap().found_speakers("s").unwrap(), None);
}

/// A live part whose words are (start, end, text) within the segment.
fn live_part(segment: u32, part: u32, words: &[(u64, u64, &str)]) -> WindowResult {
    let words: Vec<LiveWord> = words
        .iter()
        .map(|&(start_ms, end_ms, text)| LiveWord {
            start_ms,
            end_ms,
            text: text.into(),
        })
        .collect();
    WindowResult::from_live(pipeline::live::LiveRow {
        segment,
        part,
        start_ms: words.first().map_or(0, |w| w.start_ms),
        end_ms: words.last().map_or(0, |w| w.end_ms),
        words,
    })
}

#[test]
fn a_live_phrase_that_crosses_a_speaker_change_becomes_two_lines() {
    let segments = [segment(0, 10_000, 20_000)];
    let windows = [live_part(
        0,
        0,
        &[
            (0, 400, "Are"),
            (500, 900, "you"),
            (1_000, 1_400, "sure?"),
            // Mostly inside speaker 2's turn.
            (1_700, 2_300, "Yes,"),
            (2_400, 2_800, "quite."),
        ],
    )];
    let speakers = Speakers::Turns(vec![turn(0, 0, 1_800, 7), turn(0, 1_800, 5_000, 4)]);
    let t = build(&segments, &windows, &speakers);
    assert_eq!(
        turns_of(&t.lines),
        vec![
            (10_000, Some(1), "Are you sure?".into()),
            (11_700, Some(2), "Yes, quite.".into()),
        ]
    );
}

#[test]
fn live_words_and_gap_fill_words_join_in_time_order_and_unspaced_words_join_without_spaces() {
    let segments = [segment(0, 0, 20_000)];
    let mut gap = live_part(0, 0, &[(3_000, 3_300, "日"), (3_300, 3_600, "は")]);
    gap.live = false;
    gap.window = 4;
    let windows = [
        live_part(0, 0, &[(1_000, 1_300, "今")]),
        gap,
        live_part(0, 1, &[(4_000, 4_300, "晴れ")]),
    ];
    let t = build(&segments, &windows, &Speakers::Unavailable);
    assert_eq!(turns_of(&t.lines), vec![(1_000, None, "今日は晴れ".into())]);
}

#[test]
fn the_join_stage_reads_the_saved_live_rows() {
    let segments = [segment(0, 0, 20_000)];
    let dir = tempfile::tempdir().unwrap();
    let store = stored(&[], &Speakers::Unavailable);
    let part = asr::LivePart {
        start_ms: 0,
        end_ms: 20_000,
        words: vec![LiveWord {
            start_ms: 2_000,
            end_ms: 2_400,
            text: "hello".into(),
        }],
    };
    pipeline::live::save_part(&store, "s", 0, 0, &part).unwrap();
    let t = pipeline::join::run(&store, "s", dir.path(), &segments).unwrap();
    assert_eq!(turns_of(&t.lines), vec![(2_000, None, "hello".into())]);
}

/// The paragraph rule of the final transcript gives the paragraphs in the
/// shared fixture. `src/session/liveRows.test.ts` holds the live copy of the
/// rule to the same fixture (KTD7).
#[test]
fn the_paragraph_rule_gives_the_paragraphs_of_the_shared_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/paragraph-rule.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let words: Vec<(u64, u64, &str)> = case["words"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| {
                (
                    w[0].as_u64().unwrap(),
                    w[1].as_u64().unwrap(),
                    w[2].as_str().unwrap(),
                )
            })
            .collect();
        let expected: Vec<(u64, Option<u32>, String)> = case["paragraphs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let first = p[0].as_u64().unwrap() as usize;
                (words[first].0, None, p[1].as_str().unwrap().to_string())
            })
            .collect();
        let end_ms = words.last().unwrap().1;
        let t = build(
            &[segment(0, 0, end_ms)],
            &[live_part(0, 0, &words)],
            &Speakers::Unavailable,
        );
        assert_eq!(t.lines.len(), expected.len(), "{name}");
        assert_eq!(turns_of(&t.lines), expected, "{name}");
    }
}
