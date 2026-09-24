//! Turn smoothing and session-wide clustering, without models.

use diarize::{
    Analysis, SpeakerBounds, SpeakerSegment, Turn, WindowEmbedding, cluster_session,
    cluster_session_in_windows, smooth, turn_cuts,
};
use pyannote_local::segmentation::{
    FRAME_SIZE, WINDOW_SAMPLES, WindowActivity, frames_for_samples,
};

fn seg(start: f64, end: f64, speaker: usize) -> SpeakerSegment {
    SpeakerSegment {
        start,
        end,
        speaker,
    }
}

fn spans(turns: &[Turn]) -> Vec<(u64, u64, u32)> {
    turns
        .iter()
        .map(|t| (t.start_ms, t.end_ms, t.speaker))
        .collect()
}

#[test]
fn a_200_ms_same_speaker_split_is_merged() {
    let turns = smooth(&[(
        0,
        vec![seg(0.0, 4.0, 3), seg(4.2, 9.0, 3), seg(9.0, 14.0, 1)],
    )]);
    assert_eq!(spans(&turns), vec![(0, 9_000, 1), (9_000, 14_000, 2)]);
}

#[test]
fn a_400_ms_turn_between_two_long_turns_of_another_speaker_is_absorbed() {
    let turns = smooth(&[(
        0,
        vec![seg(0.0, 10.0, 0), seg(10.0, 10.4, 1), seg(10.4, 20.0, 0)],
    )]);
    assert_eq!(spans(&turns), vec![(0, 20_000, 1)]);
}

#[test]
fn a_short_turn_goes_to_the_neighbour_it_overlaps_more() {
    let turns = smooth(&[(
        0,
        vec![seg(0.0, 5.1, 0), seg(5.0, 5.4, 2), seg(5.2, 12.0, 1)],
    )]);
    // Overlaps speaker 0 by 100 ms and speaker 1 by 200 ms.
    assert_eq!(spans(&turns), vec![(0, 5_100, 1), (5_000, 12_000, 2)]);
}

#[test]
fn speakers_are_numbered_by_first_appearance_across_segments_and_never_merged_across_a_gap() {
    let turns = smooth(&[
        (0, vec![seg(0.0, 5.0, 7), seg(5.0, 10.0, 2)]),
        (1, vec![seg(0.0, 5.0, 2), seg(5.0, 9.0, 4)]),
    ]);
    assert_eq!(
        turns
            .iter()
            .map(|t| (t.segment, t.speaker))
            .collect::<Vec<_>>(),
        vec![(0, 1), (0, 2), (1, 2), (1, 3)]
    );
    // Speaker 2 ends segment 0 and opens segment 1: two turns, not one.
    assert_eq!(turns.iter().filter(|t| t.speaker == 2).count(), 2);
}

#[test]
fn cuts_fall_at_speaker_changes_within_a_segment() {
    let turns = smooth(&[(
        0,
        vec![seg(0.0, 5.0, 0), seg(6.0, 9.0, 1), seg(9.5, 12.0, 0)],
    )]);
    assert_eq!(turn_cuts(&turns, 0), vec![6_000, 9_500]);
    assert!(turn_cuts(&turns, 1).is_empty());
}

/// An analysis of `windows` windows where every window holds one anchor
/// embedding of the voice given by `voice(window)`.
fn synthetic(windows: usize, voice: impl Fn(usize) -> usize) -> Analysis {
    let step = 59 * FRAME_SIZE;
    let frames = frames_for_samples(WINDOW_SAMPLES);
    Analysis {
        total: WINDOW_SAMPLES + (windows - 1) * step,
        step,
        activities: vec![
            WindowActivity {
                frames: vec![[true, false, false]; frames]
            };
            windows
        ],
        embeddings: (0..windows)
            .map(|w| {
                let mut embedding = vec![0.0f32; 8];
                embedding[voice(w)] = 1.0;
                WindowEmbedding {
                    window: w,
                    local_speaker: 0,
                    embedding,
                    anchor: true,
                }
            })
            .collect(),
    }
}

#[test]
fn one_voice_on_both_sides_of_a_gap_keeps_one_label() {
    let a = synthetic(60, |w| usize::from(w >= 30));
    let b = synthetic(40, |_| 1);
    let out = cluster_session(&[&a, &b], SpeakerBounds::default());
    let last_of_a = out[0].last().unwrap().speaker;
    assert!(out[1].iter().all(|s| s.speaker == last_of_a));
}

#[test]
fn an_exact_count_bounds_the_session_not_each_segment() {
    // Three voices overall; the short second segment holds only one of them.
    let a = synthetic(90, |w| w / 30);
    let b = synthetic(30, |_| 2);
    let out = cluster_session(&[&a, &b], SpeakerBounds::exact(3));
    let labels: std::collections::BTreeSet<usize> =
        out.iter().flatten().map(|s| s.speaker).collect();
    assert_eq!(labels.len(), 3);
    let in_b: std::collections::BTreeSet<usize> = out[1].iter().map(|s| s.speaker).collect();
    assert_eq!(
        in_b.len(),
        1,
        "no cluster fabricated to fill the short segment"
    );
}

#[test]
fn a_session_past_the_cap_clusters_in_two_windows_whose_labels_never_collide() {
    // One voice throughout, 120 windows against a cap of 100: two clustering
    // windows (the real cap is 3,612; the rule is the same).
    let a = synthetic(120, |_| 0);
    let out = cluster_session_in_windows(&[&a], SpeakerBounds::default(), 100);
    let labels: std::collections::BTreeSet<usize> = out[0].iter().map(|s| s.speaker).collect();
    // Numbering continues across the boundary: the same voice carries a
    // second label rather than the windows sharing one.
    assert_eq!(labels, [0, 1].into_iter().collect());
}
