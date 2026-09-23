//! Launch recovery of a session a dead process left recording (U16).

mod common;

use chrono::TimeDelta;
use common::{store, write_segment};
use pipeline::RECOVERY_DIR;
use pipeline::recover::recover_crashed;
use session_core::{InterruptReason, SessionState};

const RATE: u32 = 16_000;

fn tone(seconds: f32) -> Vec<f32> {
    vec![0.1; (seconds * RATE as f32) as usize]
}

fn interrupted_since(state: &SessionState) -> chrono::DateTime<chrono::FixedOffset> {
    match state {
        SessionState::Interrupted {
            reason: InterruptReason::Crash,
            since,
            acknowledged: false,
        } => *since,
        other => panic!("expected Interrupted(crash), got {other:?}"),
    }
}

#[test]
fn a_crash_mid_segment_becomes_an_interrupted_session_with_the_segment_rebuilt() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    store
        .lock()
        .unwrap()
        .create_session("s", common::at(0), None)
        .unwrap();
    // Two closed chunks and a 37 s `.part` with a stale header.
    write_segment(&root.path().join("s"), 0, 0, RATE, &tone(157.0), true);

    let recovered = recover_crashed(&store, root.path()).unwrap();

    assert_eq!(recovered.len(), 1);
    let segments = store.lock().unwrap().segments("s").unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].offset_ms, 0);
    assert!(
        (156_900..=157_000).contains(&segments[0].duration_ms),
        "{}",
        segments[0].duration_ms
    );
    assert_eq!(segments[0].audio_file, "audio/segment-01.opus");
    let since = interrupted_since(&recovered[0].state);
    assert_eq!(since, segments[0].wall_end);
    let parts = std::fs::read_dir(root.path().join("s").join(RECOVERY_DIR))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "part")
        })
        .count();
    assert_eq!(parts, 0);
}

#[test]
fn a_crash_that_kept_under_a_second_discards_the_session() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    store
        .lock()
        .unwrap()
        .create_session("s", common::at(0), None)
        .unwrap();
    write_segment(&root.path().join("s"), 0, 0, RATE, &tone(0.5), true);

    let recovered = recover_crashed(&store, root.path()).unwrap();

    assert_eq!(recovered[0].state, SessionState::Discarded);
    assert!(!root.path().join("s").exists());
}

#[test]
fn a_resumed_session_keeps_its_closed_segments_and_gains_the_crashed_one() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    let dir = root.path().join("s");
    {
        let mut s = store.lock().unwrap();
        s.create_session("s", common::at(0), None).unwrap();
        let first = write_segment(&dir, 0, 0, RATE, &tone(20.0), false);
        s.put_segment("s", &first).unwrap();
    }
    // Three hours into the session's timeline.
    write_segment(&dir, 1, 3 * 3_600_000, RATE, &tone(12.0), true);

    let recovered = recover_crashed(&store, root.path()).unwrap();

    let segments = store.lock().unwrap().segments("s").unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].index, 1);
    assert_eq!(segments[1].offset_ms, 3 * 3_600_000);
    assert_eq!(segments[1].audio_file, "audio/segment-02.opus");
    // The wait is measured from when the audio ended on the wall clock, not
    // from the three-hour timeline position.
    assert_eq!(interrupted_since(&recovered[0].state), segments[1].wall_end);
}

#[test]
fn a_crash_45_minutes_ago_is_queued_and_5_minutes_ago_is_offered_for_resume() {
    for (minutes, queued) in [(45, true), (5, false)] {
        let root = tempfile::tempdir().unwrap();
        let store = store();
        store
            .lock()
            .unwrap()
            .create_session("s", common::at(0), None)
            .unwrap();
        write_segment(&root.path().join("s"), 0, 0, RATE, &tone(30.0), true);
        let recovered = recover_crashed(&store, root.path()).unwrap();
        let since = interrupted_since(&recovered[0].state);

        let now = since + TimeDelta::minutes(minutes);
        let ticked = store.lock().unwrap().tick(now).unwrap();

        assert_eq!(!ticked.is_empty(), queued, "{minutes} min");
    }
}

#[test]
fn sessions_not_left_recording_are_untouched() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    common::queue_session(&store, root.path(), "q", 100, None, &[(RATE, tone(5.0))]);

    assert!(recover_crashed(&store, root.path()).unwrap().is_empty());
    let state = store.lock().unwrap().session("q").unwrap().unwrap().state;
    assert!(matches!(state, SessionState::Queued { .. }));
}
