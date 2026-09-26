use chrono::{DateTime, FixedOffset, TimeDelta};
use proptest::prelude::*;
use session_core::{Closed, Location, Segment, Timeline};

fn wall(h: i64) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-22T09:00:00-05:00").unwrap() + TimeDelta::hours(h)
}

fn two_segments() -> Timeline {
    Timeline::from_segments(vec![
        Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 5_400_000,
            wall_start: wall(0),
            wall_end: wall(1),
            audio_file: "audio/segment-01.opus".into(),
        },
        Segment {
            index: 1,
            offset_ms: 5_640_000,
            duration_ms: 1_800_000,
            wall_start: wall(1),
            wall_end: wall(2),
            audio_file: "audio/segment-02.opus".into(),
        },
    ])
    .unwrap()
}

#[test]
fn position_and_location_follow_the_session_timeline() {
    let timeline = two_segments();
    assert_eq!(timeline.to_position(1, 12_000).unwrap(), 5_652_000);
    assert_eq!(timeline.locate(5_500_000), Location::InGap { after: 0 });
    assert_eq!(
        timeline.locate(5_652_000),
        Location::InSegment {
            index: 1,
            within_ms: 12_000
        }
    );
    assert_eq!(timeline.gaps(), vec![(0, 240_000)]);
}

#[test]
fn gap_comes_from_the_monotonic_offsets_not_the_wall_clock() {
    // Wall clock jumped back an hour between the segments.
    let timeline = Timeline::from_segments(vec![
        Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: wall(2),
            wall_end: wall(2) + TimeDelta::minutes(1),
            audio_file: "a".into(),
        },
        Segment {
            index: 1,
            offset_ms: 180_000,
            duration_ms: 60_000,
            wall_start: wall(1),
            wall_end: wall(1) + TimeDelta::minutes(1),
            audio_file: "b".into(),
        },
    ])
    .unwrap();
    assert_eq!(timeline.gaps(), vec![(0, 120_000)]);
}

#[test]
fn a_sub_second_segment_is_discarded_on_close() {
    let mut timeline = Timeline::new();
    timeline.open(0, wall(0)).unwrap();
    assert_eq!(
        timeline.close(800, wall(0), "a".into()).unwrap(),
        Closed::Discarded
    );
    assert!(timeline.is_empty());
    // Its index is reused by the next segment.
    assert_eq!(timeline.open(5_000, wall(0)).unwrap(), 0);
}

#[test]
fn a_short_segment_that_ends_at_a_switch_is_kept() {
    // KTD3: a quick microphone change, or a Stop just after one, loses no
    // audio.
    let mut timeline = Timeline::new();
    timeline.open(0, wall(0)).unwrap();
    let closed = timeline.close_at_switch(300, wall(0), "a".into()).unwrap();
    assert!(matches!(closed, Closed::Kept(ref s) if s.duration_ms == 300));
    assert_eq!(timeline.segments().len(), 1);
}

#[test]
fn a_segment_with_no_audio_is_discarded_even_at_a_switch() {
    let mut timeline = Timeline::new();
    timeline.open(1_000, wall(0)).unwrap();
    assert_eq!(
        timeline
            .close_at_switch(1_000, wall(0), "a".into())
            .unwrap(),
        Closed::Discarded
    );
    assert!(timeline.is_empty());
}

#[test]
fn overlapping_segments_are_rejected() {
    let mut timeline = two_segments();
    assert!(timeline.open(7_000_000, wall(3)).is_err());
}

proptest! {
    /// Positions are strictly monotonic across segments and gaps, and every
    /// in-segment position locates back to where it came from.
    #[test]
    fn positions_round_trip_and_stay_ordered(
        spans in prop::collection::vec((0u64..600_000, 1_000u64..3_600_000), 1..8),
        probe in 0usize..1_000,
    ) {
        let mut timeline = Timeline::new();
        let mut cursor = 0;
        for (gap, duration) in &spans {
            cursor += gap;
            timeline.open(cursor, wall(0)).unwrap();
            cursor += duration;
            prop_assert!(matches!(timeline.close(cursor, wall(0), "x".into()).unwrap(), Closed::Kept(_)));
        }
        let mut previous_end = 0;
        for segment in timeline.segments() {
            prop_assert!(segment.offset_ms >= previous_end);
            previous_end = segment.end_ms();
            let within = (probe as u64 * 7_919) % segment.duration_ms;
            let position = timeline.to_position(segment.index, within).unwrap();
            prop_assert_eq!(timeline.locate(position), Location::InSegment { index: segment.index, within_ms: within });
        }
        for ((after, gap), next) in timeline.gaps().into_iter().zip(timeline.segments().iter().skip(1)) {
            if gap > 0 {
                prop_assert_eq!(timeline.locate(next.offset_ms - 1), Location::InGap { after });
            }
        }
    }
}
