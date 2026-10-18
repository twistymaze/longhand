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
            channels: 1,
            ends_in_pause: false,
        },
        Segment {
            index: 1,
            offset_ms: 5_640_000,
            duration_ms: 1_800_000,
            wall_start: wall(1),
            wall_end: wall(2),
            audio_file: "audio/segment-02.opus".into(),
            channels: 1,
            ends_in_pause: false,
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
            channels: 1,
            ends_in_pause: false,
        },
        Segment {
            index: 1,
            offset_ms: 180_000,
            duration_ms: 60_000,
            wall_start: wall(1),
            wall_end: wall(1) + TimeDelta::minutes(1),
            audio_file: "b".into(),
            channels: 1,
            ends_in_pause: false,
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
fn a_retired_index_is_not_given_to_a_later_segment() {
    // C9: the close of segment 0 failed, and its chunks have no row.
    let mut timeline = Timeline::new();
    timeline.open(0, wall(0)).unwrap();
    timeline.retire(0);
    assert!(timeline.open_segment().is_none());
    assert_eq!(timeline.open(5_000, wall(0)).unwrap(), 1);
    // A later segment under a second gives back only its own index.
    timeline.close(5_500, wall(0), "b".into()).unwrap();
    assert_eq!(timeline.open(9_000, wall(0)).unwrap(), 1);
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
fn a_stereo_segment_keeps_its_channel_count_when_it_closes() {
    // KTD6: each segment has one channel count.
    let mut timeline = Timeline::new();
    timeline.open(0, wall(0)).unwrap();
    assert_eq!(
        timeline.open_segment().unwrap().channels,
        1,
        "mono by default"
    );
    timeline.set_open_channels(2).unwrap();
    let closed = timeline.close_at_switch(300, wall(0), "a".into()).unwrap();
    assert!(matches!(closed, Closed::Kept(ref s) if s.channels == 2));
    timeline.open(1_000, wall(0)).unwrap();
    let closed = timeline.close(5_000, wall(0), "b".into()).unwrap();
    assert!(matches!(closed, Closed::Kept(ref s) if s.channels == 1));
    assert!(timeline.set_open_channels(2).is_err(), "no segment is open");
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

/// A timeline with one closed segment for each `(offset_ms, duration_ms)` pair.
fn timeline_of(spans: &[(u64, u64)]) -> Timeline {
    let mut timeline = Timeline::new();
    for (offset, duration) in spans {
        timeline.open(*offset, wall(0)).unwrap();
        let closed = timeline
            .close(offset + duration, wall(0), "x".into())
            .unwrap();
        assert!(matches!(closed, Closed::Kept(_)));
    }
    timeline
}

#[test]
fn with_no_gap_the_shared_instant_locates_in_the_earlier_segment() {
    let timeline = timeline_of(&[(0, 5_000), (5_000, 5_000)]);
    assert_eq!(
        timeline.locate(5_000),
        Location::InSegment {
            index: 0,
            within_ms: 5_000
        }
    );
    assert_eq!(
        timeline.locate(5_001),
        Location::InSegment {
            index: 1,
            within_ms: 1
        }
    );
}

#[test]
fn with_a_one_ms_gap_no_position_is_in_the_gap() {
    let timeline = timeline_of(&[(0, 5_000), (5_001, 5_000)]);
    assert_eq!(timeline.gaps(), vec![(0, 1)]);
    assert_eq!(
        timeline.locate(5_000),
        Location::InSegment {
            index: 0,
            within_ms: 5_000
        }
    );
    assert_eq!(
        timeline.locate(5_001),
        Location::InSegment {
            index: 1,
            within_ms: 0
        }
    );
    for position in 4_990..5_010 {
        assert!(
            matches!(timeline.locate(position), Location::InSegment { .. }),
            "position {position} should be in a segment"
        );
    }
}

proptest! {
    /// Positions are strictly monotonic across segments and gaps, and every
    /// in-segment position locates back to where it came from. The end instant
    /// of a segment belongs to it, also when the next segment starts there.
    #[test]
    fn positions_round_trip_and_stay_ordered(
        spans in prop::collection::vec(
            (prop_oneof![0u64..3, 0u64..600_000], 1_000u64..3_600_000),
            1..8,
        ),
        probe in prop_oneof![Just(0usize), 0usize..1_000],
    ) {
        let mut timeline = Timeline::new();
        let mut cursor = 0;
        for (gap, duration) in &spans {
            cursor += gap;
            timeline.open(cursor, wall(0)).unwrap();
            cursor += duration;
            prop_assert!(matches!(timeline.close(cursor, wall(0), "x".into()).unwrap(), Closed::Kept(_)));
        }
        let mut previous: Option<&Segment> = None;
        for segment in timeline.segments() {
            prop_assert!(segment.offset_ms >= previous.map_or(0, Segment::end_ms));
            let within = (probe as u64 * 7_919) % segment.duration_ms;
            let position = timeline.to_position(segment.index, within).unwrap();
            let expected = match previous {
                // The start of a segment with no gap before it is the end
                // instant of the previous segment.
                Some(p) if within == 0 && p.end_ms() == segment.offset_ms => Location::InSegment {
                    index: p.index,
                    within_ms: p.duration_ms,
                },
                _ => Location::InSegment { index: segment.index, within_ms: within },
            };
            prop_assert_eq!(timeline.locate(position), expected);
            previous = Some(segment);
        }
        let segments = timeline.segments();
        for ((after, gap), pair) in timeline.gaps().into_iter().zip(segments.windows(2)) {
            let (earlier, next) = (&pair[0], &pair[1]);
            if gap >= 2 {
                prop_assert_eq!(timeline.locate(next.offset_ms - 1), Location::InGap { after });
            } else if gap == 1 {
                // The only position between the segments is the end instant
                // of the earlier segment.
                prop_assert_eq!(
                    timeline.locate(next.offset_ms - 1),
                    Location::InSegment { index: earlier.index, within_ms: earlier.duration_ms }
                );
            }
        }
    }
}
