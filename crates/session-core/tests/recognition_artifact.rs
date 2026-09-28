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
