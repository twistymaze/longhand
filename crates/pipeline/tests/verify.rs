//! Coverage measurement: voiced time against the time that carries text.

use asr::{AsrSegment, LiveWord, Span};
use pipeline::live::LiveRow;
use pipeline::transcribe::WindowResult;
use pipeline::verify::{MAX_UNCOVERED_MS, measure, text_spans};

mod common;

#[test]
fn verification_counts_speech_without_request_padding() {
    let store = common::store();
    store
        .lock()
        .unwrap()
        .create_session("coverage", common::at(0), None)
        .unwrap();
    let segment = session_core::Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: 20_000,
        wall_start: common::at(0),
        wall_end: common::at(20),
        audio_file: "unused.opus".into(),
        channels: 1,
    };
    let plan = serde_json::json!({
        "segment": 0, "duration_ms": 20000, "voiced_ms": 9756,
        "voiced": [span(286, 5394), span(5614, 9058), span(17342, 18546)],
        "speech": [span(736, 4944), span(6064, 8608), span(17792, 17888), span(18016, 18096)],
        "windows": [span(286, 9058), span(17342, 18546)]
    });
    pipeline::checkpoint::put(
        &store,
        "coverage",
        session_core::Stage::Transcribe,
        &pipeline::checkpoint::plan_key(0),
        &plan,
    )
    .unwrap();
    let mut window = WindowResult {
        segment: 0,
        window: 0,
        start_ms: 286,
        end_ms: 9058,
        text: "Recorded speech".into(),
        segments: vec![AsrSegment {
            start_ms: 1120,
            end_ms: 7772,
            text: "Recorded speech".into(),
        }],
        words: None,
        live: false,
    };
    let key = pipeline::checkpoint::window_key(0, 0);
    pipeline::checkpoint::put(
        &store,
        "coverage",
        session_core::Stage::Transcribe,
        &key,
        &window,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    pipeline::verify::run(&store, "coverage", dir.path(), &[segment.clone()]).unwrap();
    let coverage: pipeline::verify::Coverage = pipeline::checkpoint::get(
        &store,
        "coverage",
        session_core::Stage::Verify,
        &pipeline::checkpoint::segment_key(0),
    )
    .unwrap()
    .unwrap();
    assert_eq!(coverage.voiced_ms, 6928);
    assert_eq!(coverage.voiced_ms - coverage.covered_ms, 176);
    // The second utterance still counts when its text is absent.
    window.segments[0].end_ms = 4000;
    pipeline::checkpoint::put(
        &store,
        "coverage",
        session_core::Stage::Transcribe,
        &key,
        &window,
    )
    .unwrap();
    assert!(pipeline::verify::run(&store, "coverage", dir.path(), &[segment.clone()]).is_err());
    window.segments.clear();
    window.text.clear();
    pipeline::checkpoint::put(
        &store,
        "coverage",
        session_core::Stage::Transcribe,
        &key,
        &window,
    )
    .unwrap();
    assert!(pipeline::verify::run(&store, "coverage", dir.path(), &[segment]).is_err());
}

#[test]
fn legacy_non_silent_plan_rechecks_audio_instead_of_using_padding() {
    let store = common::store();
    let dir = tempfile::tempdir().unwrap();
    common::queue_session(
        &store,
        dir.path(),
        "legacy",
        20,
        None,
        &[(16000, common::speech())],
    );
    let segments = store.lock().unwrap().segments("legacy").unwrap();
    pipeline::assemble::run(
        &store,
        "legacy",
        &dir.path().join("legacy"),
        &segments,
        None,
        &pipeline::Unattended,
    )
    .unwrap();
    let legacy = serde_json::json!({
        "segment": 0, "duration_ms": segments[0].duration_ms, "voiced_ms": 1,
        "voiced": [span(0, 1)], "windows": [span(0, 1)]
    });
    pipeline::checkpoint::put(
        &store,
        "legacy",
        session_core::Stage::Transcribe,
        &pipeline::checkpoint::plan_key(0),
        &legacy,
    )
    .unwrap();
    assert!(
        pipeline::verify::run(&store, "legacy", &dir.path().join("legacy"), &segments).is_err()
    );
    let coverage: pipeline::verify::Coverage = pipeline::checkpoint::get(
        &store,
        "legacy",
        session_core::Stage::Verify,
        &pipeline::checkpoint::segment_key(0),
    )
    .unwrap()
    .unwrap();
    assert!(
        coverage.voiced_ms > 1,
        "legacy padding must be replaced by audio analysis"
    );
}

#[test]
fn legacy_zero_voiced_plan_preserves_the_adopted_transcript_skip() {
    let store = common::store();
    store
        .lock()
        .unwrap()
        .create_session("adopted", common::at(0), None)
        .unwrap();
    let segment = session_core::Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: 20_000,
        wall_start: common::at(0),
        wall_end: common::at(20),
        audio_file: "unavailable.opus".into(),
        channels: 1,
    };
    let legacy = serde_json::json!({
        "segment": 0, "duration_ms": 20000, "voiced_ms": 0,
        "voiced": [], "windows": [span(0, 20000)]
    });
    pipeline::checkpoint::put(
        &store,
        "adopted",
        session_core::Stage::Transcribe,
        &pipeline::checkpoint::plan_key(0),
        &legacy,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    pipeline::verify::run(&store, "adopted", dir.path(), &[segment]).unwrap();
}

#[test]
fn legacy_speech_without_audio_fails_instead_of_passing_as_silence() {
    let store = common::store();
    store
        .lock()
        .unwrap()
        .create_session("missing", common::at(0), None)
        .unwrap();
    let segment = session_core::Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: 20_000,
        wall_start: common::at(0),
        wall_end: common::at(20),
        audio_file: "unavailable.opus".into(),
        channels: 1,
    };
    let legacy = serde_json::json!({
        "segment": 0, "duration_ms": 20000, "voiced_ms": 1000,
        "voiced": [], "windows": [span(0, 20000)]
    });
    pipeline::checkpoint::put(
        &store,
        "missing",
        session_core::Stage::Transcribe,
        &pipeline::checkpoint::plan_key(0),
        &legacy,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    assert!(pipeline::verify::run(&store, "missing", dir.path(), &[segment]).is_err());
}

const MIN: u64 = 60_000;

fn span(start_ms: u64, end_ms: u64) -> Span {
    Span { start_ms, end_ms }
}

#[test]
fn a_segment_with_no_speech_passes() {
    let c = measure(0, &[], &[]);
    assert!(c.passes());
    assert_eq!(c.ratio(), 1.0);
}

#[test]
fn text_on_only_the_first_half_hour_of_an_hour_of_speech_fails_naming_the_rest() {
    let c = measure(0, &[span(0, 60 * MIN)], &[span(0, 30 * MIN)]);
    assert!(!c.passes());
    assert_eq!(c.uncovered, vec![span(30 * MIN + 1_000, 60 * MIN)]);
    assert_eq!(c.covered_ms, 30 * MIN + 1_000);
}

#[test]
fn one_short_miss_among_many_spans_is_within_tolerance() {
    let voiced: Vec<Span> = (0..100)
        .map(|i| span(i * 10_000, i * 10_000 + 8_000))
        .collect();
    let covered: Vec<Span> = voiced
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if i == 40 {
                span(v.start_ms, v.end_ms - 3_000)
            } else {
                *v
            }
        })
        .collect();
    let c = measure(0, &voiced, &covered);
    assert!(c.passes(), "{c:?}");
    // The last second of text's slack covers part of the miss.
    assert_eq!(c.voiced_ms - c.covered_ms, 2_000);
    assert_eq!(c.uncovered, vec![span(406_000, 408_000)]);
}

#[test]
fn windows_whose_text_covers_only_their_first_seconds_do_not_count_as_covered() {
    let windows: Vec<WindowResult> = (0..20u64)
        .map(|i| WindowResult {
            segment: 0,
            window: i as usize,
            start_ms: i * 30_000,
            end_ms: (i + 1) * 30_000,
            text: "hello".into(),
            segments: vec![AsrSegment {
                start_ms: 0,
                end_ms: 3_000,
                text: "hello".into(),
            }],
            words: None,
            live: false,
        })
        .collect();
    let refs: Vec<&WindowResult> = windows.iter().collect();
    let c = measure(0, &[span(0, 600_000)], &text_spans(&refs));
    assert!(!c.passes());
    // Three seconds of text and a second of slack each side, per 30 s.
    assert!((c.ratio() - 5.0 / 30.0).abs() < 0.01, "{}", c.ratio());
}

#[test]
fn a_ninety_second_miss_among_hours_of_coverage_fails_on_the_long_stretch_rule() {
    let hours = 3 * 60 * MIN;
    let voiced = [span(0, hours)];
    let covered = [span(0, 60 * MIN), span(60 * MIN + 90_000, hours)];
    let c = measure(0, &voiced, &covered);
    assert!(c.ratio() > 0.99);
    assert!(!c.passes());
    assert_eq!(c.longest_uncovered_ms, 88_000);
}

#[test]
fn a_missed_stretch_of_speech_with_pauses_counts_as_one() {
    // Three minutes of speech in 10 s bursts, none of it transcribed.
    let mut voiced = vec![span(0, 60 * MIN)];
    voiced.extend(
        (0..18).map(|i| span(60 * MIN + 1_000 + i * 10_000, 60 * MIN + 9_000 + i * 10_000)),
    );
    voiced.push(span(64 * MIN, 240 * MIN));
    let covered = [span(0, 60 * MIN), span(64 * MIN, 240 * MIN)];
    let c = measure(0, &voiced, &covered);
    assert!(c.ratio() > 0.98);
    assert_eq!(c.uncovered.len(), 1);
    assert_eq!(c.longest_uncovered_ms, 18 * 8_000);
    assert!(c.longest_uncovered_ms > MAX_UNCOVERED_MS);
    assert!(!c.passes());
}

#[test]
fn scattered_noise_in_a_long_pause_is_not_a_missed_stretch() {
    // A cough every 20 s through five minutes of quiet.
    let mut voiced = vec![span(0, 60 * MIN)];
    voiced.extend((0..15).map(|i| span(60 * MIN + i * 20_000, 60 * MIN + i * 20_000 + 500)));
    voiced.push(span(66 * MIN, 120 * MIN));
    let covered = [span(0, 60 * MIN), span(66 * MIN, 120 * MIN)];
    let c = measure(0, &voiced, &covered);
    // The first cough sits inside the slack of the speech before it.
    assert_eq!(c.longest_uncovered_ms, 7_000);
    assert!(c.passes());
}

#[test]
fn text_past_a_padded_window_is_clamped_and_blank_text_covers_nothing() {
    let window = WindowResult {
        segment: 0,
        window: 0,
        start_ms: 10_000,
        end_ms: 10_600,
        text: "hi".into(),
        segments: vec![
            AsrSegment {
                start_ms: 0,
                end_ms: 1_000,
                text: "hi".into(),
            },
            AsrSegment {
                start_ms: 0,
                end_ms: 600,
                text: "  ".into(),
            },
        ],
        words: None,
        live: false,
    };
    assert_eq!(text_spans(&[&window]), vec![span(10_000, 10_600)]);
}

#[test]
fn text_without_timestamps_covers_nothing() {
    let window = WindowResult {
        segment: 0,
        window: 0,
        start_ms: 0,
        end_ms: 30_000,
        text: "hi".into(),
        segments: vec![],
        words: None,
        live: false,
    };
    assert_eq!(text_spans(&[&window]), vec![]);
    let c = measure(0, &[span(0, 30_000)], &text_spans(&[&window]));
    assert!(!c.passes(), "{c:?}");
}

#[test]
fn live_words_put_text_on_their_own_spans() {
    let word = |start_ms, end_ms, text: &str| LiveWord {
        start_ms,
        end_ms,
        text: text.into(),
    };
    let part = WindowResult::from_live(LiveRow {
        segment: 0,
        part: 0,
        start_ms: 0,
        end_ms: 30_000,
        words: vec![
            word(1_000, 1_400, "one"),
            word(1_400, 1_800, "two"),
            word(5_000, 5_000, "three"),
            word(9_000, 9_500, " "),
            word(29_800, 30_400, "last"),
        ],
    });
    // Touching words form one span; a word of no length covers 1 ms; a blank
    // word covers nothing; a word past the part is clamped to the part.
    assert_eq!(
        text_spans(&[&part]),
        vec![span(1_000, 1_800), span(5_000, 5_001), span(29_800, 30_000)]
    );
    // The live text covers the speech around it within the slack only.
    let c = measure(0, &[span(0, 30_000)], &text_spans(&[&part]));
    assert_eq!(c.uncovered, vec![span(2_800, 4_000), span(6_001, 28_800)]);
}
