use chrono::{DateTime, FixedOffset};
use session_core::{Anchor, DEFAULT_LOOKBACK_MS, Segment, Timeline, resolve};

fn wall() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-22T09:00:00-05:00").unwrap()
}

fn two_segments() -> Timeline {
    let segment = |index, offset_ms, duration_ms| Segment {
        index,
        offset_ms,
        duration_ms,
        wall_start: wall(),
        wall_end: wall(),
        audio_file: String::new(),
    };
    Timeline::from_segments(vec![
        segment(0, 0, 5_400_000),
        segment(1, 5_640_000, 1_800_000),
    ])
    .unwrap()
}

#[test]
fn keystroke_in_a_gap_anchors_to_the_preceding_end_and_says_so() {
    assert_eq!(
        resolve(&two_segments(), Some(5_500_000), DEFAULT_LOOKBACK_MS),
        Anchor::At {
            position_ms: 5_400_000,
            gap_anchored: true
        }
    );
}

#[test]
fn lookback_into_a_gap_stops_at_the_preceding_end() {
    assert_eq!(
        resolve(&two_segments(), Some(5_642_000), DEFAULT_LOOKBACK_MS),
        Anchor::At {
            position_ms: 5_400_000,
            gap_anchored: false
        }
    );
}

#[test]
fn keystroke_inside_a_segment_looks_back() {
    assert_eq!(
        resolve(&two_segments(), Some(100_000), DEFAULT_LOOKBACK_MS),
        Anchor::At {
            position_ms: 95_000,
            gap_anchored: false
        }
    );
}

#[test]
fn keystroke_in_the_first_seconds_clamps_to_the_start() {
    assert_eq!(
        resolve(&two_segments(), Some(2_000), DEFAULT_LOOKBACK_MS),
        Anchor::At {
            position_ms: 0,
            gap_anchored: false
        }
    );
}

#[test]
fn keystroke_with_no_session_position_is_unanchored() {
    assert_eq!(
        resolve(&two_segments(), None, DEFAULT_LOOKBACK_MS),
        Anchor::Unanchored
    );
    assert_eq!(
        resolve(&Timeline::new(), Some(1_000), DEFAULT_LOOKBACK_MS),
        Anchor::Unanchored
    );
}

#[test]
fn keystroke_during_an_interruption_is_gap_anchored() {
    // Last segment closed, nothing open: the interruption is a trailing gap.
    assert_eq!(
        resolve(&two_segments(), Some(8_000_000), DEFAULT_LOOKBACK_MS),
        Anchor::At {
            position_ms: 7_440_000,
            gap_anchored: true
        }
    );
}

#[test]
fn keystroke_in_the_open_segment_anchors_inside_it() {
    let mut timeline = two_segments();
    timeline.open(8_000_000, wall()).unwrap();
    assert_eq!(
        resolve(&timeline, Some(8_020_000), DEFAULT_LOOKBACK_MS),
        Anchor::At {
            position_ms: 8_015_000,
            gap_anchored: false
        }
    );
}
