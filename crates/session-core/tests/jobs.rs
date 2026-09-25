use chrono::{DateTime, FixedOffset, TimeDelta};
use session_core::jobs::Liveness;
use session_core::{
    Event, InterruptReason, RecordingModel, SessionState, SpeakerCount, Stage, Store, StoreError,
    Title, TitleSource,
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
    }
}

#[test]
fn the_shortcut_queues_the_open_session_and_starts_a_new_one() {
    let mut store = Store::open_in_memory().unwrap();
    store.create_session("s1", t(0), None).unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    assert_eq!(
        store
            .start_session("s2", t(5), None, &parakeet_live())
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
        .start_session("s2", t(5), None, &parakeet_live())
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
    assert!(claims.delete("s").is_none());
    assert!(claims.delete("other").is_some());
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
        .start_session("s1", t(0), None, &parakeet_live())
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
        .start_session("s1", t(0), None, &parakeet_live())
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
        .start_session("s1", t(0), None, &parakeet_live())
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
        .start_session("s1", t(0), None, &parakeet_live())
        .unwrap();
    store.put_segment("s1", &segment(0, 0)).unwrap();
    assert_eq!(
        store.start_session("s2", t(5), None, &whisper()).unwrap(),
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
