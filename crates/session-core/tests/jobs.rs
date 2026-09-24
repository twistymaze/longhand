use chrono::{DateTime, FixedOffset, TimeDelta};
use session_core::jobs::Liveness;
use session_core::{Anchor, Event, InterruptReason, SessionState, Stage, Store};

fn t(minutes: i64) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-22T10:00:00-05:00").unwrap() + TimeDelta::minutes(minutes)
}

fn stop(store: &mut Store, id: &str, at: i64) {
    store
        .apply(
            id,
            &Event::UserStop {
                at: t(at),
                retained_audio: true,
            },
        )
        .unwrap();
}

#[test]
fn opens_with_wal_and_full_sync() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("longhand.db")).unwrap();
    assert_eq!(store.journal_mode().unwrap(), "wal");
    assert_eq!(store.synchronous().unwrap(), 2);
}

#[test]
fn queue_is_fifo_by_stop_and_resume_removes_from_it() {
    let mut store = Store::open_in_memory().unwrap();
    for (i, id) in ["s1", "s2", "s3"].iter().enumerate() {
        store.create_session(id, t(i as i64 * 10), None).unwrap();
        stop(&mut store, id, i as i64 * 10 + 5);
    }
    assert_eq!(store.queue().unwrap(), ["s1", "s2", "s3"]);

    // Any stopped session may resume, not only the newest (R6a).
    store
        .apply(
            "s2",
            &Event::UserResume {
                space_ok: true,
                audio_intact: true,
            },
        )
        .unwrap();
    assert_eq!(store.queue().unwrap(), ["s1", "s3"]);

    assert_eq!(
        store.dequeue("run-a", t(40)).unwrap().as_deref(),
        Some("s1")
    );
    assert_eq!(store.queue().unwrap(), ["s3"]);
    assert_eq!(store.claim_owner("s1").unwrap().as_deref(), Some("run-a"));
}

#[test]
fn a_dead_runs_claim_is_reclaimed_with_checkpoints_intact() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    stop(&mut store, "s1", 5);
    store.create_session("s2", t(10), None).unwrap();
    stop(&mut store, "s2", 15);
    store.dequeue("dead-run", t(20)).unwrap();
    store
        .put_checkpoint("s1", Stage::Transcribe, "window-0003", "{}")
        .unwrap();

    // Even a heartbeat from a second ago does not keep a foreign claim alive.
    store.heartbeat("s1", "dead-run", t(20)).unwrap();
    assert_eq!(
        store
            .reclaim_at_launch("new-run", Liveness::OnlyInstance)
            .unwrap(),
        ["s1"]
    );
    assert!(matches!(
        store.session("s1").unwrap().unwrap().state,
        SessionState::Queued { .. }
    ));
    assert_eq!(store.checkpoints("s1", Stage::Transcribe).unwrap().len(), 1);
    // It keeps its place at the head of the queue.
    assert_eq!(store.queue().unwrap(), ["s1", "s2"]);
}

#[test]
fn the_current_runs_claim_is_left_alone() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    stop(&mut store, "s1", 5);
    store.dequeue("run", t(6)).unwrap();
    assert!(
        store
            .reclaim_at_launch("run", Liveness::OnlyInstance)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn without_the_lock_a_fresh_heartbeat_keeps_a_claim_and_a_quiet_one_loses_it() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    stop(&mut store, "s1", 5);
    store.dequeue("other-run", t(10)).unwrap();
    store.heartbeat("s1", "other-run", t(100)).unwrap();

    // 5 s old: an owner that may still be alive keeps it.
    let fresh = Liveness::Unknown {
        now: t(100) + TimeDelta::seconds(5),
    };
    assert!(
        store
            .reclaim_at_launch("new-run", fresh)
            .unwrap()
            .is_empty()
    );
    // 2 min old: gone.
    let quiet = Liveness::Unknown { now: t(102) };
    assert_eq!(store.reclaim_at_launch("new-run", quiet).unwrap(), ["s1"]);
    assert!(matches!(
        store.session("s1").unwrap().unwrap().state,
        SessionState::Queued { .. }
    ));
}

#[test]
fn resume_keeps_transcription_and_drops_summary_checkpoints() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    stop(&mut store, "s1", 5);
    store.dequeue("run", t(6)).unwrap();
    store
        .put_checkpoint("s1", Stage::Transcribe, "w1", "a")
        .unwrap();
    store
        .put_checkpoint("s1", Stage::Summarize, "c1", "b")
        .unwrap();
    store
        .apply(
            "s1",
            &Event::UserResume {
                space_ok: true,
                audio_intact: true,
            },
        )
        .unwrap();
    assert_eq!(store.checkpoints("s1", Stage::Transcribe).unwrap().len(), 1);
    assert!(
        store
            .checkpoints("s1", Stage::Summarize)
            .unwrap()
            .is_empty()
    );
    assert!(store.queue().unwrap().is_empty());
}

#[test]
fn pending_notes_are_adopted_by_the_next_session() {
    let mut store = Store::open_in_memory().unwrap();
    store.add_pending_note("agenda: budget").unwrap();
    store.add_pending_note("ask about hiring").unwrap();
    store.create_session("s1", t(0), None).unwrap();

    assert!(store.pending_notes().unwrap().is_empty());
    let notes = store.notes("s1").unwrap();
    assert_eq!(notes.len(), 2);
    assert!(notes.iter().all(|n| n.anchor == Anchor::Unanchored));
}

#[test]
fn editing_a_note_never_moves_its_anchor() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    let anchor = Anchor::At {
        position_ms: 95_000,
        gap_anchored: false,
    };
    let id = store.add_note("s1", "first draft", anchor).unwrap();
    store.edit_note(id, "rewritten entirely").unwrap();
    let note = &store.notes("s1").unwrap()[0];
    assert_eq!(note.text, "rewritten entirely");
    assert_eq!(note.anchor, anchor);
}

#[test]
fn state_survives_reopening_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let mut store = Store::open(&path).unwrap();
        store.create_session("s1", t(0), None).unwrap();
        store
            .apply(
                "s1",
                &Event::CaptureLost {
                    reason: InterruptReason::Device,
                    at: t(3),
                },
            )
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.session("s1").unwrap().unwrap().state,
        SessionState::Interrupted {
            reason: InterruptReason::Device,
            since: t(3),
            acknowledged: false
        }
    );
}

#[test]
fn illegal_events_leave_the_store_unchanged() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    assert!(store.apply("s1", &Event::Saved).is_err());
    assert_eq!(
        store.session("s1").unwrap().unwrap().state,
        SessionState::Capturing
    );
}

fn segment(index: u32, offset_ms: u64) -> session_core::Segment {
    session_core::Segment {
        index,
        offset_ms,
        duration_ms: 60_000,
        wall_start: t(0),
        wall_end: t(1),
        audio_file: format!("audio/segment-{:02}.opus", index + 1),
    }
}

#[test]
fn the_shortcut_queues_the_open_session_and_starts_a_new_one() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    assert_eq!(store.start_session("s2", t(5), None).unwrap(), ["s1"]);
    assert!(matches!(
        store.session("s1").unwrap().unwrap().state,
        SessionState::Queued { .. }
    ));
    assert_eq!(
        store.session("s2").unwrap().unwrap().state,
        SessionState::Capturing
    );
}

#[test]
fn the_shortcut_discards_an_open_session_that_kept_no_audio() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.start_session("s2", t(5), None).unwrap();
    assert_eq!(
        store.session("s1").unwrap().unwrap().state,
        SessionState::Discarded
    );
    assert!(store.queue().unwrap().is_empty());
}

#[test]
fn tick_queues_only_interruptions_past_their_window() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    store
        .apply(
            "s1",
            &Event::CaptureLost {
                reason: InterruptReason::Device,
                at: t(1),
            },
        )
        .unwrap();
    assert!(store.tick(t(20)).unwrap().is_empty());
    assert_eq!(store.tick(t(32)).unwrap(), ["s1"]);
    assert_eq!(store.queue().unwrap(), ["s1"]);
}
