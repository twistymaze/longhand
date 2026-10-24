//! The `longhand-notes` skill and the app agree (R31, KTD13 of the safe
//! agent edits plan).
//!
//! The real writers make a storage folder in a temporary folder, and the
//! tests run the skill scripts in `plugins/longhand/skills/longhand-notes/`
//! on it. A change to a writer that adds a record or a setting that
//! `check.py` does not know fails these tests.
//!
//! The scripts need Python. The tests use `uv run --no-project python`, or
//! `python3` when uv is not installed. With neither, a test prints why it
//! does not run and passes, except when `CI` is set: then it fails.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, FixedOffset};
use library::scan::write_marker;
use models::recognition::{ProcessingMode, SpeakerCount, SpeakerSource};
use session_core::folder::{
    self, PendingType, ResultFlag, SegmentRecord, Summary, SummaryMeta, render_notes,
};
use session_core::jobs::{Title, TitleSource};
use session_core::note_folder::SESSION_FILE;
use session_core::note_record::{LAYOUT, NOTE_RECORD_FILE, NoteRecord};
use session_core::recognition::ArtifactReference;
use session_save::home_write::write_atomic;
use session_save::naming::folder_name;
use session_save::note_files::{new_session_md, write_record};
use shared_settings::file::{
    AudioRemoval, Field, FileRead, SCHEMA_VERSION, SharedFields, SharedValue, default_settings,
    read_settings, save_settings, settings_path,
};
use shared_settings::prompts::{
    prompts_folder, render_prompt_file, render_prompt_file_with_title, scan_prompts,
};
use summarize::prompts::{Prompt, built_ins};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scripts() -> PathBuf {
    repo().join("plugins/longhand/skills/longhand-notes/scripts")
}

/// The command that runs Python, or `None` when the computer has none.
fn python() -> Option<Vec<&'static str>> {
    let works = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .output()
            .is_ok_and(|out| out.status.success())
    };
    if works("uv", &["run", "--no-project", "python", "--version"]) {
        return Some(vec!["uv", "run", "--no-project", "python"]);
    }
    if works("python3", &["--version"]) {
        return Some(vec!["python3"]);
    }
    None
}

/// `python()`, or `None` after a printed reason. In CI a missing Python
/// fails the test.
fn python_or_skip() -> Option<Vec<&'static str>> {
    let found = python();
    if found.is_none() {
        assert!(
            std::env::var_os("CI").is_none(),
            "the skill drift test needs uv or python3 in CI"
        );
        eprintln!("skill_drift: skipped, because neither uv nor python3 is installed");
    }
    found
}

/// The result of one script run.
struct Run {
    code: i32,
    out: String,
}

/// Run `script` with `args` in the storage folder `root`. Snapshots go to
/// `tmp`, outside the storage folder.
fn run(python: &[&str], script: &str, args: &[&str], root: &Path, tmp: &Path) -> Run {
    let output = Command::new(python[0])
        .args(&python[1..])
        .arg(scripts().join(script))
        .args(args)
        .current_dir(root)
        .env("TMPDIR", tmp)
        .env("TEMP", tmp)
        .env("TMP", tmp)
        .env("PYTHONIOENCODING", "utf-8")
        .output()
        .unwrap();
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Run {
        code: output.status.code().unwrap_or(-1),
        out,
    }
}

fn at(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

/// The folder name of the note that the tests edit.
const EDITED: &str = "2026-10-01 0900 Drift check";

/// The records of a note with a value in every field. The struct literal
/// names each field, so a new field fails to compile here until the test
/// gives it a value, and then `check.py` sees its key.
fn full_record(dir: &Path, pending_type: Option<PendingType>) -> NoteRecord {
    let started = at("2026-10-01T09:00:00-05:00");
    NoteRecord {
        layout: LAYOUT,
        note_id: format!("note-{}", dir.file_name().unwrap().to_string_lossy()),
        started,
        written_title: Some(Title {
            text: "Drift check".to_string(),
            source: TitleSource::Proposed,
        }),
        pending_type,
        speaker_count: Some(folder::SpeakerCount::Range { min: 1, max: 3 }),
        asr_model: Some("parakeet-unified-en-0.6b-q80".to_string()),
        recognition_snapshot: Some(serde_json::json!({ "asr_model": "parakeet" })),
        recognition_artifact: Some(ArtifactReference {
            version: 1,
            filename: "recognition.jsonl".to_string(),
            sha256: "0".repeat(64),
        }),
        recognition_audio: Some(BTreeMap::from([(
            "audio/segment-01.opus".to_string(),
            "1".repeat(64),
        )])),
        found_speakers: Some(2),
        transcript_fingerprint: Some(folder::fingerprint(b"transcript")),
        summary_fingerprint: Some(folder::fingerprint(b"summary")),
        // `none` is not busy. `pending` is busy, and a check of an edit
        // would report it.
        result: Some(ResultFlag::NoRecording),
        segments: vec![SegmentRecord {
            index: 0,
            wall_start: started,
            wall_end: started + chrono::TimeDelta::minutes(1),
            offset_ms: 0,
            duration_ms: 60_000,
            audio: "audio/segment-01.opus".to_string(),
            channels: 1,
            ends_in_pause: true,
        }],
        summary: Some(SummaryMeta::Complete {
            endpoint_host: "llm.example.com".to_string(),
            model: "example-model".to_string(),
            prompt_name: Some("Topic summary".to_string()),
            prompt_id: Some("topic-summary".to_string()),
        }),
        // The skill learns the `import` field in U9 of the audio import
        // plan.
        import: None,
    }
}

/// The parsed field of each shared setting. The match has no `_` arm, so a
/// new setting fails to compile here. Give it its field, add the field to the
/// pattern in `every_setting_writes_a_known_value_for_each_shared_setting`,
/// and add a value for it to `every_setting()`.
fn field_of(value: &SharedValue) -> &'static str {
    match value {
        SharedValue::ActiveSummaryPrompt(_) => "active_summary_prompt",
        SharedValue::DefaultSpeakerCount(_) => "default_speaker_count",
        SharedValue::AsrModel(_) => "asr_model",
        SharedValue::LiveTranscript(_) => "live_transcript",
        SharedValue::SpeakerSource(_) => "speaker_source",
        SharedValue::SpeakerProcessingMode(_) => "speaker_processing_mode",
        SharedValue::RecognitionLanguage(_) => "recognition_language",
        SharedValue::CheckUpdatesAutomatically(_) => "check_updates_automatically",
        SharedValue::AudioRemoval(_) => "audio_removal",
        SharedValue::NoteTypes(_) => "note_types",
        SharedValue::DefaultNoteType(_) => "default_note_type",
        SharedValue::NoteTypePrompts(_) => "note_type_prompts",
        SharedValue::ShowRecordingWidget(_) => "show_recording_widget",
        SharedValue::NewNoteRecords(_) => "new_note_records",
    }
}

/// A value for every shared setting.
/// `every_setting_writes_a_known_value_for_each_shared_setting` checks
/// that the list is complete.
fn every_setting() -> Vec<SharedValue> {
    vec![
        SharedValue::ActiveSummaryPrompt("topic-summary".to_string()),
        SharedValue::DefaultSpeakerCount(Some(SpeakerCount::Exact { count: 2 })),
        SharedValue::AsrModel(models::DEFAULT_ASR_MODEL.to_string()),
        SharedValue::LiveTranscript(true),
        SharedValue::SpeakerSource(SpeakerSource::Pyannote),
        SharedValue::SpeakerProcessingMode(ProcessingMode::AfterStop),
        SharedValue::RecognitionLanguage(Some("en".to_string())),
        SharedValue::CheckUpdatesAutomatically(true),
        SharedValue::AudioRemoval(AudioRemoval::AfterDays(30)),
        SharedValue::NoteTypes(vec!["Meetings".to_string()]),
        SharedValue::DefaultNoteType(Some("Meetings".to_string())),
        SharedValue::NoteTypePrompts(BTreeMap::from([(
            "Meetings".to_string(),
            "play-by-play".to_string(),
        )])),
        SharedValue::ShowRecordingWidget(true),
        SharedValue::NewNoteRecords(true),
    ]
}

/// Fail when `field` does not hold a value that this build knows. Record
/// `name` as checked.
fn assert_known<T: std::fmt::Debug>(
    checked: &mut BTreeSet<&'static str>,
    name: &'static str,
    field: &Field<T>,
) {
    checked.insert(name);
    assert!(
        matches!(field, Field::Known(_)),
        "every_setting() writes no known value for {name}: {field:?}"
    );
}

/// The real save writes `every_setting()` into an empty settings file, and
/// the app's parser reads it back. The pattern has no `..`, so a new shared
/// setting fails to compile here until the test names its field. Each field
/// must hold a known value, so the test fails until `every_setting()`
/// writes a value for the new setting.
#[test]
fn every_setting_writes_a_known_value_for_each_shared_setting() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let path = settings_path(root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    // The file starts empty, so each value comes from `every_setting()`.
    fs::write(&path, b"{}").unwrap();
    save_settings(root, &every_setting()).unwrap();
    let FileRead::Parsed(file) = read_settings(root) else {
        panic!("the app cannot read the settings file that the save wrote");
    };
    let SharedFields {
        schema_version,
        active_summary_prompt,
        default_speaker_count,
        asr_model,
        live_transcript,
        speaker_source,
        speaker_processing_mode,
        recognition_language,
        check_updates_automatically,
        audio_removal,
        note_types,
        default_note_type,
        note_type_prompts,
        show_recording_widget,
        new_note_records,
    } = file.fields;
    assert_eq!(schema_version, Some(SCHEMA_VERSION));
    let mut checked = BTreeSet::new();
    assert_known(
        &mut checked,
        "active_summary_prompt",
        &active_summary_prompt,
    );
    assert_known(
        &mut checked,
        "default_speaker_count",
        &default_speaker_count,
    );
    assert_known(&mut checked, "asr_model", &asr_model);
    assert_known(&mut checked, "live_transcript", &live_transcript);
    assert_known(&mut checked, "speaker_source", &speaker_source);
    assert_known(
        &mut checked,
        "speaker_processing_mode",
        &speaker_processing_mode,
    );
    assert_known(&mut checked, "recognition_language", &recognition_language);
    assert_known(
        &mut checked,
        "check_updates_automatically",
        &check_updates_automatically,
    );
    assert_known(&mut checked, "audio_removal", &audio_removal);
    assert_known(&mut checked, "note_types", &note_types);
    assert_known(&mut checked, "default_note_type", &default_note_type);
    assert_known(&mut checked, "note_type_prompts", &note_type_prompts);
    assert_known(
        &mut checked,
        "show_recording_widget",
        &show_recording_widget,
    );
    assert_known(&mut checked, "new_note_records", &new_note_records);
    // Each value names one parsed field, and each parsed field has one value.
    let settings = every_setting();
    let written: BTreeSet<&str> = settings.iter().map(field_of).collect();
    assert_eq!(
        written.len(),
        settings.len(),
        "every_setting() repeats a setting"
    );
    assert_eq!(written, checked);
}

/// A storage folder that the real writers made: a note with every record,
/// a busy note in a type folder, a prompt with no id and every shared
/// setting.
fn storage_folder() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    assert!(write_marker(root));

    let started = at("2026-10-01T09:00:00-05:00");
    let dir = root.join(folder_name(started, Some("Drift check")));
    assert_eq!(dir.file_name().unwrap(), EDITED);
    fs::create_dir_all(&dir).unwrap();
    write_atomic(&dir, "notes.md", render_notes("- A note\n").as_bytes()).unwrap();
    let summary = Summary {
        meta: SummaryMeta::Complete {
            endpoint_host: "llm.example.com".to_string(),
            model: "example-model".to_string(),
            prompt_name: None,
            prompt_id: None,
        },
        body: "## Topics\n\n- One topic.".to_string(),
    }
    .render_body();
    write_atomic(&dir, "summary.md", summary.as_bytes()).unwrap();
    let tags = vec!["Finance".to_string()];
    write_atomic(
        &dir,
        SESSION_FILE,
        new_session_md(Some("Drift check"), &tags).as_bytes(),
    )
    .unwrap();
    write_record(&dir, &full_record(&dir, None)).unwrap();

    // A busy note: its move to another type waits.
    let busy = root
        .join("Meetings")
        .join(folder_name(started, Some("Busy note")));
    fs::create_dir_all(&busy).unwrap();
    write_atomic(
        &busy,
        SESSION_FILE,
        new_session_md(Some("Busy note"), &[]).as_bytes(),
    )
    .unwrap();
    let pending = PendingType {
        to: None,
        from: Some("Meetings".to_string()),
        by: "computer-a".to_string(),
        at: started,
    };
    write_record(&busy, &full_record(&busy, Some(pending))).unwrap();

    let prompts = prompts_folder(root);
    fs::create_dir_all(&prompts).unwrap();
    write_atomic(&prompts, "Plain.md", b"Write three sentences.\n").unwrap();

    fs::write(
        settings_path(root),
        serde_json::to_vec(&default_settings()).unwrap(),
    )
    .unwrap();
    save_settings(root, &every_setting()).unwrap();
    temp
}

fn snapshot(python: &[&str], root: &Path, tmp: &Path) -> String {
    let result = run(python, "check.py", &["snapshot"], root, tmp);
    assert_eq!(result.code, 0, "{}", result.out);
    assert!(
        !result.out.contains("does not know"),
        "check.py does not know a record or a setting that Longhand writes. \
         Add it to _longhand.py and the references.\n{}",
        result.out
    );
    snapshot_path(&result)
}

/// The snapshot file that a run of `check.py snapshot` names.
fn snapshot_path(result: &Run) -> String {
    result
        .out
        .lines()
        .find_map(|line| line.strip_prefix("Snapshot: "))
        .unwrap()
        .to_string()
}

/// The edit of an agent: one more tag in `session.md`.
fn add_tag(dir: &Path) {
    let path = dir.join(SESSION_FILE);
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("- Finance\n"), "{text}");
    fs::write(
        &path,
        text.replace("- Finance\n", "- Finance\n- Person::Nathan\n"),
    )
    .unwrap();
}

/// The edit of an agent: one value in `settings.json`, with the other keys
/// as they are.
fn edit_settings(root: &Path, key: &str, value: serde_json::Value) {
    let path = settings_path(root);
    let mut settings: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    settings.insert(key.to_string(), value);
    fs::write(&path, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();
}

#[test]
fn check_passes_a_tag_and_a_settings_edit_on_a_folder_that_the_writers_made() {
    let Some(python) = python_or_skip() else {
        return;
    };
    let storage = storage_folder();
    let tmp = tempfile::tempdir().unwrap();
    let root = storage.path();
    let snapshot = snapshot(&python, root, tmp.path());

    add_tag(&root.join(EDITED));
    edit_settings(root, "show_recording_widget", false.into());
    // The id that the prompt scan gives a file with no id. `check.py`
    // must make the same id, or it refuses the setting.
    let scan = scan_prompts(root).unwrap();
    let plain = scan
        .prompts
        .iter()
        .find(|p| p.file_name == "Plain.md")
        .unwrap();
    assert_eq!(plain.id_in_file, None);
    edit_settings(root, "active_summary_prompt", plain.id.clone().into());

    let result = run(
        &python,
        "check.py",
        &["compare", &snapshot],
        root,
        tmp.path(),
    );
    assert_eq!(result.code, 0, "{}", result.out);
    assert!(
        result
            .out
            .contains(&format!("changed: {EDITED}: {SESSION_FILE}")),
        "{}",
        result.out
    );
    assert!(
        result.out.contains("show_recording_widget"),
        "{}",
        result.out
    );
}

#[test]
fn check_does_not_pass_an_edit_of_a_note_with_a_record_that_it_does_not_know() {
    let Some(python) = python_or_skip() else {
        return;
    };
    let storage = storage_folder();
    let tmp = tempfile::tempdir().unwrap();
    let root = storage.path();
    // A writer that adds a record: the file has a key that `check.py`
    // does not know.
    let dir = root.join(EDITED);
    let path = dir.join(NOTE_RECORD_FILE);
    let mut record: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record.insert("future_record".to_string(), true.into());
    fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let snapshot = run(&python, "check.py", &["snapshot"], root, tmp.path());
    assert_eq!(snapshot.code, 0, "{}", snapshot.out);
    assert!(snapshot.out.contains("future_record"), "{}", snapshot.out);
    let path = snapshot_path(&snapshot);
    add_tag(&dir);
    let result = run(&python, "check.py", &["compare", &path], root, tmp.path());
    assert_eq!(result.code, 2, "{}", result.out);
    assert!(result.out.contains("future_record"), "{}", result.out);
}

#[test]
fn new_id_gives_an_id_that_the_prompt_scan_accepts() {
    let Some(python) = python_or_skip() else {
        return;
    };
    let storage = storage_folder();
    let tmp = tempfile::tempdir().unwrap();
    let root = storage.path();
    let result = run(&python, "new_id.py", &[], root, tmp.path());
    assert_eq!(result.code, 0, "{}", result.out);
    let id = result.out.trim().to_string();

    let text = render_prompt_file(&id, "Drift prompt", "List the topics.");
    write_atomic(&prompts_folder(root), "Drift prompt.md", text.as_bytes()).unwrap();
    let scan = scan_prompts(root).unwrap();
    let prompt = scan.get(&id).expect("the scan lists the prompt by its id");
    assert_eq!(prompt.id_in_file.as_deref(), Some(id.as_str()));
    assert_eq!(prompt.name, "Drift prompt");
    assert!(!scan.conflicted.contains(&id));
    assert!(
        uuid_like(&id),
        "{id} is not a UUID in the format that Longhand writes"
    );
}

/// The form of `Uuid::to_string`: 8-4-4-4-12 lowercase hex digits.
fn uuid_like(id: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    parts.iter().map(|p| p.len()).collect::<Vec<_>>() == [8, 4, 4, 4, 12]
        && parts
            .iter()
            .all(|p| p.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')))
}

#[test]
fn folder_names_match_the_shared_cases() {
    let text = fs::read_to_string(repo().join("testdata/folder-names.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let cases = value["cases"].as_array().unwrap();
    assert!(cases.len() > 10);
    for case in cases {
        let started = at(case["started"].as_str().unwrap());
        assert_eq!(
            folder_name(started, case["title"].as_str()),
            case["name"].as_str().unwrap(),
            "{}",
            case["case"]
        );
    }
}

/// The skill's copy of the built-in prompts.
fn built_in_reference() -> String {
    let path = repo().join("plugins/longhand/skills/longhand-notes/references/built-in-prompts.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The first fenced block in the section whose `## ` heading names `id` in
/// a code span, or `None` when there is no such section or block.
fn reference_block(reference: &str, id: &str) -> Option<String> {
    let name = format!("`{id}`");
    let mut lines = reference
        .lines()
        .skip_while(|line| !(line.starts_with("## ") && line.contains(&name)))
        .skip(1);
    for line in lines.by_ref() {
        if line.starts_with("## ") {
            return None;
        }
        if line.starts_with("```") {
            break;
        }
    }
    // The iterator is now after the opening fence.
    let mut block = String::new();
    for line in lines {
        if line == "```" {
            return Some(block);
        }
        block.push_str(line);
        block.push('\n');
    }
    None
}

/// The text after the front matter of the prompt file that Longhand writes
/// for `prompt`.
fn prompt_file_body(prompt: &Prompt) -> String {
    let file =
        render_prompt_file_with_title(&prompt.id, &prompt.name, &prompt.text, &prompt.title_text);
    let (_, body) = file
        .split_once("\n---\n\n")
        .expect("a prompt file has front matter");
    body.to_string()
}

#[test]
fn the_skill_has_the_text_of_each_built_in_prompt() {
    let reference = built_in_reference();
    for prompt in built_ins() {
        let block = reference_block(&reference, &prompt.id).unwrap_or_else(|| {
            panic!(
                "references/built-in-prompts.md has no block for the built-in prompt `{}`",
                prompt.id
            )
        });
        assert_eq!(
            block,
            prompt_file_body(&prompt),
            "the block of `{}` in references/built-in-prompts.md is not the text of the app",
            prompt.id
        );
    }
}

/// An agent copies a block after new front matter. Longhand reads the copy
/// as a prompt with the text of the built-in prompt.
#[test]
fn a_copied_block_scans_as_the_built_in_prompt() {
    let reference = built_in_reference();
    for (n, prompt) in built_ins().iter().enumerate() {
        let id = format!("00000000-0000-4000-8000-{n:012}");
        let id = id.as_str();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let block = reference_block(&reference, &prompt.id).unwrap();
        let file = format!("---\nid: {id}\nname: \"My copy\"\n---\n\n{block}");
        let prompts = prompts_folder(root);
        fs::create_dir_all(&prompts).unwrap();
        write_atomic(&prompts, "My copy.md", file.as_bytes()).unwrap();

        let scan = scan_prompts(root).unwrap();
        let copy = scan.get(id).expect("the scan lists the copy by its id");
        assert_eq!(copy.text, prompt.text, "`{}`", prompt.id);
        assert_eq!(copy.title_text, prompt.title_text, "`{}`", prompt.id);
    }
}
