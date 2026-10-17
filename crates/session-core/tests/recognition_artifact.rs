use serde_json::json;
use session_core::{Store, recognition::*};

fn fingerprints() -> Fingerprints {
    Fingerprints {
        execution: digest(b"execution"),
        asr: digest(b"asr"),
        speakers: digest(b"speakers"),
        audio: digest(b"audio"),
        preprocessing: digest(b"preprocessing"),
    }
}
fn header() -> ArtifactHeader {
    ArtifactHeader {
        version: ARTIFACT_VERSION,
        generation: "g".into(),
        snapshot: json!({"schema_version":1}),
        fingerprints: fingerprints(),
        transcript: digest(b"transcript"),
        limitations: vec![],
        coverage: vec![SampleRange {
            start: 0,
            end: 16000,
        }],
    }
}
fn block(revision: u64, final_output: bool) -> OutputBlock {
    let range = SampleRange {
        start: 0,
        end: 16000,
    };
    OutputBlock {
        key: "window-0".into(),
        scope: SpeakerScope {
            generation: "g".into(),
            epoch: 1,
        },
        revision,
        coverage: Coverage {
            received: Some(range),
            computed: Some(range),
            provisional: (!final_output).then_some(range),
            committed: final_output.then_some(range),
        },
        rows: vec![],
        speakers: vec![],
    }
}
fn store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store
        .create_session(
            "s",
            chrono::DateTime::parse_from_rfc3339("2026-09-27T10:00:00-05:00").unwrap(),
            None,
        )
        .unwrap();
    store
}
#[test]
fn fingerprints_separate_asr_reuse_from_labels_and_reject_external_edits() {
    let old = header();
    let mut next = fingerprints();
    assert_eq!(old.reuse(&next, b"transcript"), Reuse::Complete);
    next.speakers = digest(b"different speaker artifact");
    next.execution = digest(b"new execution");
    assert_eq!(old.reuse(&next, b"transcript"), Reuse::TimedAsr);
    assert_eq!(old.reuse(&next, b"external edit"), Reuse::TranscribeAgain);
    for field in ["asr", "audio", "preprocessing"] {
        let mut next = fingerprints();
        match field {
            "asr" => next.asr = digest(b"new"),
            "audio" => next.audio = digest(b"new"),
            _ => next.preprocessing = digest(b"new"),
        }
        assert_eq!(old.reuse(&next, b"transcript"), Reuse::TranscribeAgain);
    }
}
#[test]
fn committed_output_and_revision_are_one_durable_operation() {
    let mut store = store();
    assert!(store.commit_recognition("s", &block(2, true)).unwrap());
    assert!(!store.commit_recognition("s", &block(2, true)).unwrap());
    assert!(!store.commit_recognition("s", &block(1, false)).unwrap());
    assert!(store.commit_recognition("s", &block(3, false)).is_err());
    assert_eq!(
        store.recognition_block("s", "g", "window-0").unwrap(),
        Some(block(2, true))
    );
}
#[test]
fn received_or_computed_audio_is_not_committed_and_cursor_requires_replay() {
    let mut progress = block(1, false);
    assert!(progress.coverage.committed.is_none());
    progress.coverage.committed = Some(SampleRange {
        start: 0,
        end: 16001,
    });
    assert!(progress.validate().is_err());
    let mut store = store();
    let first = store
        .restart_recognition_epoch("s", "g", 32000, 96000)
        .unwrap();
    let second = store
        .restart_recognition_epoch("s", "g", 32000, 96000)
        .unwrap();
    assert_eq!(first.replay_from, 32000);
    assert_eq!(second.replay_from, 32000);
    assert_ne!(first.scope, second.scope);
    assert!(!first.native_state_restored);
}
#[test]
fn portable_artifact_checks_checksum_version_and_bounded_records() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("recognition.jsonl");
    let reference = write_artifact(&path, &header(), [block(1, true)]).unwrap();
    let mut restored = vec![];
    assert_eq!(
        read_artifact(&path, &reference, |b| {
            restored.push(b);
            Ok(())
        })
        .unwrap(),
        header()
    );
    assert_eq!(restored, [block(1, true)]);
    std::fs::write(&path, b"external change").unwrap();
    assert!(read_artifact(&path, &reference, |_| Ok(())).is_err());
    let mut unsupported = header();
    unsupported.version += 1;
    assert!(write_artifact(&path, &unsupported, []).is_err());
    let mut huge = block(1, true);
    huge.key = "x".repeat(MAX_BLOCK_BYTES + 1);
    assert!(write_artifact(&path, &header(), [huge]).is_err());
}
#[test]
fn previous_schema_migrates_without_changing_old_results() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("jobs.db");
    {
        let mut connection = rusqlite::Connection::open(&path).unwrap();
        session_core::schema::migrations()
            .to_version(&mut connection, 9)
            .unwrap();
        connection.execute("INSERT INTO sessions(id,created_at,state) VALUES ('s','2026-09-27T10:00:00-05:00','{\"state\":\"saved\"}')", []).unwrap();
        connection.execute("INSERT INTO checkpoints(session_id,stage,key,payload) VALUES ('s','transcribe','legacy','{}')", []).unwrap();
    }
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .checkpoints("s", session_core::Stage::Transcribe)
            .unwrap()
            .len(),
        1
    );
    store.commit_recognition("s", &block(1, true)).unwrap();
}

#[test]
fn same_revision_conflicts_and_bad_tail_has_no_import_side_effects() {
    let mut store = store();
    store.commit_recognition("s", &block(2, false)).unwrap();
    assert!(store.commit_recognition("s", &block(2, true)).is_err());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("recognition.jsonl");
    let mut reference = write_artifact(&path, &header(), [block(1, true)]).unwrap();
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"invalid\":true}\n")
        .unwrap();
    reference.sha256 = file_digest(&path).unwrap();
    let mut called = false;
    assert!(
        read_artifact(&path, &reference, |_| {
            called = true;
            Ok(())
        })
        .is_err()
    );
    assert!(!called);
}

#[test]
fn imported_coverage_holes_and_duplicate_identities_are_rejected_before_visiting() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("recognition.jsonl");
    for blocks in [vec![], vec![block(1, true), block(1, true)]] {
        let reference = write_artifact(&path, &header(), blocks).unwrap();
        let mut visited = false;
        assert!(
            read_artifact(&path, &reference, |_| {
                visited = true;
                Ok(())
            })
            .is_err()
        );
        assert!(!visited);
    }
}

#[test]
fn committed_output_survives_reopen_and_interrupted_replay_uses_a_new_scope() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("jobs.db");
    let old;
    {
        let mut store = Store::open(&path).unwrap();
        store
            .create_session(
                "s",
                chrono::DateTime::parse_from_rfc3339("2026-09-27T10:00:00-05:00").unwrap(),
                None,
            )
            .unwrap();
        old = store.restart_recognition_epoch("s", "g", 0, 8000).unwrap();
        store.commit_recognition("s", &block(1, true)).unwrap();
        // The native output exists, but no task completion cursor is written.
    }
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.recognition_block("s", "g", "window-0").unwrap(),
        Some(block(1, true))
    );
    let new = store.restart_recognition_epoch("s", "g", 0, 8000).unwrap();
    assert_eq!(new.replay_from, 0);
    assert_ne!(new.scope, old.scope);
    assert!(!new.native_state_restored);
}

#[test]
fn recognition_batch_is_atomic_and_idempotent() {
    let mut store = store();
    let first = block(2, true);
    let mut second = first.clone();
    second.key = "window-1".into();
    assert_eq!(
        store
            .commit_recognition_batch("s", &[first.clone(), second.clone()])
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .commit_recognition_batch("s", &[first.clone(), second.clone()])
            .unwrap(),
        0
    );
    let mut third = first.clone();
    third.key = "window-2".into();
    let mut conflict = second.clone();
    conflict.scope.epoch += 1;
    assert!(
        store
            .commit_recognition_batch("s", &[third.clone(), conflict])
            .is_err()
    );
    assert!(
        store
            .recognition_block("s", "g", &third.key)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.recognition_block("s", "g", &second.key).unwrap(),
        Some(second)
    );
    let oversized = vec![first; MAX_COMMIT_BLOCKS + 1];
    assert!(store.commit_recognition_batch("s", &oversized).is_err());
}

#[test]
fn recognition_batch_size_limit_writes_no_partial_output() {
    let mut store = store();
    let mut first = block(1, true);
    first.key = "a".repeat(MAX_COMMIT_BYTES / 2);
    let mut second = first.clone();
    second.key.replace_range(0..1, "b");
    first.validate().unwrap();
    second.validate().unwrap();
    assert!(
        store
            .commit_recognition_batch("s", &[first.clone(), second])
            .is_err()
    );
    assert!(
        store
            .recognition_block("s", "g", &first.key)
            .unwrap()
            .is_none()
    );
}

fn keyed(generation: &str, key: &str) -> OutputBlock {
    let mut block = block(1, true);
    block.key = key.into();
    block.scope.generation = generation.into();
    block
}

fn keys_of(store: &Store, id: &str, generation: &str) -> Vec<String> {
    let mut keys = vec![];
    store
        .visit_recognition(id, generation, |block| {
            keys.push(block.key);
            Ok(())
        })
        .unwrap();
    keys
}

/// The next epoch number. A generation with no epoch row starts at 1.
fn next_epoch(store: &mut Store, id: &str, generation: &str) -> u64 {
    store
        .restart_recognition_epoch(id, generation, 0, 0)
        .unwrap()
        .scope
        .epoch
}

#[test]
fn a_deleted_generation_keeps_the_rows_of_other_generations_and_sessions() {
    let mut store = store();
    store
        .create_session(
            "t",
            chrono::DateTime::parse_from_rfc3339("2026-09-27T11:00:00-05:00").unwrap(),
            None,
        )
        .unwrap();
    for (id, generation) in [("s", "g"), ("s", "h"), ("t", "g")] {
        store
            .commit_recognition(id, &keyed(generation, "window-0"))
            .unwrap();
        next_epoch(&mut store, id, generation);
        next_epoch(&mut store, id, generation);
    }

    store.delete_recognition_generation("s", "g").unwrap();

    assert!(keys_of(&store, "s", "g").is_empty());
    assert_eq!(next_epoch(&mut store, "s", "g"), 1, "the epoch row is gone");
    assert_eq!(keys_of(&store, "s", "h"), ["window-0"]);
    assert_eq!(next_epoch(&mut store, "s", "h"), 3);
    assert_eq!(keys_of(&store, "t", "g"), ["window-0"]);
    assert_eq!(next_epoch(&mut store, "t", "g"), 3);
}

#[test]
fn a_new_index_removes_the_prefix_that_it_replaces() {
    let mut store = store();
    for key in ["result/1/row/0", "result/2/row/0", "other/0"] {
        store.commit_recognition("s", &keyed("g", key)).unwrap();
    }
    store
        .replace_recognition_index(
            "s",
            session_core::Stage::Transcribe,
            "index",
            "\"new\"",
            Some(("g", "result/1/")),
        )
        .unwrap();
    assert_eq!(keys_of(&store, "s", "g"), ["other/0", "result/2/row/0"]);
    assert_eq!(
        store
            .checkpoint_value("s", session_core::Stage::Transcribe, "index")
            .unwrap()
            .as_deref(),
        Some("\"new\"")
    );
}

#[test]
fn an_index_put_and_the_deletion_of_the_old_prefix_are_one_transaction() {
    // A fault between the two writes leaves the old index and its blocks.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    let mut store = Store::open(&path).unwrap();
    store
        .create_session(
            "s",
            chrono::DateTime::parse_from_rfc3339("2026-09-27T10:00:00-05:00").unwrap(),
            None,
        )
        .unwrap();
    for key in ["result/1/row/0", "result/2/row/0"] {
        store.commit_recognition("s", &keyed("g", key)).unwrap();
    }
    store
        .put_checkpoint("s", session_core::Stage::Transcribe, "index", "\"old\"")
        .unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER no_delete BEFORE DELETE ON recognition_output
             BEGIN SELECT RAISE(ABORT, 'the rows cannot be deleted'); END;",
        )
        .unwrap();

    assert!(
        store
            .replace_recognition_index(
                "s",
                session_core::Stage::Transcribe,
                "index",
                "\"new\"",
                Some(("g", "result/1/")),
            )
            .is_err()
    );
    assert_eq!(
        store
            .checkpoint_value("s", session_core::Stage::Transcribe, "index")
            .unwrap()
            .as_deref(),
        Some("\"old\"")
    );
    assert_eq!(
        keys_of(&store, "s", "g"),
        ["result/1/row/0", "result/2/row/0"]
    );
}

#[test]
fn live_blocks_go_only_for_epochs_of_the_current_generation_with_a_final_copy() {
    let mut store = store();
    store
        .put_checkpoint(
            "s",
            session_core::Stage::Transcribe,
            GENERATION_KEY,
            "\"g\"",
        )
        .unwrap();
    for key in [
        "nemo/00000000000000000001/word/0",
        "nemo/00000000000000000001/turn/0",
        "native-final/00000000000000000001/row/0",
        // A live worker still writes epoch 2: it has no final copy.
        "nemo/00000000000000000002/word/0",
    ] {
        store.commit_recognition("s", &keyed("g", key)).unwrap();
    }
    for key in [
        "nemo/00000000000000000001/word/0",
        "native-final/00000000000000000001/row/0",
    ] {
        store.commit_recognition("s", &keyed("h", key)).unwrap();
    }

    store.delete_finished_live_blocks("s").unwrap();

    assert_eq!(
        keys_of(&store, "s", "g"),
        [
            "native-final/00000000000000000001/row/0",
            "nemo/00000000000000000002/word/0",
        ]
    );
    assert_eq!(
        keys_of(&store, "s", "h").len(),
        2,
        "another generation stays"
    );
}
