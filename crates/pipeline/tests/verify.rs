//! Coverage measurement: voiced time against the time that carries text.

use asr::{AsrSegment, Span};
use pipeline::transcribe::WindowResult;
use pipeline::verify::{MAX_UNCOVERED_MS, measure, text_spans};

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
    };
    assert_eq!(text_spans(&[&window]), vec![span(10_000, 10_600)]);
}
