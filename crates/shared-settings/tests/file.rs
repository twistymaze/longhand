//! The shared settings file (U1): a read never writes, and a save patches
//! only its own fields, so versions and computers do not reset each
//! other's values (R15, R17, KTD2, KTD3).

use std::fs;
use std::path::Path;

use models::recognition::{ProcessingMode, SpeakerCount, SpeakerSource};
use serde_json::{Value, json};
use shared_settings::file::{
    AUDIO_REMOVAL_MAX_DAYS, AudioRemoval, DEFAULT_AUDIO_REMOVAL_DAYS, Field, FileRead,
    SCHEMA_VERSION, SETTINGS_FILE, SaveError, SharedValue, Unavailable, read_settings,
    save_settings,
};
use shared_settings::view::load;

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
        "check_updates_automatically": false,
        "audio_removal": 90
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
    assert_eq!(
        fields.audio_removal,
        Field::Known(AudioRemoval::AfterDays(90))
    );
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
            SharedValue::AudioRemoval(AudioRemoval::Off),
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
            "check_updates_automatically": false,
            "audio_removal": "off"
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
    assert_eq!(
        fields.audio_removal,
        Field::Known(AudioRemoval::AfterDays(DEFAULT_AUDIO_REMOVAL_DAYS))
    );
    assert_eq!(DEFAULT_AUDIO_REMOVAL_DAYS, 30);
    assert_eq!(fields.schema_version, Some(SCHEMA_VERSION));
}

// The audio removal setting (U1 of the plan for removing old audio: R1,
// R2, AE10, KTD3).

/// The number of days after which the shared view of `root` removes audio.
/// `None` is "no automatic removal". This reads the folder and writes
/// nothing.
fn delay_days(root: &Path) -> Option<u32> {
    load(root, None).view.audio_removal_days()
}

#[test]
fn a_number_of_days_and_off_parse_as_known_and_give_the_delay() {
    let root = folder();
    write_raw(root.path(), &json!({"audio_removal": 45}));
    assert_eq!(
        parsed(root.path()).fields.audio_removal,
        Field::Known(AudioRemoval::AfterDays(45))
    );
    assert_eq!(delay_days(root.path()), Some(45));

    write_raw(root.path(), &json!({"audio_removal": "off"}));
    assert_eq!(
        parsed(root.path()).fields.audio_removal,
        Field::Known(AudioRemoval::Off)
    );
    assert_eq!(delay_days(root.path()), None);
}

#[test]
fn the_first_and_the_last_day_of_the_range_parse() {
    let root = folder();
    for days in [1, AUDIO_REMOVAL_MAX_DAYS] {
        write_raw(root.path(), &json!({"audio_removal": days}));
        assert_eq!(
            parsed(root.path()).fields.audio_removal,
            Field::Known(AudioRemoval::AfterDays(days))
        );
        assert_eq!(delay_days(root.path()), Some(days));
    }
    assert_eq!(AUDIO_REMOVAL_MAX_DAYS, 3650);
}

#[test]
fn a_value_outside_the_range_or_of_another_form_is_unknown_and_gives_no_delay() {
    let root = folder();
    for value in [
        json!(0),
        json!(-1),
        json!(3651),
        json!(1.5),
        json!("30"),
        json!("never"),
        json!("Off"),
        json!(null),
        json!(true),
        json!({"days": 30}),
    ] {
        write_raw(root.path(), &json!({"audio_removal": value}));
        assert_eq!(
            parsed(root.path()).fields.audio_removal,
            Field::Unknown(value.clone()),
            "{value}"
        );
        assert_eq!(delay_days(root.path()), None, "{value}");
    }
}

#[test]
fn an_unknown_audio_removal_value_shows_in_the_unknown_values() {
    let root = folder();
    write_raw(root.path(), &json!({"audio_removal": 0}));
    let fields = parsed(root.path()).fields;
    assert_eq!(
        fields.unknown_values(),
        vec![(shared_settings::file::KEY_AUDIO_REMOVAL, &json!(0))]
    );
}

#[test]
fn a_readable_file_with_no_audio_removal_key_gives_the_default_delay() {
    // KTD3: an absent key in a readable file means the default.
    let root = folder();
    write_raw(root.path(), &json!({"schema_version": 1}));
    assert_eq!(parsed(root.path()).fields.audio_removal, Field::Absent);
    assert_eq!(delay_days(root.path()), Some(DEFAULT_AUDIO_REMOVAL_DAYS));
}

#[test]
fn a_file_from_a_newer_build_still_gives_its_delay() {
    let root = folder();
    write_raw(
        root.path(),
        &json!({"schema_version": 9, "audio_removal": 12, "future_field": 1}),
    );
    assert_eq!(delay_days(root.path()), Some(12));
}

#[test]
fn a_settings_file_that_does_not_parse_gives_no_delay_and_the_read_writes_nothing() {
    // AE10: a half-synced file.
    let root = folder();
    let mut value = full();
    value["audio_removal"] = json!(5);
    let text = serde_json::to_string_pretty(&value).unwrap();
    let half = &text[..text.len() / 2];
    let path = root.path().join(".longhand").join(SETTINGS_FILE);
    fs::write(&path, half).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    for _ in 0..3 {
        assert_eq!(delay_days(root.path()), None);
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), half);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    let names: Vec<_> = fs::read_dir(root.path().join(".longhand"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![SETTINGS_FILE]);
}

#[test]
fn a_cache_with_a_delay_gives_no_delay_while_the_file_does_not_parse() {
    // R2: the cache is only for display. No audio goes while the shared
    // settings cannot be read.
    let root = folder();
    write_raw(root.path(), &json!({"audio_removal": 7}));
    let cache = load(root.path(), None)
        .cache
        .expect("a first load gives a cache");
    fs::write(
        root.path().join(".longhand").join(SETTINGS_FILE),
        "{\"audio",
    )
    .unwrap();
    let view = load(root.path(), Some(&cache)).view;
    assert_eq!(
        view.fields.audio_removal,
        Field::Known(AudioRemoval::AfterDays(7))
    );
    assert_eq!(view.audio_removal_days(), None);
}

#[test]
fn a_folder_with_no_settings_file_or_no_folder_gives_no_delay() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(delay_days(root.path()), None);
    assert!(!root.path().join(".longhand").exists());
    let gone = root.path().join("gone");
    assert_eq!(delay_days(&gone), None);
    assert!(!gone.exists());
}

#[test]
fn a_conflict_copy_with_another_delay_has_no_effect() {
    let root = folder();
    write_raw(root.path(), &json!({"audio_removal": 90}));
    fs::write(
        root.path().join(".longhand").join("settings 2.json"),
        serde_json::to_vec(&json!({"audio_removal": 1})).unwrap(),
    )
    .unwrap();
    assert_eq!(delay_days(root.path()), Some(90));
}

#[test]
fn a_save_of_another_field_keeps_the_audio_removal_value() {
    let root = folder();
    for value in [json!("off"), json!(12), json!(0)] {
        write_raw(
            root.path(),
            &json!({"schema_version": 1, "audio_removal": value}),
        );
        save_settings(root.path(), &[SharedValue::LiveTranscript(false)]).unwrap();
        assert_eq!(on_disk(root.path())["audio_removal"], value);
    }
}

#[test]
fn a_save_of_audio_removal_keeps_every_other_field_and_unknown_fields() {
    let root = folder();
    let mut value = full();
    value["future_field"] = json!({"nested": true});
    value["asr_model"] = json!("model-from-the-future");
    write_raw(root.path(), &value);
    save_settings(
        root.path(),
        &[SharedValue::AudioRemoval(AudioRemoval::AfterDays(3650))],
    )
    .unwrap();
    let mut expected = value;
    expected["audio_removal"] = json!(3650);
    assert_eq!(on_disk(root.path()), expected);
    save_settings(root.path(), &[SharedValue::AudioRemoval(AudioRemoval::Off)]).unwrap();
    expected["audio_removal"] = json!("off");
    assert_eq!(on_disk(root.path()), expected);
}

// ------------------------------------------------------------ note types

#[test]
fn the_note_types_and_the_default_type_parse_and_null_is_no_type() {
    // R2, R7, KTD9.
    let root = folder();
    write_raw(
        root.path(),
        &json!({"note_types": ["Pathfinder", "One on One"], "default_note_type": "Pathfinder"}),
    );
    let fields = parsed(root.path()).fields;
    assert_eq!(
        fields.note_types,
        Field::Known(vec!["Pathfinder".to_string(), "One on One".to_string()])
    );
    assert_eq!(
        fields.default_note_type,
        Field::Known(Some("Pathfinder".to_string()))
    );

    write_raw(root.path(), &json!({"default_note_type": null}));
    let fields = parsed(root.path()).fields;
    assert_eq!(fields.note_types, Field::Absent);
    assert_eq!(fields.default_note_type, Field::Known(None));
}

#[test]
fn a_note_type_list_of_the_wrong_form_is_unknown() {
    let root = folder();
    write_raw(
        root.path(),
        &json!({"note_types": "Pathfinder", "default_note_type": 3}),
    );
    let fields = parsed(root.path()).fields;
    assert_eq!(fields.note_types, Field::Unknown(json!("Pathfinder")));
    assert_eq!(fields.default_note_type, Field::Unknown(json!(3)));
    let keys: Vec<_> = fields
        .unknown_values()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(keys, ["note_types", "default_note_type"]);
}

#[test]
fn a_save_of_another_field_keeps_the_note_types_and_the_default_type() {
    // The prevention rule of the shared settings rules: a save of another
    // field keeps each new field.
    let root = folder();
    write_raw(
        root.path(),
        &json!({
            "schema_version": 1,
            "note_types": ["Pathfinder", "One on One"],
            "default_note_type": "One on One"
        }),
    );
    save_settings(root.path(), &[SharedValue::LiveTranscript(false)]).unwrap();
    let raw = on_disk(root.path());
    assert_eq!(raw["note_types"], json!(["Pathfinder", "One on One"]));
    assert_eq!(raw["default_note_type"], json!("One on One"));
    assert_eq!(raw["live_transcript"], json!(false));
}

#[test]
fn a_save_of_the_note_types_keeps_every_other_field() {
    let root = folder();
    let mut before = full();
    before["from_a_newer_build"] = json!({"x": 1});
    write_raw(root.path(), &before);
    save_settings(
        root.path(),
        &[
            SharedValue::NoteTypes(vec!["Pathfinder".into()]),
            SharedValue::DefaultNoteType(None),
        ],
    )
    .unwrap();
    let mut expected = before;
    expected["note_types"] = json!(["Pathfinder"]);
    expected["default_note_type"] = Value::Null;
    assert_eq!(on_disk(root.path()), expected);
}

/// Each name and its bytes in the `.longhand` folder of `root`.
fn longhand_files(root: &Path) -> Vec<(std::ffi::OsString, Vec<u8>)> {
    let mut files: Vec<_> = fs::read_dir(root.join(".longhand"))
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (e.file_name(), fs::read(e.path()).unwrap())
        })
        .collect();
    files.sort();
    files
}

/// The effective type names of a read of `root` by a computer with no
/// cache.
fn effective_names(root: &Path) -> Vec<String> {
    let loaded = load(root, None);
    let shared = loaded
        .view
        .fields
        .note_types
        .known()
        .cloned()
        .unwrap_or_default();
    shared_settings::note_types::effective_types(&shared, &[])
        .into_iter()
        .map(|t| t.name)
        .collect()
}

#[test]
fn a_read_of_the_note_types_writes_nothing_for_a_file_that_does_not_parse() {
    let root = folder();
    let path = root.path().join(".longhand").join(SETTINGS_FILE);
    fs::write(&path, r#"{"note_types": ["Pathfinder""#).unwrap();
    let before = longhand_files(root.path());
    assert!(effective_names(root.path()).is_empty());
    assert_eq!(longhand_files(root.path()), before);
}

#[test]
fn a_read_of_the_note_types_writes_nothing_for_a_newer_schema_version() {
    let root = folder();
    write_raw(
        root.path(),
        &json!({"schema_version": SCHEMA_VERSION + 1, "note_types": ["Pathfinder"]}),
    );
    let before = longhand_files(root.path());
    assert_eq!(effective_names(root.path()), ["Pathfinder"]);
    assert_eq!(longhand_files(root.path()), before);
}

#[test]
fn a_conflict_copy_with_other_note_types_has_no_effect_and_the_read_writes_nothing() {
    let root = folder();
    write_raw(root.path(), &json!({"note_types": ["Pathfinder"]}));
    fs::write(
        root.path().join(".longhand").join("settings 2.json"),
        r#"{"note_types": ["Customer Calls"], "default_note_type": "Customer Calls"}"#,
    )
    .unwrap();
    let before = longhand_files(root.path());
    assert_eq!(effective_names(root.path()), ["Pathfinder"]);
    assert_eq!(longhand_files(root.path()), before);
}

#[test]
fn the_type_prompts_parse_as_a_map_and_a_save_of_another_field_keeps_them() {
    // R2, KTD1, and the prevention rule of the shared settings rules.
    let root = folder();
    write_raw(
        root.path(),
        &json!({
            "schema_version": 1,
            "note_types": ["Pathfinder"],
            "note_type_prompts": {"General": "standup-id", "Pathfinder": "recap-id"}
        }),
    );
    let expected: std::collections::BTreeMap<String, String> = [
        ("General".to_string(), "standup-id".to_string()),
        ("Pathfinder".to_string(), "recap-id".to_string()),
    ]
    .into();
    assert_eq!(
        parsed(root.path()).fields.note_type_prompts,
        Field::Known(expected)
    );
    save_settings(root.path(), &[SharedValue::LiveTranscript(false)]).unwrap();
    assert_eq!(
        on_disk(root.path())["note_type_prompts"],
        json!({"General": "standup-id", "Pathfinder": "recap-id"})
    );
}

#[test]
fn a_save_of_the_type_prompts_writes_the_map_and_keeps_every_other_field() {
    let root = folder();
    let mut before = full();
    before["from_a_newer_build"] = json!({"x": 1});
    write_raw(root.path(), &before);
    save_settings(
        root.path(),
        &[SharedValue::NoteTypePrompts(
            [("Pathfinder".to_string(), "recap-id".to_string())].into(),
        )],
    )
    .unwrap();
    let mut expected = before;
    expected["note_type_prompts"] = json!({"Pathfinder": "recap-id"});
    assert_eq!(on_disk(root.path()), expected);
}

#[test]
fn a_type_prompts_value_that_is_not_an_object_of_strings_is_unknown_and_stays() {
    // KTD1: the value stays in the file and gives no choices.
    let root = folder();
    write_raw(
        root.path(),
        &json!({"schema_version": 1, "note_type_prompts": {"Pathfinder": 3}}),
    );
    let fields = parsed(root.path()).fields;
    assert_eq!(
        fields.note_type_prompts,
        Field::Unknown(json!({"Pathfinder": 3}))
    );
    assert!(fields.note_type_prompts.known().is_none());
    let keys: Vec<_> = fields
        .unknown_values()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(keys, ["note_type_prompts"]);
    save_settings(root.path(), &[SharedValue::LiveTranscript(false)]).unwrap();
    assert_eq!(
        on_disk(root.path())["note_type_prompts"],
        json!({"Pathfinder": 3})
    );
}

#[test]
fn an_absent_type_prompts_field_is_absent() {
    // R1: no map is "Default" for every type.
    let root = folder();
    write_raw(root.path(), &full());
    assert_eq!(parsed(root.path()).fields.note_type_prompts, Field::Absent);
}

// Show recording widget and New Note records are shared settings (U7 of
// the safe agent edits plan: R14, R15).

#[test]
fn a_save_of_another_field_keeps_the_widget_and_new_note_fields() {
    let root = folder();
    let mut before = full();
    before["show_recording_widget"] = json!(false);
    before["new_note_records"] = json!(false);
    write_raw(root.path(), &before);
    save_settings(root.path(), &[SharedValue::LiveTranscript(false)]).unwrap();
    let after = on_disk(root.path());
    assert_eq!(after["show_recording_widget"], json!(false));
    assert_eq!(after["new_note_records"], json!(false));
}

#[test]
fn the_widget_and_new_note_fields_parse_and_save_under_their_own_keys() {
    let root = folder();
    write_raw(
        root.path(),
        &json!({"schema_version": 1, "show_recording_widget": false, "new_note_records": "yes"}),
    );
    let fields = parsed(root.path()).fields;
    assert_eq!(fields.show_recording_widget, Field::Known(false));
    assert_eq!(fields.new_note_records, Field::Unknown(json!("yes")));
    save_settings(
        root.path(),
        &[
            SharedValue::ShowRecordingWidget(true),
            SharedValue::NewNoteRecords(false),
        ],
    )
    .unwrap();
    let after = on_disk(root.path());
    assert_eq!(after["show_recording_widget"], json!(true));
    assert_eq!(after["new_note_records"], json!(false));
}

#[test]
fn a_file_without_the_widget_and_new_note_keys_has_them_absent() {
    let root = folder();
    write_raw(root.path(), &full());
    let fields = parsed(root.path()).fields;
    assert_eq!(fields.show_recording_widget, Field::Absent);
    assert_eq!(fields.new_note_records, Field::Absent);
    let defaults =
        shared_settings::file::SharedFields::from_raw(&shared_settings::file::default_settings());
    assert_eq!(defaults.show_recording_widget, Field::Known(true));
    assert_eq!(defaults.new_note_records, Field::Known(true));
}
