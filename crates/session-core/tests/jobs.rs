use chrono::{DateTime, FixedOffset, TimeDelta};
use session_core::jobs::Liveness;
use session_core::{
    CaptureChoice, Event, InterruptReason, Microphone, RecordingModel, SessionState, SpeakerCount,
    Stage, Store, StoreError, Title, TitleSource,
};

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

// The in-use markers of this computer's earlier runs are its own work
// (KTD5 of the plan for notes without a recording).
#[test]
fn a_run_gets_the_earlier_runs_newest_first_and_the_store_keeps_the_newest() {
    let mut store = Store::open_in_memory().unwrap();
    assert!(store.add_run("run-1").unwrap().is_empty());
    assert_eq!(store.add_run("run-2").unwrap(), ["run-1"]);
    assert_eq!(store.add_run("run-3").unwrap(), ["run-2", "run-1"]);
    for n in 4..=session_core::jobs::RUNS_KEPT + 2 {
        store.add_run(&format!("run-{n}")).unwrap();
    }
    let last = session_core::jobs::RUNS_KEPT + 3;
    let earlier = store.add_run(&format!("run-{last}")).unwrap();
    assert_eq!(earlier.len(), session_core::jobs::RUNS_KEPT - 1);
    assert_eq!(earlier[0], format!("run-{}", last - 1));
    assert!(!earlier.iter().any(|id| id == "run-1" || id == "run-3"));
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
fn a_proposal_never_replaces_a_title_the_user_set() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    assert!(
        store
            .set_title("s1", "Budget review", TitleSource::User)
            .unwrap()
    );
    assert!(
        !store
            .set_title("s1", "Weekly sync", TitleSource::Proposed)
            .unwrap()
    );
    assert_eq!(
        store.title("s1").unwrap(),
        Some(Title {
            text: "Budget review".into(),
            source: TitleSource::User
        })
    );
    // The user can still change it.
    assert!(store.set_title("s1", "Budget", TitleSource::User).unwrap());
    assert_eq!(store.title("s1").unwrap().unwrap().text, "Budget");
}

#[test]
fn a_proposal_replaces_an_earlier_proposal() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    assert_eq!(store.title("s1").unwrap(), None);
    assert!(
        store
            .set_title("s1", "First", TitleSource::Proposed)
            .unwrap()
    );
    assert!(
        store
            .set_title("s1", "Second", TitleSource::Proposed)
            .unwrap()
    );
    assert_eq!(
        store.session("s1").unwrap().unwrap().title,
        Some(Title {
            text: "Second".into(),
            source: TitleSource::Proposed
        })
    );
}

#[test]
fn an_empty_user_title_clears_the_title_and_a_later_proposal_is_kept() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store
        .set_title("s1", "Budget review", TitleSource::User)
        .unwrap();
    assert!(store.set_title("s1", "  ", TitleSource::User).unwrap());
    assert_eq!(store.title("s1").unwrap(), None);
    assert!(
        store
            .set_title("s1", "Weekly sync", TitleSource::Proposed)
            .unwrap()
    );
    assert_eq!(
        store.title("s1").unwrap(),
        Some(Title {
            text: "Weekly sync".into(),
            source: TitleSource::Proposed
        })
    );
}

#[test]
fn an_empty_proposal_is_refused() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store
        .set_title("s1", "First", TitleSource::Proposed)
        .unwrap();
    assert!(!store.set_title("s1", "", TitleSource::Proposed).unwrap());
    assert_eq!(store.title("s1").unwrap().unwrap().text, "First");
}

#[test]
fn a_title_for_no_session_is_an_error() {
    let store = Store::open_in_memory().unwrap();
    assert!(matches!(
        store.set_title("gone", "x", TitleSource::User),
        Err(StoreError::NoSuchSession(_))
    ));
}

#[test]
fn the_notes_document_is_kept_as_written() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    assert_eq!(store.notes("s1").unwrap(), "");
    let document = "# Agenda\n\n- [ ] budget\n- [x] hiring\n\n```\ncode  kept\n```\n";
    store.set_notes("s1", document).unwrap();
    assert_eq!(store.notes("s1").unwrap(), document);
    assert!(matches!(
        store.set_notes("gone", document),
        Err(StoreError::NoSuchSession(_))
    ));
    assert!(matches!(
        store.notes("gone"),
        Err(StoreError::NoSuchSession(_))
    ));
}

#[test]
fn the_found_speaker_count_is_kept_apart_from_the_bound() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .create_session("s1", t(0), Some(SpeakerCount::Exact(3)))
        .unwrap();
    assert_eq!(store.found_speakers("s1").unwrap(), None);
    store.set_found_speakers("s1", Some(4)).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.found_speakers, Some(4));
    assert_eq!(record.speaker_count, Some(SpeakerCount::Exact(3)));
    store.set_found_speakers("s1", None).unwrap();
    assert_eq!(store.found_speakers("s1").unwrap(), None);
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
        channels: 1,
        ends_in_pause: false,
    }
}

#[test]
fn the_shortcut_queues_the_open_session_and_starts_a_new_one() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    assert_eq!(
        store
            .start_session(
                "s2",
                t(5),
                None,
                &parakeet_live(),
                &CaptureChoice::default()
            )
            .unwrap(),
        ["s1"]
    );
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
    store
        .start_session(
            "s2",
            t(5),
            None,
            &parakeet_live(),
            &CaptureChoice::default(),
        )
        .unwrap();
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

// ------------------------------------------------------------ delete (U9)

use session_core::jobs::{Claims, DeleteError, delete_session};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A saved session "s" with a home folder and a working directory on disk.
struct Deletable {
    store: Mutex<Store>,
    home: PathBuf,
    working: PathBuf,
    _dir: tempfile::TempDir,
}

fn saved_session() -> Deletable {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("dest").join("2026-09-22 Budget review");
    let working = dir.path().join("working").join("s");
    std::fs::create_dir_all(home.join("audio")).unwrap();
    std::fs::write(home.join("transcript.md"), "x").unwrap();
    std::fs::create_dir_all(&working).unwrap();
    std::fs::write(working.join("recovery.wav"), "x").unwrap();

    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s", t(0), None).unwrap();
    store.put_segment("s", &segment(0, 0)).unwrap();
    stop(&mut store, "s", 5);
    store.dequeue("run", t(6)).unwrap();
    store
        .put_checkpoint("s", Stage::Transcribe, "w", "{}")
        .unwrap();
    store.apply("s", &Event::Saved).unwrap();
    store.set_home_path("s", home.to_str().unwrap()).unwrap();
    // A second session, which a delete leaves alone.
    store.create_session("other", t(20), None).unwrap();
    Deletable {
        store: Mutex::new(store),
        home,
        working,
        _dir: dir,
    }
}

/// A remover that records each call and removes the directory, as the
/// Trash would move it away.
fn recording(calls: &mut Vec<PathBuf>) -> impl FnMut(&Path) -> Result<(), String> + '_ {
    |path| {
        calls.push(path.to_path_buf());
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    }
}

fn rows_gone(store: &Mutex<Store>) -> bool {
    let store = store.lock().unwrap();
    store.session("s").unwrap().is_none()
        && store.segments("s").unwrap().is_empty()
        && store
            .checkpoints("s", Stage::Transcribe)
            .unwrap()
            .is_empty()
        && store.session("other").unwrap().is_some()
}

#[test]
fn a_saved_session_is_deleted_home_folder_first_then_working_directory_then_rows() {
    let d = saved_session();
    let mut calls = Vec::new();
    delete_session(
        &d.store,
        "s",
        &d.working,
        &Claims::default(),
        recording(&mut calls),
    )
    .unwrap();
    assert_eq!(calls, vec![d.home.clone(), d.working.clone()]);
    assert!(!d.home.exists() && !d.working.exists());
    assert!(rows_gone(&d.store));
    let listed: Vec<String> = d
        .store
        .lock()
        .unwrap()
        .sessions()
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(listed, ["other"]);
}

#[test]
fn a_busy_session_is_refused_and_nothing_changes() {
    let d = saved_session();
    // Resume makes it capture again; stop queues it; dequeue processes it.
    let mut store = d.store.lock().unwrap();
    store
        .apply(
            "s",
            &Event::UserResume {
                space_ok: true,
                audio_intact: true,
            },
        )
        .unwrap();
    drop(store);
    let busy = |d: &Deletable, want: &str| {
        let mut calls = Vec::new();
        let err = delete_session(
            &d.store,
            "s",
            &d.working,
            &Claims::default(),
            recording(&mut calls),
        )
        .unwrap_err();
        assert!(
            matches!(&err, DeleteError::Busy { state } if *state == want),
            "{err:?}"
        );
        assert!(calls.is_empty());
        assert!(d.home.exists() && d.working.exists());
        assert!(d.store.lock().unwrap().session("s").unwrap().is_some());
    };
    busy(&d, "capturing");
    d.store
        .lock()
        .unwrap()
        .apply(
            "s",
            &Event::CaptureLost {
                reason: InterruptReason::Device,
                at: t(30),
            },
        )
        .unwrap();
    busy(&d, "interrupted");
    stop(&mut d.store.lock().unwrap(), "s", 31);
    busy(&d, "queued");
    d.store.lock().unwrap().dequeue("run", t(32)).unwrap();
    // Covers AE5.
    busy(&d, "processing");
}

#[test]
fn a_session_whose_summary_is_regenerating_is_refused() {
    let d = saved_session();
    let mut calls = Vec::new();
    let claims = Claims::default();
    let _regeneration = claims.regenerate("s").unwrap();
    let err =
        delete_session(&d.store, "s", &d.working, &claims, recording(&mut calls)).unwrap_err();
    assert!(
        matches!(
            err,
            DeleteError::Busy {
                state: "regenerating"
            }
        ),
        "{err:?}"
    );
    assert!(calls.is_empty());
    assert!(d.store.lock().unwrap().session("s").unwrap().is_some());
}

#[test]
fn a_regeneration_and_a_delete_exclude_each_other_until_the_claim_is_dropped() {
    let claims = Claims::default();
    let regeneration = claims.regenerate("s").unwrap();
    assert_eq!(claims.delete("s").err(), Some("regenerating"));
    assert!(claims.delete("other").is_ok());
    assert!(claims.busy("s") && claims.is_regenerating("s"));
    drop(regeneration);
    assert!(!claims.busy("s"));
    let delete = claims.delete("s").unwrap();
    assert!(claims.regenerate("s").is_none());
    assert!(claims.busy("s") && !claims.is_regenerating("s"));
    drop(delete);
    assert!(!claims.busy("s"));
    assert!(claims.regenerate("s").is_some());
}

/// The Move's claims (KTD9): the move claim refuses a delete, and the hold
/// refuses a regeneration and makes an edit wait until it is released.
#[test]
fn the_move_claims_refuse_a_delete_and_a_hold_makes_an_edit_wait() {
    let claims = Claims::default();
    let moving = claims.moving("s").unwrap();
    assert!(claims.moving("s").is_none());
    assert!(claims.busy("s") && !claims.is_held("s"));
    assert_eq!(claims.delete("s").err(), Some("moving"));
    // A regeneration can run while the Move copies: the final check finds
    // its change.
    let regeneration = claims.regenerate("s").unwrap();
    assert!(claims.hold("s").is_none());
    drop(regeneration);
    let hold = claims.hold("s").unwrap();
    assert!(claims.is_held("s"));
    assert!(claims.regenerate("s").is_none());

    let waited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let waiter = {
        let (claims, waited) = (claims.clone(), std::sync::Arc::clone(&waited));
        std::thread::spawn(move || {
            claims.wait_unheld("s");
            waited.store(true, std::sync::atomic::Ordering::SeqCst);
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!waited.load(std::sync::atomic::Ordering::SeqCst));
    drop(hold);
    waiter.join().unwrap();
    assert!(waited.load(std::sync::atomic::Ordering::SeqCst));
    drop(moving);
    assert!(!claims.busy("s"));
    assert!(claims.moving("s").is_some());
}

#[test]
fn a_move_waits_for_a_regeneration_or_a_delete() {
    let claims = Claims::default();
    let regeneration = claims.regenerate("s").unwrap();
    assert!(claims.moving("s").is_none());
    assert!(claims.is_regenerating_or_deleting("s"));
    drop(regeneration);
    let delete = claims.delete("s").unwrap();
    assert!(claims.moving("s").is_none() && claims.hold("s").is_none());
    drop(delete);
    assert!(!claims.is_regenerating_or_deleting("s"));
}

#[test]
fn a_delete_releases_its_claim_when_it_ends() {
    let d = saved_session();
    let claims = Claims::default();
    let mut calls = Vec::new();
    delete_session(&d.store, "s", &d.working, &claims, |path: &Path| {
        // A regeneration that starts while the folder is removed is refused.
        assert!(claims.regenerate("s").is_none());
        recording(&mut calls)(path)
    })
    .unwrap();
    assert!(!claims.busy("s"));
    assert!(rows_gone(&d.store));
}

#[test]
fn a_failure_on_the_home_folder_leaves_everything_in_place() {
    // Covers AE10.
    let d = saved_session();
    let mut calls = Vec::new();
    let err = delete_session(
        &d.store,
        "s",
        &d.working,
        &Claims::default(),
        |path: &Path| {
            calls.push(path.to_path_buf());
            Err("the volume has no Trash".to_string())
        },
    )
    .unwrap_err();
    match err {
        DeleteError::Remove { path, message } => {
            assert_eq!(path, d.home);
            assert_eq!(message, "the volume has no Trash");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(calls, vec![d.home.clone()]);
    assert!(d.home.exists() && d.working.exists());
    assert!(d.store.lock().unwrap().session("s").unwrap().is_some());
}

#[test]
fn a_failure_on_the_working_directory_keeps_the_rows() {
    let d = saved_session();
    let working = d.working.clone();
    let err = delete_session(
        &d.store,
        "s",
        &d.working,
        &Claims::default(),
        |path: &Path| {
            if path == working {
                return Err("busy".to_string());
            }
            std::fs::remove_dir_all(path).map_err(|e| e.to_string())
        },
    )
    .unwrap_err();
    assert!(matches!(err, DeleteError::Remove { ref path, .. } if *path == d.working));
    assert!(!d.home.exists());
    assert!(d.store.lock().unwrap().session("s").unwrap().is_some());
    // A second try, with the home folder already gone, completes.
    let mut calls = Vec::new();
    delete_session(
        &d.store,
        "s",
        &d.working,
        &Claims::default(),
        recording(&mut calls),
    )
    .unwrap();
    assert_eq!(calls, vec![d.working.clone()]);
    assert!(rows_gone(&d.store));
}

#[test]
fn a_session_never_saved_deletes_its_working_directory_only() {
    let d = saved_session();
    {
        let mut store = d.store.lock().unwrap();
        store.create_session("n", t(40), None).unwrap();
        store
            .apply(
                "n",
                &Event::UserStop {
                    at: t(41),
                    retained_audio: false,
                },
            )
            .unwrap();
    }
    let working = d.working.with_file_name("n");
    std::fs::create_dir_all(&working).unwrap();
    let mut calls = Vec::new();
    delete_session(
        &d.store,
        "n",
        &working,
        &Claims::default(),
        recording(&mut calls),
    )
    .unwrap();
    assert_eq!(calls, vec![working.clone()]);
    assert!(d.store.lock().unwrap().session("n").unwrap().is_none());
    assert!(d.home.exists());
}

#[test]
fn a_retry_after_a_crash_with_both_directories_gone_deletes_the_rows() {
    let d = saved_session();
    std::fs::remove_dir_all(&d.home).unwrap();
    std::fs::remove_dir_all(&d.working).unwrap();
    let mut calls = Vec::new();
    delete_session(
        &d.store,
        "s",
        &d.working,
        &Claims::default(),
        recording(&mut calls),
    )
    .unwrap();
    assert!(calls.is_empty());
    assert!(rows_gone(&d.store));
}

#[test]
fn a_remover_that_fails_on_a_directory_already_gone_counts_it_as_removed() {
    // Another process removed the folder between the check and the call.
    let d = saved_session();
    delete_session(
        &d.store,
        "s",
        &d.working,
        &Claims::default(),
        |path: &Path| {
            std::fs::remove_dir_all(path).unwrap();
            Err("the item could not be found".to_string())
        },
    )
    .unwrap();
    assert!(rows_gone(&d.store));
}

#[test]
fn deleting_no_such_session_is_an_error() {
    let d = saved_session();
    let err = delete_session(
        &d.store,
        "nope",
        &d.working,
        &Claims::default(),
        |_: &Path| Ok(()),
    )
    .unwrap_err();
    assert!(
        matches!(err, DeleteError::Store(StoreError::NoSuchSession(_))),
        "{err:?}"
    );
}

#[test]
fn an_owed_rename_is_kept_until_it_is_cleared() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.create_session("s2", t(1), None).unwrap();
    assert!(store.owed_renames().unwrap().is_empty());
    store.set_rename_owed("s1", true).unwrap();
    store.set_rename_owed("s1", true).unwrap();
    store.set_rename_owed("s2", true).unwrap();
    assert_eq!(store.owed_renames().unwrap(), ["s1", "s2"]);
    store.set_rename_owed("s1", false).unwrap();
    assert_eq!(store.owed_renames().unwrap(), ["s2"]);
    assert!(store.set_rename_owed("nobody", true).is_err());
}

#[test]
fn a_rename_intent_is_kept_until_it_is_cleared_and_goes_with_its_session() {
    use session_core::RenameIntent;
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    assert_eq!(store.rename_intent("s1").unwrap(), None);
    let first = RenameIntent {
        from: "/d/a".into(),
        to: "/d/b".into(),
        temp: None,
    };
    store.set_rename_intent("s1", &first).unwrap();
    let second = RenameIntent {
        from: "/d/a".into(),
        to: "/d/A".into(),
        temp: Some("/d/.longhand-rename-s1".into()),
    };
    store.set_rename_intent("s1", &second).unwrap();
    assert_eq!(store.rename_intent("s1").unwrap(), Some(second.clone()));
    assert_eq!(
        store.rename_intents().unwrap(),
        vec![("s1".to_string(), second)]
    );
    store.clear_rename_intent("s1").unwrap();
    assert!(store.rename_intents().unwrap().is_empty());
    assert!(store.set_rename_intent("nobody", &first).is_err());
}

const PARAKEET: &str = "parakeet-unified-en-0.6b-q80";
const WHISPER: &str = "whisper-large-v3-turbo-q5km";

fn parakeet_live() -> RecordingModel {
    RecordingModel {
        asr_model: PARAKEET.into(),
        live_transcript: true,
    }
}

fn whisper() -> RecordingModel {
    RecordingModel {
        asr_model: WHISPER.into(),
        live_transcript: false,
    }
}

#[test]
fn a_session_records_its_model_and_live_mode_at_start() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .start_session(
            "s1",
            t(0),
            None,
            &parakeet_live(),
            &CaptureChoice::default(),
        )
        .unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.asr_model.as_deref(), Some(PARAKEET));
    assert_eq!(record.live_transcript, Some(true));
}

#[test]
fn a_model_change_during_recording_does_not_change_the_session() {
    // AE6: the stop offers the model selected now, Whisper. The session
    // keeps the model it started with.
    let mut store = Store::open_in_memory().unwrap();
    store
        .start_session(
            "s1",
            t(0),
            None,
            &parakeet_live(),
            &CaptureChoice::default(),
        )
        .unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    stop(&mut store, "s1", 5);
    assert!(!store.record_model_if_unset("s1", WHISPER).unwrap());
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.asr_model.as_deref(), Some(PARAKEET));
    assert_eq!(record.live_transcript, Some(true));
}

#[test]
fn a_session_with_no_model_at_stop_gets_the_selected_model_and_no_live_mode() {
    // A recording that was in progress at the update has no model.
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.asr_model, None);
    assert_eq!(record.live_transcript, None);
    assert!(store.record_model_if_unset("s1", WHISPER).unwrap());
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.asr_model.as_deref(), Some(WHISPER));
    assert_eq!(record.live_transcript, Some(false));
}

#[test]
fn a_recording_model_replaces_the_model_and_the_live_mode() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .start_session(
            "s1",
            t(0),
            None,
            &parakeet_live(),
            &CaptureChoice::default(),
        )
        .unwrap();
    store.set_recording_model("s1", &whisper()).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.asr_model.as_deref(), Some(WHISPER));
    assert_eq!(record.live_transcript, Some(false));
}

#[test]
fn a_session_stopped_by_a_new_session_keeps_its_model() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .start_session(
            "s1",
            t(0),
            None,
            &parakeet_live(),
            &CaptureChoice::default(),
        )
        .unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    assert_eq!(
        store
            .start_session("s2", t(5), None, &whisper(), &CaptureChoice::default())
            .unwrap(),
        ["s1"]
    );
    assert!(!store.record_model_if_unset("s1", WHISPER).unwrap());
    let s1 = store.session("s1").unwrap().unwrap();
    assert_eq!(s1.asr_model.as_deref(), Some(PARAKEET));
    let s2 = store.session("s2").unwrap().unwrap();
    assert_eq!(s2.asr_model.as_deref(), Some(WHISPER));
    assert_eq!(s2.live_transcript, Some(false));
}

#[test]
fn model_writes_for_no_session_are_errors() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.set_recording_model("nobody", &whisper()).is_err());
    assert!(store.record_model_if_unset("nobody", WHISPER).is_err());
}

// ---------------------------------------------------------------- transcribe again (U9)

const STAGES: [Stage; 7] = [
    Stage::Assemble,
    Stage::Diarize,
    Stage::Transcribe,
    Stage::Join,
    Stage::Verify,
    Stage::Summarize,
    Stage::Save,
];

/// A saved session recorded with Parakeet, with one checkpoint in each stage.
fn saved_with_checkpoints() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store
        .start_session(
            "s1",
            t(0),
            None,
            &parakeet_live(),
            &CaptureChoice::default(),
        )
        .unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    stop(&mut store, "s1", 5);
    store.dequeue("run", t(6)).unwrap();
    for stage in STAGES {
        store.put_checkpoint("s1", stage, "old", "\"old\"").unwrap();
    }
    store.apply("s1", &Event::Saved).unwrap();
    store
}

fn keys(store: &Store, stage: Stage) -> Vec<String> {
    store
        .checkpoints("s1", stage)
        .unwrap()
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

fn again(at: i64) -> Event {
    Event::UserTranscribeAgain {
        at: t(at),
        audio_intact: true,
        capture_active: false,
        regenerating: false,
    }
}

fn resume(store: &mut Store) {
    store
        .apply(
            "s1",
            &Event::UserResume {
                space_ok: true,
                audio_intact: true,
            },
        )
        .unwrap();
}

#[test]
fn transcribe_again_queues_the_note_with_its_model_and_sets_aside_the_checkpoints() {
    let mut store = saved_with_checkpoints();
    let state = store.transcribe_again("s1", WHISPER, &again(10)).unwrap();
    assert_eq!(state, SessionState::Queued { stopped_at: t(10) });
    assert_eq!(store.queue().unwrap(), ["s1"]);
    let marks = store.transcribe_again_marks("s1").unwrap();
    assert_eq!(marks.model.as_deref(), Some(WHISPER));
    assert_eq!(marks.error, None);
    // The model of the note changes only after the save (KTD14).
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.asr_model.as_deref(), Some(PARAKEET));
    assert_eq!(record.transcribe_again_model.as_deref(), Some(WHISPER));
    // Every stage runs again. Assemble decodes the audio from the folder.
    for stage in STAGES {
        assert!(keys(&store, stage).is_empty(), "{stage:?}");
    }
}

#[test]
fn a_refused_transcribe_again_changes_nothing() {
    let mut store = saved_with_checkpoints();
    let refused = Event::UserTranscribeAgain {
        at: t(10),
        audio_intact: false,
        capture_active: false,
        regenerating: false,
    };
    assert!(matches!(
        store.transcribe_again("s1", WHISPER, &refused),
        Err(StoreError::Illegal(_))
    ));
    assert_eq!(
        store.session("s1").unwrap().unwrap().state,
        SessionState::Saved
    );
    assert_eq!(store.transcribe_again_marks("s1").unwrap().model, None);
    for stage in STAGES {
        assert_eq!(keys(&store, stage), ["old"], "{stage:?}");
    }
}

#[test]
fn a_failed_transcribe_again_returns_to_saved_with_the_previous_checkpoints() {
    let mut store = saved_with_checkpoints();
    store.transcribe_again("s1", WHISPER, &again(10)).unwrap();
    store.dequeue("run", t(11)).unwrap();
    store
        .put_checkpoint("s1", Stage::Transcribe, "new", "\"new\"")
        .unwrap();
    let state = store
        .fail_transcribe_again("s1", "the decoder failed")
        .unwrap();
    assert_eq!(state, SessionState::Saved);
    assert!(store.queue().unwrap().is_empty());
    for stage in STAGES {
        assert_eq!(keys(&store, stage), ["old"], "{stage:?}");
    }
    let marks = store.transcribe_again_marks("s1").unwrap();
    assert_eq!(marks.model, None);
    assert_eq!(marks.error.as_deref(), Some("the decoder failed"));
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.asr_model.as_deref(), Some(PARAKEET));
    // The next run clears the error.
    store.transcribe_again("s1", WHISPER, &again(20)).unwrap();
    assert_eq!(store.transcribe_again_marks("s1").unwrap().error, None);
}

#[test]
fn a_saved_transcribe_again_changes_the_model_and_drops_the_previous_checkpoints() {
    let mut store = saved_with_checkpoints();
    store.transcribe_again("s1", WHISPER, &again(10)).unwrap();
    store.dequeue("run", t(11)).unwrap();
    store
        .finish_transcribe_again("s1", Some("the endpoint is offline"))
        .unwrap();
    store.apply("s1", &Event::Saved).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.asr_model.as_deref(), Some(WHISPER));
    assert_eq!(record.transcribe_again_model, None);
    let marks = store.transcribe_again_marks("s1").unwrap();
    assert_eq!(marks.model, None);
    assert_eq!(
        marks.summary_error.as_deref(),
        Some("the endpoint is offline")
    );
    // A later failure has no previous checkpoints to restore.
    for stage in STAGES {
        assert!(keys(&store, stage).is_empty(), "{stage:?}");
    }
    store.clear_summary_not_updated("s1").unwrap();
    assert_eq!(
        store.transcribe_again_marks("s1").unwrap().summary_error,
        None
    );
}

#[test]
fn a_save_that_is_no_transcribe_again_keeps_the_model_and_clears_the_notices() {
    let mut store = saved_with_checkpoints();
    store.transcribe_again("s1", WHISPER, &again(10)).unwrap();
    store.dequeue("run", t(11)).unwrap();
    store.fail_transcribe_again("s1", "failed").unwrap();
    // A resume, then the save of the new recording (R6d).
    resume(&mut store);
    stop(&mut store, "s1", 30);
    store.dequeue("run", t(31)).unwrap();
    store
        .finish_transcribe_again("s1", Some("ignored"))
        .unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.asr_model.as_deref(), Some(PARAKEET));
    let marks = store.transcribe_again_marks("s1").unwrap();
    assert_eq!(
        (marks.model, marks.error, marks.summary_error),
        (None, None, None)
    );
}

#[test]
fn a_resume_of_a_queued_transcribe_again_is_a_resume_of_the_saved_note() {
    let mut store = saved_with_checkpoints();
    store.transcribe_again("s1", WHISPER, &again(10)).unwrap();
    store.dequeue("run", t(11)).unwrap();
    store
        .put_checkpoint("s1", Stage::Transcribe, "new", "\"new\"")
        .unwrap();
    resume(&mut store);
    assert_eq!(store.transcribe_again_marks("s1").unwrap().model, None);
    // As a resume from Saved: assembly and transcription stay, the rest
    // runs again (R6d).
    assert_eq!(keys(&store, Stage::Assemble), ["old"]);
    assert_eq!(keys(&store, Stage::Transcribe), ["old"]);
    for stage in [
        Stage::Diarize,
        Stage::Join,
        Stage::Verify,
        Stage::Summarize,
        Stage::Save,
    ] {
        assert!(keys(&store, stage).is_empty(), "{stage:?}");
    }
    // A later failure is a normal failure: no Transcribe Again runs.
    stop(&mut store, "s1", 30);
    store.dequeue("run", t(31)).unwrap();
    assert!(store.fail_transcribe_again("s1", "x").is_err());
}

fn usb_mic() -> CaptureChoice {
    CaptureChoice {
        microphone: Some(Microphone {
            id: "usb-uid".into(),
            label: "USB Audio".into(),
        }),
        system_audio: true,
    }
}

#[test]
fn a_session_records_its_capture_choice_at_start() {
    // KTD2: restart, resume and the capture watcher read this, not Settings.
    let mut store = Store::open_in_memory().unwrap();
    store
        .start_session("s1", t(0), None, &parakeet_live(), &usb_mic())
        .unwrap();
    assert_eq!(store.capture_choice("s1").unwrap(), usb_mic());
}

#[test]
fn a_capture_choice_change_updates_only_its_session() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .start_session("s1", t(0), None, &parakeet_live(), &usb_mic())
        .unwrap();
    store.create_session("s2", t(1), None).unwrap();
    store.set_capture_choice("s2", &usb_mic()).unwrap();
    store
        .set_capture_choice("s1", &CaptureChoice::default())
        .unwrap();
    assert_eq!(
        store.capture_choice("s1").unwrap(),
        CaptureChoice::default()
    );
    assert_eq!(store.capture_choice("s2").unwrap(), usb_mic());
    assert!(matches!(
        store.set_capture_choice("none", &usb_mic()),
        Err(StoreError::NoSuchSession(_))
    ));
    assert!(matches!(
        store.capture_choice("none"),
        Err(StoreError::NoSuchSession(_))
    ));
}

#[test]
fn a_segment_row_keeps_its_channel_count() {
    // KTD6: each segment row records its channel count.
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    let stereo = session_core::Segment {
        channels: 2,
        ..segment(1, 120_000)
    };
    store.put_segment("s1", &stereo).unwrap();
    let channels: Vec<u16> = store
        .segments("s1")
        .unwrap()
        .iter()
        .map(|s| s.channels)
        .collect();
    assert_eq!(channels, [1, 2]);
}

fn pause_flags(store: &Store, id: &str) -> Vec<bool> {
    store
        .segments(id)
        .unwrap()
        .iter()
        .map(|s| s.ends_in_pause)
        .collect()
}

fn resume_event() -> Event {
    Event::UserResume {
        space_ok: true,
        audio_intact: true,
    }
}

#[test]
fn the_pause_flag_marks_only_the_last_segment_and_survives_a_reopen() {
    // KTD4 of the pause plan.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let mut store = Store::open(&path).unwrap();
        store.create_session("s1", t(0), None).unwrap();
        store.put_segment("s1", &segment(0, 0)).unwrap();
        store.put_segment("s1", &segment(1, 120_000)).unwrap();
        assert!(store.mark_pause("s1").unwrap());
        assert_eq!(pause_flags(&store, "s1"), [false, true]);
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(pause_flags(&store, "s1"), [false, true]);
}

#[test]
fn the_pause_flag_on_a_note_with_no_segment_changes_nothing() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    assert!(!store.mark_pause("s1").unwrap());
    assert!(store.segments("s1").unwrap().is_empty());
}

#[test]
fn resuming_an_interrupted_note_clears_its_pause_flag() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.create_session("s2", t(0), None).unwrap();
    for id in ["s1", "s2"] {
        store.put_segment(id, &segment(0, 0)).unwrap();
        store.put_segment(id, &segment(1, 120_000)).unwrap();
        store.mark_pause(id).unwrap();
    }
    store
        .apply(
            "s1",
            &Event::CaptureLost {
                reason: InterruptReason::Device,
                at: t(3),
            },
        )
        .unwrap();
    store.apply("s1", &resume_event()).unwrap();
    assert_eq!(pause_flags(&store, "s1"), [false, false]);
    // The resume of one note leaves the flag of another note.
    assert_eq!(pause_flags(&store, "s2"), [false, true]);
}

#[test]
fn a_paused_note_that_stops_and_resumes_has_no_pause_flag() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    store.put_segment("s1", &segment(1, 120_000)).unwrap();
    store.mark_pause("s1").unwrap();
    stop(&mut store, "s1", 5);
    // The stop keeps the flag, so the transcript can show the pause.
    assert_eq!(pause_flags(&store, "s1"), [false, true]);
    store.apply("s1", &resume_event()).unwrap();
    assert_eq!(pause_flags(&store, "s1"), [false, false]);
}

#[test]
fn a_resume_keeps_the_flag_of_a_pause_that_the_note_continued_after() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    store.mark_pause("s1").unwrap();
    // Continue, then a second pause.
    store.put_segment("s1", &segment(1, 120_000)).unwrap();
    store.put_segment("s1", &segment(2, 240_000)).unwrap();
    store.mark_pause("s1").unwrap();
    stop(&mut store, "s1", 5);
    assert_eq!(pause_flags(&store, "s1"), [true, false, true]);
    store.apply("s1", &resume_event()).unwrap();
    assert_eq!(pause_flags(&store, "s1"), [true, false, false]);
}

#[test]
fn the_stereo_mark_is_the_first_stereo_segment() {
    // KTD2: set when the first stereo segment is kept, and never moved.
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    assert_eq!(store.stereo_from("s1").unwrap(), None);
    assert!(store.mark_stereo_from("s1", 2).unwrap());
    assert!(!store.mark_stereo_from("s1", 5).unwrap());
    assert_eq!(store.stereo_from("s1").unwrap(), Some(2));
    assert!(matches!(
        store.stereo_from("none"),
        Err(StoreError::NoSuchSession(_))
    ));
    assert!(matches!(
        store.mark_stereo_from("none", 0),
        Err(StoreError::NoSuchSession(_))
    ));
}

#[test]
fn a_session_created_with_no_choice_has_the_system_default() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    assert_eq!(
        store.capture_choice("s1").unwrap(),
        CaptureChoice::default()
    );
}

#[test]
fn the_capture_choice_survives_a_reopen_of_the_store() {
    // A crash resume reads the stored choice (KTD2).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let mut store = Store::open(&path).unwrap();
        store
            .start_session("s1", t(0), None, &parakeet_live(), &usb_mic())
            .unwrap();
    }
    assert_eq!(
        Store::open(&path).unwrap().capture_choice("s1").unwrap(),
        usb_mic()
    );
}

// ---------------------------------------------------------------- adopt and forget

fn adoption(id: &str, home: &str) -> session_core::Adoption {
    let mut stereo = segment(1, 60_000);
    stereo.channels = 2;
    session_core::Adoption {
        id: id.to_string(),
        created_at: t(0),
        speaker_count: Some(SpeakerCount::Exact(2)),
        segments: vec![segment(0, 0), stereo],
        notes: "- agenda\n".to_string(),
        title: Some(Title {
            text: "Budget".to_string(),
            source: TitleSource::User,
        }),
        asr_model: Some("whisper-large-v3-turbo-q5km".to_string()),
        recognition_snapshot: None,
        found_speakers: Some(3),
        home_path: home.to_string(),
        transcript_fingerprint: Some("abc123".to_string()),
        removed_audio: Vec::new(),
        note_type: None,
        tags: Vec::new(),
        pending_type: None,
    }
}

// ---------------------------------------------------------------- note type and tags

fn pending(to: Option<&str>) -> session_core::folder::PendingType {
    session_core::folder::PendingType {
        to: to.map(str::to_string),
        from: None,
        by: "computer-b".to_string(),
        at: t(5_000),
    }
}

#[test]
fn an_adopted_note_has_the_type_tags_and_pending_type_of_its_folder() {
    let mut store = Store::open_in_memory().unwrap();
    let mut note = adoption("s1", "/notes/Pathfinder/Budget");
    note.note_type = Some("Pathfinder".to_string());
    note.tags = vec!["Person::Nathan".to_string(), "follow-up".to_string()];
    note.pending_type = Some(pending(Some("One on One")));
    store.adopt(&note).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.note_type.as_deref(), Some("Pathfinder"));
    assert_eq!(record.tags, note.tags);
    assert_eq!(record.pending_type, note.pending_type);
}

#[test]
fn a_new_note_has_no_type_no_tags_and_no_pending_type() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.note_type, None);
    assert!(record.tags.is_empty());
    assert_eq!(record.pending_type, None);
}

#[test]
fn the_store_records_the_type_tags_and_pending_type() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.set_note_type("s1", Some("One on One")).unwrap();
    store
        .set_tags("s1", &["Topic::Security".to_string()])
        .unwrap();
    store.set_pending_type("s1", Some(&pending(None))).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.note_type.as_deref(), Some("One on One"));
    assert_eq!(record.tags, vec!["Topic::Security".to_string()]);
    assert_eq!(record.pending_type, Some(pending(None)));

    store.set_note_type("s1", None).unwrap();
    store.set_tags("s1", &[]).unwrap();
    store.set_pending_type("s1", None).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.note_type, None);
    assert!(record.tags.is_empty());
    assert_eq!(record.pending_type, None);
}

#[test]
fn the_type_tags_and_pending_type_of_an_unknown_note_are_an_error() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.set_note_type("nope", None).is_err());
    assert!(store.set_tags("nope", &[]).is_err());
    assert!(store.set_pending_type("nope", None).is_err());
}

// ---------------------------------------------------------------- removed audio (U2)

#[test]
fn a_new_segment_has_its_audio() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    assert!(store.removed_audio("s1").unwrap().is_empty());
}

#[test]
fn the_removed_flags_follow_the_given_audio_files_and_report_a_change() {
    let mut store = Store::open_in_memory().unwrap();
    store.adopt(&adoption("s1", "/notes/Budget")).unwrap();
    let one = "audio/segment-01.opus".to_string();
    let two = "audio/segment-02.opus".to_string();
    assert!(
        store
            .set_removed_audio("s1", std::slice::from_ref(&one))
            .unwrap()
    );
    assert_eq!(store.removed_audio("s1").unwrap(), vec![one.clone()]);
    // The same flags again change nothing.
    assert!(
        !store
            .set_removed_audio("s1", std::slice::from_ref(&one))
            .unwrap()
    );
    assert!(
        store
            .set_removed_audio("s1", &[two.clone(), one.clone()])
            .unwrap()
    );
    assert_eq!(store.removed_audio("s1").unwrap(), vec![one, two]);
    // An audio file that no segment names sets no flag.
    assert!(
        store
            .set_removed_audio("s1", &["audio/other.opus".to_string()])
            .unwrap()
    );
    assert!(store.removed_audio("s1").unwrap().is_empty());
}

#[test]
fn a_segment_written_again_keeps_its_removed_flag() {
    let mut store = Store::open_in_memory().unwrap();
    store.adopt(&adoption("s1", "/notes/Budget")).unwrap();
    let one = "audio/segment-01.opus".to_string();
    store
        .set_removed_audio("s1", std::slice::from_ref(&one))
        .unwrap();
    let mut longer = segment(0, 0);
    longer.duration_ms = 90_000;
    store.put_segment("s1", &longer).unwrap();
    assert_eq!(store.segments("s1").unwrap()[0].duration_ms, 90_000);
    assert_eq!(store.removed_audio("s1").unwrap(), vec![one]);
}

#[test]
fn an_adopted_note_keeps_the_removed_flags_of_its_folder() {
    let mut store = Store::open_in_memory().unwrap();
    let mut note = adoption("s1", "/notes/Budget");
    note.removed_audio = vec!["audio/segment-02.opus".to_string()];
    store.adopt(&note).unwrap();
    assert_eq!(
        store.removed_audio("s1").unwrap(),
        vec!["audio/segment-02.opus".to_string()]
    );
}

#[test]
fn the_removed_audio_of_an_unknown_note_is_an_error() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.set_removed_audio("nope", &[]).is_err());
}

#[test]
fn an_adopted_note_is_saved_with_the_fields_of_its_folder() {
    let mut store = Store::open_in_memory().unwrap();
    assert!(store.adopt(&adoption("s1", "/notes/Budget")).unwrap());
    let sessions = store.sessions().unwrap();
    assert_eq!(sessions.len(), 1);
    let record = &sessions[0];
    assert_eq!(record.id, "s1");
    assert_eq!(record.state, SessionState::Saved);
    assert_eq!(record.created_at, t(0));
    assert_eq!(record.queue_seq, None);
    assert_eq!(record.speaker_count, Some(SpeakerCount::Exact(2)));
    assert_eq!(
        record.asr_model.as_deref(),
        Some("whisper-large-v3-turbo-q5km")
    );
    assert_eq!(record.home_path.as_deref(), Some("/notes/Budget"));
    assert_eq!(
        record.title,
        Some(Title {
            text: "Budget".to_string(),
            source: TitleSource::User,
        })
    );
    assert_eq!(record.found_speakers, Some(3));
    assert_eq!(store.notes("s1").unwrap(), "- agenda\n");
    assert_eq!(store.segments("s1").unwrap(), adoption("s1", "").segments);
    assert_eq!(store.stereo_from("s1").unwrap(), Some(1));
    assert_eq!(
        store.transcript_fingerprint("s1").unwrap().as_deref(),
        Some("abc123")
    );
    assert!(store.queue().unwrap().is_empty());
    // No capture choice comes with the folder (R4).
    assert_eq!(
        store.capture_choice("s1").unwrap(),
        CaptureChoice::default()
    );
}

#[test]
fn an_adopted_note_accepts_an_edit_and_a_resume() {
    let mut store = Store::open_in_memory().unwrap();
    store.adopt(&adoption("s1", "/notes/Budget")).unwrap();
    assert_eq!(
        store.apply("s1", &Event::SavedEdit).unwrap(),
        SessionState::Saved
    );
    assert_eq!(
        store
            .apply(
                "s1",
                &Event::UserResume {
                    space_ok: true,
                    audio_intact: true,
                },
            )
            .unwrap(),
        SessionState::Capturing
    );
    // The resumed note keeps its folder as its home.
    assert_eq!(
        store.session("s1").unwrap().unwrap().home_path.as_deref(),
        Some("/notes/Budget")
    );
}

#[test]
fn an_adopted_proposed_title_accepts_a_new_proposal() {
    let mut store = Store::open_in_memory().unwrap();
    let mut note = adoption("s1", "/notes/Budget");
    note.title = Some(Title {
        text: "Budget".to_string(),
        source: TitleSource::Proposed,
    });
    store.adopt(&note).unwrap();
    assert!(
        store
            .set_title("s1", "Budget review", TitleSource::Proposed)
            .unwrap()
    );
}

#[test]
fn adopt_refuses_an_id_that_has_a_row() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    assert!(!store.adopt(&adoption("s1", "/notes/Budget")).unwrap());
    // The row is as it was: capturing, with no segments, notes or home.
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.state, SessionState::Capturing);
    assert_eq!(record.home_path, None);
    assert!(store.segments("s1").unwrap().is_empty());
    assert_eq!(store.notes("s1").unwrap(), "");
    assert_eq!(store.transcript_fingerprint("s1").unwrap(), None);
}

#[test]
fn an_adoption_from_session_md_takes_its_fields() {
    let mut doc = session_core::folder::SessionDoc::new("s1", None, &[segment(0, 0)]).unwrap();
    doc.title = Some("Budget".to_string());
    doc.asr_model = Some("parakeet".to_string());
    doc.found_speakers = Some(2);
    let note = session_core::Adoption::from_doc(
        &doc,
        "- agenda\n".to_string(),
        "/notes/Budget".to_string(),
        Some("abc123".to_string()),
    );
    assert_eq!(note.id, "s1");
    assert_eq!(note.created_at, t(0));
    assert_eq!(note.segments, vec![segment(0, 0)]);
    // A title with no recorded source counts as set by the user (R5).
    assert_eq!(note.title.unwrap().source, TitleSource::User);
    assert_eq!(note.asr_model.as_deref(), Some("parakeet"));
    assert_eq!(note.found_speakers, Some(2));
    assert_eq!(note.notes, "- agenda\n");
    assert_eq!(note.home_path, "/notes/Budget");
    assert_eq!(note.transcript_fingerprint.as_deref(), Some("abc123"));
    // `session.md` records no removal: the scan reads the markers.
    assert!(note.removed_audio.is_empty());
}

#[test]
fn forget_removes_the_row_and_its_checkpoints_and_leaves_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("Budget");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("session.md"), "---\n---\n").unwrap();
    let path = dir.path().join("longhand.db");
    let mut store = Store::open(&path).unwrap();
    store
        .adopt(&adoption("s1", home.to_str().unwrap()))
        .unwrap();
    store
        .put_checkpoint("s1", Stage::Transcribe, "w1", "{}")
        .unwrap();
    store.adopt(&adoption("s2", "/notes/Other")).unwrap();
    assert!(store.forget("s1").unwrap());
    assert!(store.session("s1").unwrap().is_none());
    assert!(home.join("session.md").exists(), "forget touches no folder");
    // The other note stays.
    assert!(store.session("s2").unwrap().is_some());
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let left: i64 = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM checkpoints WHERE session_id = 's1')
                  + (SELECT COUNT(*) FROM segments WHERE session_id = 's1')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn forget_refuses_a_note_that_is_not_saved_or_that_a_rename_moves() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("capturing", t(0), None).unwrap();
    assert!(!store.forget("capturing").unwrap());
    assert!(store.session("capturing").unwrap().is_some());

    store.adopt(&adoption("renamed", "/notes/A")).unwrap();
    store
        .set_rename_intent(
            "renamed",
            &session_core::RenameIntent {
                from: "/notes/A".into(),
                to: "/notes/B".into(),
                temp: None,
            },
        )
        .unwrap();
    assert!(!store.forget("renamed").unwrap());
    assert!(store.session("renamed").unwrap().is_some());

    assert!(!store.forget("nope").unwrap());
}

#[test]
fn the_transcript_fingerprint_is_kept_and_replaced() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    assert_eq!(store.transcript_fingerprint("s1").unwrap(), None);
    store.set_transcript_fingerprint("s1", Some("abc")).unwrap();
    assert_eq!(
        store.transcript_fingerprint("s1").unwrap().as_deref(),
        Some("abc")
    );
    store.set_transcript_fingerprint("s1", None).unwrap();
    assert_eq!(store.transcript_fingerprint("s1").unwrap(), None);
    assert!(matches!(
        store.set_transcript_fingerprint("nope", Some("abc")),
        Err(StoreError::NoSuchSession(_))
    ));
    assert!(matches!(
        store.transcript_fingerprint("nope"),
        Err(StoreError::NoSuchSession(_))
    ));
}

#[test]
fn clearing_recognition_results_removes_assemble_transcribe_and_diarize_only() {
    let mut store = Store::open_in_memory().unwrap();
    store.adopt(&adoption("s1", "/notes/Budget")).unwrap();
    store.adopt(&adoption("s2", "/notes/Other")).unwrap();
    for id in ["s1", "s2"] {
        for stage in [
            Stage::Assemble,
            Stage::Transcribe,
            Stage::Diarize,
            Stage::Summarize,
            Stage::Save,
        ] {
            store.put_checkpoint(id, stage, "k", "{}").unwrap();
        }
    }
    store.clear_recognition_results("s1").unwrap();
    for stage in [Stage::Assemble, Stage::Transcribe, Stage::Diarize] {
        assert!(
            store.checkpoints("s1", stage).unwrap().is_empty(),
            "{stage:?}"
        );
        assert_eq!(store.checkpoints("s2", stage).unwrap().len(), 1);
    }
    for stage in [Stage::Summarize, Stage::Save] {
        assert_eq!(
            store.checkpoints("s1", stage).unwrap().len(),
            1,
            "{stage:?}"
        );
    }
    assert!(matches!(
        store.clear_recognition_results("nope"),
        Err(StoreError::NoSuchSession(_))
    ));
}

#[test]
fn only_a_saved_unclaimed_session_with_a_home_folder_is_not_busy() {
    use session_core::jobs::busy;
    let d = saved_session();
    let record = d.store.lock().unwrap().session("s").unwrap().unwrap();
    assert!(!busy(&record, false));
    assert!(
        busy(&record, true),
        "a regeneration or a delete holds a claim"
    );
    let mut homeless = record.clone();
    homeless.home_path = None;
    assert!(busy(&homeless, false));
    let mut recording = record;
    recording.state = SessionState::Capturing;
    assert!(busy(&recording, false));
}

#[test]
fn recognition_snapshot_is_immutable_across_default_and_count_changes() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("snapshot", t(0), None).unwrap();
    let snapshot = serde_json::json!({"schema_version":1,"config":{"asr_model":"pinned"}});
    store
        .set_recognition_snapshot("snapshot", &snapshot)
        .unwrap();
    store
        .set_speaker_count("snapshot", Some(SpeakerCount::Exact(3)))
        .unwrap();
    assert_eq!(
        store.recognition_snapshot("snapshot").unwrap(),
        Some(snapshot)
    );
    assert!(
        store
            .set_recognition_snapshot("snapshot", &serde_json::json!({"different":true}))
            .is_err()
    );
}

#[test]
fn adoption_preserves_recognition_snapshot_without_queuing_work() {
    let mut store = Store::open_in_memory().unwrap();
    let mut note = adoption("portable", "/notes/portable");
    note.recognition_snapshot =
        Some(serde_json::json!({"schema_version":1,"artifacts":[{"sha256":"pinned"}]}));
    store.adopt(&note).unwrap();
    assert_eq!(
        store.recognition_snapshot("portable").unwrap(),
        note.recognition_snapshot
    );
    assert!(store.queue().unwrap().is_empty());
}

/// The Move journal (KTD9): one journal at a time, a Pending row for each
/// note in the old folder, and a note that the Move works on is not
/// forgotten.
#[test]
fn the_move_journal_holds_the_notes_of_the_old_folder() {
    use session_core::{MovePause, MoveStep};
    let d = saved_session();
    let old = d.home.parent().unwrap().to_path_buf();
    let new = old.parent().unwrap().join("new");
    let mut store = d.store.lock().unwrap();
    assert!(store.move_journal().unwrap().is_none());
    assert!(store.open_move(&old, &new).unwrap());
    assert!(!store.open_move(&new, &old).unwrap());
    let journal = store.move_journal().unwrap().unwrap();
    assert_eq!(
        (journal.old_folder, journal.new_folder),
        (old.clone(), new.clone())
    );
    assert_eq!((journal.cross_volume, journal.pause), (None, None));
    assert_eq!(
        store.move_notes().unwrap(),
        vec![("s".to_string(), MoveStep::Pending)]
    );
    assert_eq!(store.moving_notes().unwrap(), vec!["s".to_string()]);
    assert!(!store.add_move_note("s").unwrap());

    let cause = MovePause::DiskFull {
        folder: new.to_string_lossy().into_owned(),
    };
    store.set_move_pause(Some(&cause)).unwrap();
    assert_eq!(store.move_journal().unwrap().unwrap().pause, Some(cause));
    store.set_move_pause(None).unwrap();
    store.set_move_cross_volume(true).unwrap();
    let journal = store.move_journal().unwrap().unwrap();
    assert_eq!((journal.cross_volume, journal.pause), (Some(true), None));

    // A note that the Move works on is not forgotten.
    assert!(!store.forget("s").unwrap());
    let placed = new.join("2026-09-22 Budget review");
    let homed = MoveStep::Homed {
        source: Some(d.home.clone()),
        path: placed.clone(),
    };
    store.set_move_home("s", &placed, &homed).unwrap();
    assert_eq!(store.move_step("s").unwrap(), Some(homed));
    assert_eq!(
        store.session("s").unwrap().unwrap().home_path.as_deref(),
        placed.to_str()
    );
    store.set_move_step("s", &MoveStep::Done).unwrap();
    assert!(store.moving_notes().unwrap().is_empty());
    store.close_move().unwrap();
    assert!(store.move_journal().unwrap().is_none());
    assert!(store.move_notes().unwrap().is_empty());
    assert!(store.forget("s").unwrap());
}

#[test]
fn a_note_that_the_move_has_copied_is_not_deleted() {
    use session_core::MoveStep;
    let d = saved_session();
    let old = d.home.parent().unwrap().to_path_buf();
    {
        let mut store = d.store.lock().unwrap();
        store.open_move(&old, &old.join("new")).unwrap();
        let copied = MoveStep::Copied {
            source: d.home.clone(),
            staging: old.join("new").join(".longhand-move-s"),
            replaces: None,
        };
        store.set_move_step("s", &copied).unwrap();
    }
    let claims = Claims::default();
    let mut calls = Vec::new();
    let err =
        delete_session(&d.store, "s", &d.working, &claims, recording(&mut calls)).unwrap_err();
    assert!(
        matches!(err, DeleteError::Busy { state: "moving" }),
        "{err:?}"
    );
    assert!(calls.is_empty());
    // A Pending note can be deleted: its row leaves the journal.
    d.store
        .lock()
        .unwrap()
        .set_move_step("s", &MoveStep::Pending)
        .unwrap();
    delete_session(&d.store, "s", &d.working, &claims, recording(&mut calls)).unwrap();
    assert!(d.store.lock().unwrap().move_notes().unwrap().is_empty());
}

#[test]
fn a_save_into_the_old_folder_is_found_until_the_note_is_saved() {
    let mut store = Store::open_in_memory().unwrap();
    let old = Path::new("/notes/old");
    store.create_session("s", t(0), None).unwrap();
    store
        .put_checkpoint(
            "s",
            Stage::Save,
            "state",
            r#"{"step":"staged","staging":"/notes/old/.longhand-staging-s","copied":false}"#,
        )
        .unwrap();
    assert_eq!(store.saves_into(old).unwrap(), vec!["s".to_string()]);
    assert!(
        store
            .saves_into(Path::new("/notes/new"))
            .unwrap()
            .is_empty()
    );
    store.put_segment("s", &segment(0, 0)).unwrap();
    stop(&mut store, "s", 5);
    store.dequeue("run", t(6)).unwrap();
    store.apply("s", &Event::Saved).unwrap();
    assert!(store.saves_into(old).unwrap().is_empty());
}

// KTD4 of the note types plan: a save into a home folder in a type folder
// is a save into the storage folder.
#[test]
fn a_save_into_a_type_folder_of_the_old_folder_is_found() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s", t(0), None).unwrap();
    store
        .put_checkpoint(
            "s",
            Stage::Save,
            "state",
            r#"{"step":"done","path":"/notes/old/Pathfinder/2027-01-15 0800 Note"}"#,
        )
        .unwrap();
    assert_eq!(
        store.saves_into(Path::new("/notes/old")).unwrap(),
        vec!["s".to_string()]
    );
    assert!(store.saves_into(Path::new("/notes")).unwrap().is_empty());
}

/// R4 of the type prompt plan: a note records no prompt when it starts.
#[test]
fn a_new_note_records_no_prompt_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    let mut store = Store::open(&path).unwrap();
    let model = RecordingModel {
        asr_model: "m".into(),
        live_transcript: false,
    };
    store
        .start_session_with_recognition("new", t(0), None, &model, &CaptureChoice::default(), None)
        .unwrap();
    store.create_session("old", t(1), None).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let set: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE prompt_snapshot_id IS NOT NULL
               OR prompt_snapshot_name IS NOT NULL OR prompt_snapshot_text IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(set, 0);
}

// ---------------------------------------------------------------- notes-only notes (KTD2, KTD8)

#[test]
fn a_notes_only_note_is_a_saved_row_with_no_segments_that_lists_with_its_open_time() {
    use session_core::jobs::busy;
    let mut store = Store::open_in_memory().unwrap();
    store
        .create_notes_only("n1", t(0), Some("/notes/2026-09-22 1000"))
        .unwrap();
    let record = store.session("n1").unwrap().unwrap();
    assert_eq!(record.state, SessionState::Saved);
    assert_eq!(record.home_path.as_deref(), Some("/notes/2026-09-22 1000"));
    assert_eq!(record.started_at, Some(t(0)));
    assert!(store.segments("n1").unwrap().is_empty());
    assert_eq!(store.notes("n1").unwrap(), "");
    assert!(store.started_here("n1").unwrap());
    // It is not in the queue and has no job.
    assert!(store.queue().unwrap().is_empty());
    assert!(!busy(&record, false));

    let listed = store.sessions().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].start(&[]), t(0));
}

#[test]
fn a_notes_only_note_with_no_home_folder_yet_is_busy() {
    use session_core::jobs::busy;
    let mut store = Store::open_in_memory().unwrap();
    store.create_notes_only("n1", t(0), None).unwrap();
    let record = store.session("n1").unwrap().unwrap();
    assert_eq!(record.state, SessionState::Saved);
    assert!(busy(&record, false));
    store.set_home_path("n1", "/notes/a").unwrap();
    assert!(!busy(&store.session("n1").unwrap().unwrap(), false));
}

#[test]
fn record_into_a_notes_only_note_captures_and_keeps_its_open_time() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .create_notes_only("n1", t(0), Some("/notes/a"))
        .unwrap();
    let next = store
        .apply(
            "n1",
            &Event::UserRecord {
                has_segments: false,
            },
        )
        .unwrap();
    assert_eq!(next, SessionState::Capturing);
    // A note opened at 10:00 and recorded at 11:00 keeps 10:00 (R16).
    let mut later = segment(0, 0);
    later.wall_start = t(60);
    later.wall_end = t(61);
    store.put_segment("n1", &later).unwrap();
    let record = store.session("n1").unwrap().unwrap();
    assert_eq!(record.started_at, Some(t(0)));
    assert_eq!(record.start(&store.segments("n1").unwrap()), t(0));
    assert_eq!(record.home_path.as_deref(), Some("/notes/a"));

    // Record is refused once the note has segments: that is a Resume.
    store
        .apply(
            "n1",
            &Event::UserStop {
                at: t(62),
                retained_audio: true,
            },
        )
        .unwrap();
    assert!(matches!(
        store.apply("n1", &Event::UserRecord { has_segments: true }),
        Err(StoreError::Illegal(_))
    ));
}

// Covers AE12 (R16): a note that capture starts keeps the time it opened,
// so its folder name does not change when its first segment starts later.
#[test]
fn a_note_that_starts_to_record_keeps_the_time_it_opened() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .start_session("s1", t(0), None, &whisper(), &CaptureChoice::default())
        .unwrap();
    let mut first = segment(0, 0);
    first.wall_start = t(1);
    store.put_segment("s1", &first).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.started_at, Some(t(0)));
    assert_eq!(record.start(&store.segments("s1").unwrap()), t(0));
}

#[test]
fn a_recorded_note_starts_at_its_first_segment() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    let mut first = segment(0, 0);
    first.wall_start = t(2);
    store.put_segment("s1", &first).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.started_at, None);
    assert_eq!(record.start(&store.segments("s1").unwrap()), t(2));
    // With no segment yet, the start is the time the row was made.
    assert_eq!(record.start(&[]), t(0));
}

#[test]
fn only_a_note_that_this_computer_started_is_started_here() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("recorded", t(0), None).unwrap();
    store.create_notes_only("notes", t(1), None).unwrap();
    assert!(store.adopt(&adoption("remote", "/notes/remote")).unwrap());
    assert!(store.started_here("recorded").unwrap());
    assert!(store.started_here("notes").unwrap());
    assert!(!store.started_here("remote").unwrap());
    assert!(matches!(
        store.started_here("absent"),
        Err(StoreError::NoSuchSession(_))
    ));
}

#[test]
fn an_adopted_note_with_no_segments_keeps_its_start_time() {
    let mut store = Store::open_in_memory().unwrap();
    let mut note = adoption("remote", "/notes/remote");
    note.segments.clear();
    note.created_at = t(5);
    assert!(store.adopt(&note).unwrap());
    let record = store.session("remote").unwrap().unwrap();
    assert_eq!(record.started_at, Some(t(5)));
    // An adopted note with segments starts at its first segment, as before.
    assert!(store.adopt(&adoption("audio", "/notes/audio")).unwrap());
    assert_eq!(store.session("audio").unwrap().unwrap().started_at, None);
}

#[test]
fn the_last_written_fingerprints_update_on_each_write_and_read_back() {
    use session_core::jobs::WrittenFingerprints;
    let mut store = Store::open_in_memory().unwrap();
    store
        .create_notes_only("n1", t(0), Some("/notes/a"))
        .unwrap();
    assert_eq!(
        store.written_fingerprints("n1").unwrap(),
        WrittenFingerprints::default()
    );

    store
        .set_written_session_fingerprint("n1", Some("s-1"))
        .unwrap();
    store
        .set_written_notes_fingerprint("n1", Some("n-1"))
        .unwrap();
    assert_eq!(
        store.written_fingerprints("n1").unwrap(),
        WrittenFingerprints {
            session: Some("s-1".into()),
            notes: Some("n-1".into()),
        }
    );

    // Each later write replaces only its own fingerprint.
    store
        .set_written_notes_fingerprint("n1", Some("n-2"))
        .unwrap();
    assert_eq!(
        store.written_fingerprints("n1").unwrap(),
        WrittenFingerprints {
            session: Some("s-1".into()),
            notes: Some("n-2".into()),
        }
    );
    store
        .set_written_session_fingerprint("n1", Some("s-2"))
        .unwrap();
    store.set_written_notes_fingerprint("n1", None).unwrap();
    assert_eq!(
        store.written_fingerprints("n1").unwrap(),
        WrittenFingerprints {
            session: Some("s-2".into()),
            notes: None,
        }
    );

    assert!(matches!(
        store.written_fingerprints("absent"),
        Err(StoreError::NoSuchSession(_))
    ));
    assert!(matches!(
        store.set_written_session_fingerprint("absent", Some("x")),
        Err(StoreError::NoSuchSession(_))
    ));
    assert!(matches!(
        store.set_written_notes_fingerprint("absent", Some("x")),
        Err(StoreError::NoSuchSession(_))
    ));
}

#[test]
fn a_type_move_is_kept_until_it_is_cleared() {
    use session_core::TypeMove;
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.create_session("s2", t(1), None).unwrap();
    assert_eq!(store.type_move("s1").unwrap(), None);
    let first = TypeMove {
        to: Some("One on One".into()),
        from: None,
        changed_at: t(5),
        taken_over_at: None,
        remove_empty_from: false,
        field_written: false,
    };
    store.set_type_move("s1", &first).unwrap();
    assert_eq!(store.type_move("s1").unwrap(), Some(first.clone()));
    // A second record for the session replaces the first.
    let second = TypeMove {
        to: None,
        from: Some("Pathfinder".into()),
        changed_at: t(6),
        taken_over_at: Some(t(7)),
        remove_empty_from: true,
        field_written: true,
    };
    store.set_type_move("s1", &second).unwrap();
    store.set_type_move("s2", &first).unwrap();
    assert_eq!(
        store.type_moves().unwrap(),
        [
            ("s1".to_string(), second.clone()),
            ("s2".to_string(), first)
        ]
    );
    store.clear_type_move("s2").unwrap();
    assert_eq!(store.type_move("s2").unwrap(), None);
    assert!(store.set_type_move("nobody", &second).is_err());
}

#[test]
fn a_bulk_edit_is_kept_in_order_until_it_is_removed() {
    use session_core::BulkEdit;
    let store = Store::open_in_memory().unwrap();
    assert!(store.bulk_edits().unwrap().is_empty());
    let rename = BulkEdit::RenameType {
        from: "1 on 1".into(),
        to: "One on One".into(),
    };
    let delete = BulkEdit::DeleteType {
        name: "Pathfinder".into(),
    };
    let first = store.add_bulk_edit(&rename, &[]).unwrap();
    let second = store.add_bulk_edit(&delete, &[]).unwrap();
    let open = store.bulk_edits().unwrap();
    assert_eq!(open.len(), 2);
    assert_eq!(
        (open[0].id, &open[0].edit, open[0].shared_saved),
        (first, &rename, false)
    );
    assert_eq!((open[1].id, &open[1].edit), (second, &delete));
    store.set_bulk_edit_shared_saved(first).unwrap();
    assert!(store.bulk_edits().unwrap()[0].shared_saved);
    store.remove_bulk_edit(first).unwrap();
    let open = store.bulk_edits().unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].edit, delete);
}

#[test]
fn a_bulk_edit_keeps_its_notes_after_a_reopen() {
    use session_core::BulkEdit;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("longhand.db");
    let delete = BulkEdit::DeleteTag {
        name: "follow-up".into(),
    };
    let id = {
        let store = Store::open(&db).unwrap();
        let id = store
            .add_bulk_edit(&delete, &["a".into(), "b".into()])
            .unwrap();
        assert_eq!(store.bulk_edits().unwrap()[0].notes, ["a", "b"]);
        store.set_bulk_edit_notes(id, &["b".into()]).unwrap();
        id
    };
    let store = Store::open(&db).unwrap();
    let open = store.bulk_edits().unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        (open[0].id, &open[0].edit, open[0].notes.as_slice()),
        (id, &delete, ["b".to_string()].as_slice())
    );
}

#[test]
fn a_tag_edit_is_kept_with_its_texts() {
    use session_core::BulkEdit;
    let store = Store::open_in_memory().unwrap();
    let rename = BulkEdit::RenameTag {
        from: "Customer::Mayo".into(),
        to: "Customer::Mayo Clinic".into(),
    };
    let delete = BulkEdit::DeleteTag {
        name: "follow-up".into(),
    };
    store.add_bulk_edit(&rename, &[]).unwrap();
    store.add_bulk_edit(&delete, &[]).unwrap();
    let open = store.bulk_edits().unwrap();
    assert_eq!((&open[0].edit, &open[1].edit), (&rename, &delete));
    assert!(open.iter().all(|o| o.edit.is_tag_edit()));
    assert!(
        !BulkEdit::DeleteType {
            name: "Pathfinder".into()
        }
        .is_tag_edit()
    );
}

#[test]
fn the_computer_id_stays_the_same_and_differs_between_databases() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    let id = Store::open(&path).unwrap().computer_id().unwrap();
    assert_eq!(id.len(), 32);
    assert_eq!(Store::open(&path).unwrap().computer_id().unwrap(), id);
    assert_ne!(Store::open_in_memory().unwrap().computer_id().unwrap(), id);
}
