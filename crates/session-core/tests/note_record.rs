use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, TimeDelta};
use session_core::folder::{
    FolderError, PendingType, ResultFlag, SegmentRecord, SummaryMeta, fingerprint,
};
use session_core::note_record::{
    LAYOUT, NOTE_RECORD_FILE, NoteRecord, NoteRecordRead, session_id_of,
};
use session_core::recognition::ArtifactReference;
use session_core::{SpeakerCount, Title, TitleSource};

fn wall(minutes: i64) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-22T09:00:00-05:00").unwrap() + TimeDelta::minutes(minutes)
}

/// A record with every field set.
fn full_record() -> NoteRecord {
    NoteRecord {
        layout: LAYOUT,
        session_id: "01j8z".into(),
        started: wall(0),
        written_title: Some(Title {
            text: "Budget review: Q3 \"plan\"".into(),
            source: TitleSource::Proposed,
        }),
        pending_type: Some(PendingType {
            to: Some("One on One".into()),
            from: None,
            by: "computer-b".into(),
            at: wall(5),
        }),
        speaker_count: Some(SpeakerCount::Range { min: 2, max: 4 }),
        asr_model: Some("parakeet-tdt-0.6b-v3".into()),
        recognition_snapshot: Some(serde_json::json!({"model": "m", "options": {"z": 1, "a": 2}})),
        recognition_artifact: Some(ArtifactReference {
            version: 1,
            filename: "recognition.jsonl".into(),
            sha256: fingerprint(b"artifact"),
        }),
        recognition_audio: Some(BTreeMap::from([(
            "audio/segment-01.opus".to_string(),
            fingerprint(b"audio"),
        )])),
        found_speakers: Some(3),
        transcript_fingerprint: Some(fingerprint(b"transcript")),
        summary_fingerprint: Some(fingerprint(b"summary")),
        result: Some(ResultFlag::NotSaved),
        segments: vec![
            SegmentRecord {
                index: 0,
                wall_start: wall(0),
                wall_end: wall(1),
                offset_ms: 0,
                duration_ms: 60_000,
                audio: "audio/segment-01.opus".into(),
                channels: 2,
                ends_in_pause: true,
            },
            SegmentRecord {
                index: 1,
                wall_start: wall(2),
                wall_end: wall(3),
                offset_ms: 120_000,
                duration_ms: 60_000,
                audio: "audio/segment-02.opus".into(),
                channels: 1,
                ends_in_pause: false,
            },
        ],
        summary: Some(SummaryMeta::Complete {
            endpoint_host: "api.example.com".into(),
            model: "gpt-x".into(),
            prompt_name: Some("Meeting".into()),
            prompt_id: Some("prompt-1".into()),
        }),
    }
}

/// The keys of the lines at `indent` spaces, in file order.
fn keys_at(text: &str, indent: usize) -> Vec<String> {
    let prefix = format!("{}\"", " ".repeat(indent));
    text.lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .filter_map(|rest| rest.split_once("\":").map(|(key, _)| key.to_string()))
        .collect()
}

#[test]
fn the_file_name_has_no_leading_dot_and_the_layout_is_one() {
    assert_eq!(NOTE_RECORD_FILE, "longhand.json");
    assert_eq!(LAYOUT, 1);
}

#[test]
fn a_record_with_every_field_round_trips_and_two_writes_give_the_same_bytes() {
    let record = full_record();
    let text = record.render();
    assert_eq!(
        NoteRecord::parse(&text).unwrap(),
        NoteRecordRead::Current(Box::new(record.clone()))
    );
    assert_eq!(
        record.clone().render().into_bytes(),
        text.clone().into_bytes()
    );
    assert!(!text.contains('\r'), "{text}");
    assert!(text.ends_with("}\n"), "{text}");

    let top = keys_at(&text, 2);
    assert_eq!(
        top,
        [
            "asr_model",
            "found_speakers",
            "layout",
            "pending_type",
            "recognition_artifact",
            "recognition_audio",
            "recognition_snapshot",
            "result",
            "segments",
            "session_id",
            "speaker_count",
            "started",
            "summary",
            "summary_fingerprint",
            "transcript_fingerprint",
            "written_title",
        ]
    );
    let nested = keys_at(&text, 4);
    let summary_keys = [
        "endpoint_host",
        "model",
        "prompt_id",
        "prompt_name",
        "status",
    ];
    assert!(
        nested
            .windows(5)
            .any(|w| w == summary_keys.map(String::from)),
        "{text}"
    );
    assert!(text.contains("\"layout\": 1"), "{text}");
}

#[test]
fn a_record_with_only_the_required_fields_round_trips() {
    let record = NoteRecord::new("01j8z", wall(0));
    let text = record.render();
    assert_eq!(
        keys_at(&text, 2),
        ["layout", "segments", "session_id", "started"]
    );
    assert_eq!(
        NoteRecord::parse(&text).unwrap(),
        NoteRecordRead::Current(Box::new(record))
    );
}

#[test]
fn a_record_with_a_newer_layout_parses_as_newer_not_as_an_error() {
    let text = "{\n  \"layout\": 2,\n  \"something_new\": [1, 2]\n}\n";
    assert_eq!(
        NoteRecord::parse(text).unwrap(),
        NoteRecordRead::Newer { layout: 2 }
    );
}

#[test]
fn a_record_that_does_not_parse_gives_the_line_and_the_column() {
    let text = "{\n  \"layout\": 1,\n  \"session_id\": \n}\n";
    match NoteRecord::parse(text) {
        Err(FolderError::At {
            file, line, column, ..
        }) => {
            assert_eq!(file, "longhand.json");
            assert_eq!((line, column), (4, 1));
        }
        other => panic!("expected a parse error, got {other:?}"),
    }
}

#[test]
fn a_record_with_a_wrong_field_type_gives_the_line() {
    let text = "{\n  \"layout\": 1,\n  \"session_id\": \"01j8z\",\n  \"started\": 5,\n  \"segments\": []\n}\n";
    match NoteRecord::parse(text) {
        Err(FolderError::At { file, line, .. }) => {
            assert_eq!(file, "longhand.json");
            assert_eq!(line, 4);
        }
        other => panic!("expected a parse error, got {other:?}"),
    }
}

#[test]
fn a_record_whose_session_id_is_not_a_plain_note_id_does_not_parse() {
    // The id becomes a folder name under the working root, so a path, a
    // parent reference, an empty id, uppercase (file systems that ignore
    // case) or a long id is refused.
    let long = "a".repeat(65);
    for id in [
        "/tmp/x",
        "../..",
        "",
        "a/b",
        "a\\b",
        "C:",
        "..",
        "01J8Z",
        long.as_str(),
    ] {
        let text = NoteRecord::new(id, wall(0)).render();
        assert!(NoteRecord::parse(&text).is_err(), "{id:?} parsed");
        assert_eq!(session_id_of(&text), None, "{id:?}");
    }
    for id in [
        "s1",
        "note-budget-review",
        "6f1c0e1a-3b7d-4c2e-9a51-0d2f4b8e7c10",
        "a_b",
    ] {
        let text = NoteRecord::new(id, wall(0)).render();
        assert!(NoteRecord::parse(&text).is_ok(), "{id:?} did not parse");
    }
}

#[test]
fn a_newer_record_with_a_bad_session_id_gives_no_id() {
    let text = "{\n  \"layout\": 9,\n  \"session_id\": \"../..\"\n}\n";
    assert_eq!(session_id_of(text), None);
}
