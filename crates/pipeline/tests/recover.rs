//! Launch recovery of a session a dead process left recording (U16).

mod common;

use chrono::TimeDelta;
use common::{store, write_segment, write_stereo_segment};
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
fn a_crash_in_a_stereo_segment_rebuilds_a_two_channel_row_and_the_stereo_mark() {
    // KTD6: the open segment has no row, so the channel count comes from
    // the first chunk's WAV header.
    let root = tempfile::tempdir().unwrap();
    let store = store();
    store
        .lock()
        .unwrap()
        .create_session("s", common::at(0), None)
        .unwrap();
    let dir = root.path().join("s");
    // A mono first segment, closed and in the store (AE3).
    let mono = write_segment(&dir, 0, 0, RATE, &tone(20.0), false);
    store.lock().unwrap().put_segment("s", &mono).unwrap();
    // The stereo second segment was recording when the process died.
    write_stereo_segment(&dir, 1, 20_030, RATE, &tone(75.0), &tone(75.0), true);

    let recovered = recover_crashed(&store, root.path()).unwrap();

    interrupted_since(&recovered[0].state);
    let store = store.lock().unwrap();
    let segments = store.segments("s").unwrap();
    let channels: Vec<u16> = segments.iter().map(|s| s.channels).collect();
    assert_eq!(channels, [1, 2]);
    assert!(
        (74_900..=75_000).contains(&segments[1].duration_ms),
        "{}",
        segments[1].duration_ms
    );
    assert_eq!(store.stereo_from("s").unwrap(), Some(1));
}

#[test]
fn a_crash_in_a_mono_note_sets_no_stereo_mark() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    store
        .lock()
        .unwrap()
        .create_session("s", common::at(0), None)
        .unwrap();
    write_segment(&root.path().join("s"), 0, 0, RATE, &tone(5.0), true);
    recover_crashed(&store, root.path()).unwrap();
    let store = store.lock().unwrap();
    assert_eq!(store.segments("s").unwrap()[0].channels, 1);
    assert_eq!(store.stereo_from("s").unwrap(), None);
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

// ------------------------------- old audio copy (U5 of the audio removal plan)

const OLD: &str = "audio/segment-01.opus";
const OLD_TOO: &str = "audio/segment-02.opus";

/// A resumed saved note `id` with two old segments: its home folder holds
/// their audio, and its working location holds their copies and one new
/// segment that the Resume recorded, still in chunks.
fn resumed_note(
    store: &pipeline::SharedStore,
    root: &std::path::Path,
    id: &str,
) -> std::path::PathBuf {
    let working = root.join(id);
    let home = root.join(format!("{id}-home"));
    let old = [
        write_segment(&working, 0, 0, RATE, &tone(5.0), false),
        write_segment(&working, 1, 10_000, RATE, &tone(5.0), false),
    ];
    std::fs::remove_dir_all(&working).unwrap();
    std::fs::create_dir_all(home.join("audio")).unwrap();
    for segment in &old {
        std::fs::write(home.join(&segment.audio_file), b"old audio").unwrap();
    }
    {
        let mut s = store.lock().unwrap();
        s.create_session(id, common::at(0), None).unwrap();
        for segment in &old {
            s.put_segment(id, segment).unwrap();
        }
        s.set_home_path(id, home.to_str().unwrap()).unwrap();
    }
    pipeline::old_audio::begin(&working, &old).unwrap();
    let from = home.clone();
    pipeline::old_audio::copy(&working, &move || Some(from.clone())).unwrap();
    // The segment that the Resume records, open when the process stops.
    write_segment(&working, 2, 100_000, RATE, &tone(5.0), true);
    working
}

fn apply(store: &pipeline::SharedStore, id: &str, events: &[session_core::Event]) {
    let mut s = store.lock().unwrap();
    for event in events {
        s.apply(id, event).unwrap();
    }
}

fn saved_events() -> Vec<session_core::Event> {
    vec![
        session_core::Event::UserStop {
            at: common::at(200),
            retained_audio: true,
        },
        session_core::Event::Dequeued,
        session_core::Event::Saved,
    ]
}

#[test]
fn a_crash_during_a_resumed_recording_keeps_the_old_audio_copy_until_the_note_ends() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    let working = resumed_note(&store, root.path(), "s");

    let recovered = recover_crashed(&store, root.path()).unwrap();
    interrupted_since(&recovered[0].state);
    assert!(
        working.join(OLD).is_file(),
        "an interrupted note keeps its copy"
    );
    assert!(working.join(OLD_TOO).is_file());

    // The note is saved, and the process stops before the save deletes the
    // working location. The next launch deletes the copy.
    apply(&store, "s", &saved_events());
    recover_crashed(&store, root.path()).unwrap();
    assert!(!working.join(OLD).exists());
    assert!(!working.join(OLD_TOO).exists());
    assert!(!working.join(pipeline::old_audio::LIST_FILE).exists());
    assert!(working.join(RECOVERY_DIR).is_dir(), "only the copy goes");
}

#[test]
fn launch_recovery_deletes_the_old_audio_copy_of_a_note_that_is_gone() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    let working = resumed_note(&store, root.path(), "s");
    apply(&store, "s", &saved_events());
    assert!(store.lock().unwrap().forget("s").unwrap());
    recover_crashed(&store, root.path()).unwrap();
    assert!(!working.join(OLD).exists());
    assert!(!working.join(OLD_TOO).exists());
}

#[test]
fn launch_recovery_of_a_failed_note_deletes_only_the_copies_of_marked_old_audio() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    let working = resumed_note(&store, root.path(), "s");
    apply(
        &store,
        "s",
        &[
            session_core::Event::UserStop {
                at: common::at(200),
                retained_audio: true,
            },
            session_core::Event::Dequeued,
            session_core::Event::StageFailed(session_core::Stage::Transcribe),
        ],
    );
    let home = root.path().join("s-home");
    // The marker comes while Longhand is stopped.
    std::fs::write(
        home.join(session_core::folder::removal_marker(OLD)),
        session_core::folder::REMOVAL_MARKER_TEXT,
    )
    .unwrap();
    recover_crashed(&store, root.path()).unwrap();
    assert!(!working.join(OLD).exists(), "the marked copy stayed");
    assert!(working.join(OLD_TOO).is_file(), "the unmarked copy went");
}
