//! The shared settings file (U1): a read never writes, and a save patches
//! only its own fields, so versions and computers do not reset each
//! other's values (R15, R17, KTD2, KTD3).

use std::fs;
use std::path::Path;

use models::recognition::{ProcessingMode, SpeakerCount, SpeakerSource};
use serde_json::{Value, json};
use shared_settings::file::{
    Field, FileRead, SCHEMA_VERSION, SETTINGS_FILE, SaveError, SharedValue, Unavailable,
    read_settings, save_settings,
};

fn folder() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join(".longhand")).unwrap();
    root
}

fn write_raw(root: &Path, value: &Value) {
    fs::write(
        root.join(".longhand").join(SETTINGS_FILE),
        serde_json::to_vec_pretty(value).unwrap(),
    )
    .unwrap();
}

fn on_disk(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(root.join(".longhand").join(SETTINGS_FILE)).unwrap()).unwrap()
}

fn parsed(root: &Path) -> shared_settings::file::SettingsFile {
    match read_settings(root) {
        FileRead::Parsed(file) => file,
        other => panic!("expected a parsed file, got {other:?}"),
    }
}

fn full() -> Value {
    json!({
        "schema_version": 1,
        "active_summary_prompt": "prompt-a",
        "default_speaker_count": {"kind": "exact", "count": 3},
        "asr_model": models::DEFAULT_ASR_MODEL,
        "live_transcript": true,
        "speaker_source": "sortformer",
        "speaker_processing_mode": "during_recording",
        "recognition_language": "de",
        "check_updates_automatically": false
    })
}

#[test]
fn the_longhand_folder_matches_the_library_scan() {
    assert_eq!(
        shared_settings::file::LONGHAND_FOLDER,
        library::scan::LONGHAND_FOLDER
    );
}

#[test]
fn a_full_file_parses_every_field_as_known() {
    let root = folder();
    write_raw(root.path(), &full());
    let fields = parsed(root.path()).fields;
    assert_eq!(
        fields.active_summary_prompt,
        Field::Known("prompt-a".into())
    );
    assert_eq!(
        fields.default_speaker_count,
        Field::Known(Some(SpeakerCount::Exact { count: 3 }))
    );
    assert_eq!(
        fields.asr_model,
        Field::Known(models::DEFAULT_ASR_MODEL.to_string())
    );
    assert_eq!(fields.live_transcript, Field::Known(true));
    assert_eq!(
        fields.speaker_source,
        Field::Known(SpeakerSource::Sortformer)
    );
    assert_eq!(
        fields.speaker_processing_mode,
        Field::Known(ProcessingMode::DuringRecording)
    );
    assert_eq!(fields.recognition_language, Field::Known(Some("de".into())));
    assert_eq!(fields.check_updates_automatically, Field::Known(false));
    assert_eq!(fields.schema_version, Some(1));
    assert!(!fields.newer_schema());
}

#[test]
fn a_missing_field_is_absent_and_null_is_a_known_empty_value() {
    let root = folder();
    write_raw(
        root.path(),
        &json!({"default_speaker_count": null, "recognition_language": null}),
    );
    let fields = parsed(root.path()).fields;
    assert_eq!(fields.default_speaker_count, Field::Known(None));
    assert_eq!(fields.recognition_language, Field::Known(None));
    assert_eq!(fields.live_transcript, Field::Absent);
    assert_eq!(fields.asr_model, Field::Absent);
    assert_eq!(fields.schema_version, None);
}

#[test]
fn a_patch_of_live_transcript_keeps_every_other_field() {
    // AE12: B has A's active prompt, and B turns off live transcript.
    let root = folder();
    write_raw(root.path(), &full());
    save_settings(root.path(), &[SharedValue::LiveTranscript(false)]).unwrap();
    let mut expected = full();
    expected["live_transcript"] = json!(false);
    assert_eq!(on_disk(root.path()), expected);
}

#[test]
fn a_newer_version_and_an_unknown_field_survive_a_patch() {
    // AE11: the file comes from a newer build.
    let root = folder();
    let mut newer = full();
    newer["schema_version"] = json!(9);
    newer["future_field"] = json!({"nested": [1, 2, 3]});
    write_raw(root.path(), &newer);
    assert!(parsed(root.path()).fields.newer_schema());
    save_settings(root.path(), &[SharedValue::CheckUpdatesAutomatically(true)]).unwrap();
    let after = on_disk(root.path());
    assert_eq!(after["schema_version"], json!(9));
    assert_eq!(after["future_field"], json!({"nested": [1, 2, 3]}));
    assert_eq!(after["check_updates_automatically"], json!(true));
}

#[test]
fn a_patch_raises_an_older_version_to_this_build() {
    let root = folder();
    let mut older = full();
    older["schema_version"] = json!(0);
    write_raw(root.path(), &older);
    save_settings(root.path(), &[SharedValue::LiveTranscript(true)]).unwrap();
    assert_eq!(
        on_disk(root.path())["schema_version"],
        json!(SCHEMA_VERSION)
    );
}

#[test]
fn a_model_that_is_not_in_the_catalog_is_unknown_and_survives_a_patch() {
    let root = folder();
    let mut value = full();
    value["asr_model"] = json!("model-from-the-future");
    write_raw(root.path(), &value);
    assert_eq!(
        parsed(root.path()).fields.asr_model,
        Field::Unknown(json!("model-from-the-future"))
    );
    save_settings(root.path(), &[SharedValue::RecognitionLanguage(None)]).unwrap();
    let after = on_disk(root.path());
    assert_eq!(after["asr_model"], json!("model-from-the-future"));
    assert_eq!(after["recognition_language"], Value::Null);
}

#[test]
fn a_catalog_id_that_is_not_a_transcription_model_is_unknown() {
    let root = folder();
    let not_asr = models::CATALOG
        .iter()
        .find(|m| m.kind != models::ModelKind::Asr)
        .expect("the catalog has a model that is not a transcription model");
    write_raw(root.path(), &json!({"asr_model": not_asr.id}));
    assert_eq!(
        parsed(root.path()).fields.asr_model,
        Field::Unknown(json!(not_asr.id))
    );
}

#[test]
fn a_newer_speaker_source_is_unknown_not_the_default() {
    let root = folder();
    write_raw(
        root.path(),
        &json!({"speaker_source": "hologram", "speaker_processing_mode": "telepathy"}),
    );
    let fields = parsed(root.path()).fields;
    assert_eq!(fields.speaker_source, Field::Unknown(json!("hologram")));
    assert_eq!(
        fields.speaker_processing_mode,
        Field::Unknown(json!("telepathy"))
    );
}

#[test]
fn a_value_of_the_wrong_type_is_unknown_and_other_fields_still_parse() {
    let root = folder();
    write_raw(
        root.path(),
        &json!({
            "live_transcript": "yes",
            "default_speaker_count": {"kind": "exact", "count": 0},
            "active_summary_prompt": 7,
            "check_updates_automatically": true
        }),
    );
    let fields = parsed(root.path()).fields;
    assert_eq!(fields.live_transcript, Field::Unknown(json!("yes")));
    assert_eq!(
        fields.default_speaker_count,
        Field::Unknown(json!({"kind": "exact", "count": 0}))
    );
    assert_eq!(fields.active_summary_prompt, Field::Unknown(json!(7)));
    assert_eq!(fields.check_updates_automatically, Field::Known(true));
}

#[test]
fn a_read_writes_nothing() {
    let root = folder();
    write_raw(root.path(), &full());
    let path = root.path().join(".longhand").join(SETTINGS_FILE);
    let bytes = fs::read(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    for _ in 0..3 {
        parsed(root.path());
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    let names: Vec<_> = fs::read_dir(root.path().join(".longhand"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![SETTINGS_FILE]);
}

#[test]
fn a_truncated_file_is_not_available_and_a_patch_refuses_to_write() {
    let root = folder();
    let text = serde_json::to_string_pretty(&full()).unwrap();
    let half = &text[..text.len() / 2];
    let path = root.path().join(".longhand").join(SETTINGS_FILE);
    fs::write(&path, half).unwrap();
    assert!(matches!(
        read_settings(root.path()),
        FileRead::NotAvailable(Unavailable::DoesNotParse(_))
    ));
    let saved = save_settings(root.path(), &[SharedValue::LiveTranscript(false)]);
    assert!(matches!(
        saved,
        Err(SaveError::NotAvailable(Unavailable::DoesNotParse(_)))
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), half);
}

#[test]
fn a_file_that_is_not_an_object_is_not_available() {
    let root = folder();
    write_raw(root.path(), &json!([1, 2]));
    assert!(matches!(
        read_settings(root.path()),
        FileRead::NotAvailable(Unavailable::DoesNotParse(_))
    ));
}

#[test]
fn a_conflict_copy_of_the_settings_file_has_no_effect() {
    let root = folder();
    write_raw(root.path(), &full());
    let mut other = full();
    other["live_transcript"] = json!(false);
    other["active_summary_prompt"] = json!("prompt-b");
    fs::write(
        root.path().join(".longhand").join("settings 2.json"),
        serde_json::to_vec(&other).unwrap(),
    )
    .unwrap();
    let fields = parsed(root.path()).fields;
    assert_eq!(fields.live_transcript, Field::Known(true));
    assert_eq!(
        fields.active_summary_prompt,
        Field::Known("prompt-a".into())
    );
    save_settings(root.path(), &[SharedValue::CheckUpdatesAutomatically(true)]).unwrap();
    let copy: Value = serde_json::from_slice(
        &fs::read(root.path().join(".longhand").join("settings 2.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(copy, other);
}

#[test]
fn a_folder_with_no_settings_file_reads_as_missing_and_a_patch_refuses() {
    let root = tempfile::tempdir().unwrap();
    assert!(matches!(read_settings(root.path()), FileRead::Missing));
    fs::create_dir_all(root.path().join(".longhand")).unwrap();
    assert!(matches!(read_settings(root.path()), FileRead::Missing));
    assert!(matches!(
        save_settings(root.path(), &[SharedValue::LiveTranscript(false)]),
        Err(SaveError::Missing)
    ));
    assert!(!root.path().join(".longhand").join(SETTINGS_FILE).exists());
}

#[test]
fn a_folder_that_does_not_exist_is_not_available() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("gone");
    assert!(matches!(
        read_settings(&root),
        FileRead::NotAvailable(Unavailable::FolderMissing)
    ));
    assert!(matches!(
        save_settings(&root, &[SharedValue::LiveTranscript(false)]),
        Err(SaveError::NotAvailable(Unavailable::FolderMissing))
    ));
    assert!(!root.exists());
}

#[test]
fn every_value_of_a_patch_is_written_under_its_own_key() {
    let root = folder();
    write_raw(root.path(), &json!({"schema_version": 1}));
    save_settings(
        root.path(),
        &[
            SharedValue::ActiveSummaryPrompt("p".into()),
            SharedValue::DefaultSpeakerCount(Some(SpeakerCount::Range { min: 2, max: 4 })),
            SharedValue::AsrModel(models::DEFAULT_ASR_MODEL.into()),
            SharedValue::LiveTranscript(false),
            SharedValue::SpeakerSource(SpeakerSource::Off),
            SharedValue::SpeakerProcessingMode(ProcessingMode::AfterStop),
            SharedValue::RecognitionLanguage(Some("fr".into())),
            SharedValue::CheckUpdatesAutomatically(false),
        ],
    )
    .unwrap();
    assert_eq!(
        on_disk(root.path()),
        json!({
            "schema_version": SCHEMA_VERSION,
            "active_summary_prompt": "p",
            "default_speaker_count": {"kind": "range", "min": 2, "max": 4},
            "asr_model": models::DEFAULT_ASR_MODEL,
            "live_transcript": false,
            "speaker_source": "off",
            "speaker_processing_mode": "after_stop",
            "recognition_language": "fr",
            "check_updates_automatically": false
        })
    );
}

#[test]
fn the_defaults_parse_as_known_values() {
    let fields =
        shared_settings::file::SharedFields::from_raw(&shared_settings::file::default_settings());
    assert_eq!(
        fields.active_summary_prompt,
        Field::Known("topic-summary".into())
    );
    assert_eq!(fields.default_speaker_count, Field::Known(None));
    assert_eq!(
        fields.asr_model,
        Field::Known(models::DEFAULT_ASR_MODEL.to_string())
    );
    assert_eq!(fields.live_transcript, Field::Known(true));
    assert_eq!(
        fields.speaker_source,
        Field::Known(SpeakerSource::default())
    );
    assert_eq!(
        fields.speaker_processing_mode,
        Field::Known(ProcessingMode::default())
    );
    assert_eq!(fields.recognition_language, Field::Known(None));
    assert_eq!(fields.check_updates_automatically, Field::Known(true));
    assert_eq!(fields.schema_version, Some(SCHEMA_VERSION));
}
