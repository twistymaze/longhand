//! Two or three computers over one synced storage folder (U3): each has
//! its own local cache, and all of them read and change the shared files
//! in one temporary folder, as a sync service shows them.

use std::fs;
use std::path::Path;

use serde_json::json;
use shared_settings::background::save_background;
use shared_settings::cache::read_cache;
use shared_settings::file::{
    AudioRemoval, Field, SharedValue, apply_patch, default_settings, save_settings, settings_path,
};
use shared_settings::join::{JoinKind, Joined, join_folder};
use shared_settings::prompts::{
    create_prompt, render_prompt_file, set_prompt_text, set_prompt_title_text,
};
use shared_settings::view::{ActivePrompt, Availability, SharedView, refresh};

/// One computer: its local cache folder.
struct Computer {
    cache: tempfile::TempDir,
}

fn computer() -> Computer {
    Computer {
        cache: tempfile::tempdir().unwrap(),
    }
}

impl Computer {
    fn join(&self, root: &Path, kind: JoinKind) -> Joined {
        join_folder(root, self.cache.path(), kind).unwrap()
    }

    fn refresh(&self, root: &Path) -> SharedView {
        refresh(root, self.cache.path())
    }

    fn save(&self, root: &Path, patch: &[SharedValue]) -> SharedView {
        save_settings(root, patch).unwrap();
        self.refresh(root)
    }
}

fn names(view: &SharedView) -> Vec<&str> {
    view.prompts.iter().map(|p| p.name.as_str()).collect()
}

fn active_text(view: &SharedView) -> &str {
    match &view.active {
        ActivePrompt::Custom { prompt, .. } | ActivePrompt::BuiltIn(prompt) => &prompt.text,
        ActivePrompt::Missing(id) => panic!("the active prompt {id} is missing"),
    }
}

#[test]
fn a_change_of_the_active_prompt_on_a_shows_on_b() {
    // AE2.
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    b.join(folder.path(), JoinKind::ChangeOfFolder);
    let standup = create_prompt(folder.path(), "Standup", "A's standup.").unwrap();
    a.save(
        folder.path(),
        &[SharedValue::ActiveSummaryPrompt(standup.id.clone())],
    );
    let on_b = b.refresh(folder.path());
    assert_eq!(
        on_b.fields.active_summary_prompt,
        Field::Known(standup.id.clone())
    );
    assert_eq!(active_text(&on_b), "A's standup.");
}

#[test]
fn b_joins_the_folder_of_a_and_a_sees_the_prompt_of_b() {
    // AE3: B has "Standup", and the folder has A's settings.
    let folder = tempfile::tempdir().unwrap();
    let old_folder_of_b = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    a.save(folder.path(), &[SharedValue::LiveTranscript(false)]);
    b.join(old_folder_of_b.path(), JoinKind::ChangeOfFolder);
    create_prompt(old_folder_of_b.path(), "Standup", "B's standup.").unwrap();
    b.refresh(old_folder_of_b.path());

    let joined = b.join(folder.path(), JoinKind::ChangeOfFolder);
    assert_eq!(joined.written.prompt_files, vec!["Standup.md".to_string()]);
    // B uses A's settings.
    assert_eq!(joined.view.fields.live_transcript, Field::Known(false));
    assert_eq!(names(&a.refresh(folder.path())), vec!["Standup"]);
    // The old folder keeps its copy (R21).
    assert!(settings_path(old_folder_of_b.path()).exists());
    assert_eq!(
        names(&refresh(
            old_folder_of_b.path(),
            tempfile::tempdir().unwrap().path()
        )),
        vec!["Standup"]
    );
}

#[test]
fn two_texts_of_one_prompt_show_as_two_prompts_on_both_computers() {
    // AE8.
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    b.join(folder.path(), JoinKind::ChangeOfFolder);
    let recap = create_prompt(folder.path(), "Recap", "First text.").unwrap();
    set_prompt_text(folder.path(), &recap.id, "A's text.").unwrap();
    // The sync keeps B's offline change as a conflict copy.
    fs::write(
        folder
            .path()
            .join(".longhand/prompts/Recap (B's conflicted copy).md"),
        render_prompt_file(&recap.id, "Recap", "B's text."),
    )
    .unwrap();
    let on_a = a.refresh(folder.path());
    let on_b = b.refresh(folder.path());
    assert_eq!(on_a.prompts, on_b.prompts);
    assert_eq!(on_a.prompts.len(), 2);
    let texts: Vec<&str> = on_a.prompts.iter().map(|p| p.text.as_str()).collect();
    assert!(texts.contains(&"A's text.") && texts.contains(&"B's text."));
}

#[test]
fn a_title_text_change_on_a_shows_on_b() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    b.join(folder.path(), JoinKind::ChangeOfFolder);
    let recap = create_prompt(folder.path(), "Recap", "Recap text.").unwrap();
    b.save(
        folder.path(),
        &[SharedValue::ActiveSummaryPrompt(recap.id.clone())],
    );
    set_prompt_title_text(folder.path(), &recap.id, "A's title text.").unwrap();
    let on_b = b.refresh(folder.path());
    assert_eq!(on_b.prompts.len(), 1);
    assert_eq!(on_b.prompts[0].title_text, "A's title text.");
    assert_eq!(on_b.prompts[0].text, "Recap text.");
    match &on_b.active {
        ActivePrompt::Custom { prompt, from_cache } => {
            assert_eq!(prompt.title_text, "A's title text.");
            assert!(!from_cache);
        }
        other => panic!("unexpected active prompt {other:?}"),
    }
    // B's cache keeps the title text for a folder that goes away.
    let cache = read_cache(b.cache.path()).unwrap();
    assert_eq!(cache.prompts[0].title_text, "A's title text.");
    assert_eq!(cache.active_prompt.unwrap().title_text, "A's title text.");
}

#[test]
fn two_computers_with_different_caches_give_each_file_the_same_id() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    let recap = create_prompt(folder.path(), "Recap", "Text one.").unwrap();
    // Only A has read the folder since the prompt was made.
    a.refresh(folder.path());
    fs::write(
        folder.path().join(".longhand/prompts/Recap 2.md"),
        render_prompt_file(&recap.id, "Recap", "Text two."),
    )
    .unwrap();
    fs::write(
        folder.path().join(".longhand/prompts/Plain.md"),
        "A prompt with no front matter.",
    )
    .unwrap();
    assert_ne!(read_cache(a.cache.path()), read_cache(b.cache.path()));
    let ids = |view: &SharedView| {
        let mut ids: Vec<(String, String)> = view
            .prompts
            .iter()
            .map(|p| (p.name.clone(), p.id.clone()))
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(
        ids(&a.refresh(folder.path())),
        ids(&b.refresh(folder.path()))
    );
}

#[test]
fn a_save_on_b_keeps_the_active_prompt_that_a_chose() {
    // AE12.
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    b.join(folder.path(), JoinKind::ChangeOfFolder);
    let standup = create_prompt(folder.path(), "Standup", "Text.").unwrap();
    a.save(
        folder.path(),
        &[SharedValue::ActiveSummaryPrompt(standup.id.clone())],
    );
    b.refresh(folder.path());
    let on_b = b.save(folder.path(), &[SharedValue::LiveTranscript(false)]);
    assert_eq!(
        on_b.fields.active_summary_prompt,
        Field::Known(standup.id.clone())
    );
    let on_a = a.refresh(folder.path());
    assert_eq!(on_a.fields.active_summary_prompt, Field::Known(standup.id));
    assert_eq!(on_a.fields.live_transcript, Field::Known(false));
}

#[test]
fn an_older_version_keeps_the_model_that_a_newer_version_chose() {
    // AE11: A runs a newer version and names a model that B does not know.
    let folder = tempfile::tempdir().unwrap();
    let b = computer();
    let mut newer = apply_patch(&default_settings(), &[]);
    newer.insert("schema_version".into(), json!(4));
    newer.insert("asr_model".into(), json!("whisper-next-9b"));
    fs::create_dir_all(folder.path().join(".longhand")).unwrap();
    fs::write(
        settings_path(folder.path()),
        serde_json::to_vec_pretty(&newer).unwrap(),
    )
    .unwrap();
    let on_b = b.join(folder.path(), JoinKind::ChangeOfFolder).view;
    assert!(on_b.fields.newer_schema());
    assert_eq!(
        on_b.fields.asr_model,
        Field::Unknown(json!("whisper-next-9b"))
    );
    b.save(
        folder.path(),
        &[SharedValue::CheckUpdatesAutomatically(false)],
    );
    let on_disk: serde_json::Value =
        serde_json::from_slice(&fs::read(settings_path(folder.path())).unwrap()).unwrap();
    assert_eq!(on_disk["asr_model"], json!("whisper-next-9b"));
    assert_eq!(on_disk["schema_version"], json!(4));
}

#[test]
fn c_keeps_the_settings_of_the_old_folder_when_a_changes_folder() {
    // AE9: A and C share a folder, and A changes to a new folder.
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let (a, c) = (computer(), computer());
    a.join(old.path(), JoinKind::ChangeOfFolder);
    create_prompt(old.path(), "Standup", "Text.").unwrap();
    a.save(old.path(), &[SharedValue::LiveTranscript(false)]);
    c.join(old.path(), JoinKind::ChangeOfFolder);

    let on_a = a.join(new.path(), JoinKind::ChangeOfFolder);
    assert!(on_a.written.settings_file);
    assert_eq!(names(&on_a.view), vec!["Standup"]);
    assert_eq!(on_a.view.fields.live_transcript, Field::Known(false));

    let on_c = c.refresh(old.path());
    assert_eq!(on_c.availability, Availability::Available);
    assert_eq!(names(&on_c), vec!["Standup"]);
    assert_eq!(on_c.fields.live_transcript, Field::Known(false));
}

#[test]
fn a_folder_that_goes_away_keeps_the_last_shared_settings() {
    // AE7, R16: the folder is on a disk that is not connected.
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("Notes");
    fs::create_dir(&folder).unwrap();
    let a = computer();
    a.join(&folder, JoinKind::ChangeOfFolder);
    let standup = create_prompt(&folder, "Standup", "Text.").unwrap();
    a.save(&folder, &[SharedValue::ActiveSummaryPrompt(standup.id)]);
    fs::rename(&folder, parent.path().join("Disconnected")).unwrap();

    let view = a.refresh(&folder);
    assert!(!view.writable());
    assert_eq!(names(&view), vec!["Standup"]);
    assert_eq!(active_text(&view), "Text.");
    // The cache still holds the prompts for the next launch.
    assert_eq!(read_cache(a.cache.path()).unwrap().prompts.len(), 1);

    fs::rename(parent.path().join("Disconnected"), &folder).unwrap();
    assert!(a.refresh(&folder).writable());
}

#[test]
fn a_partial_settings_file_makes_the_settings_read_only_until_it_is_complete() {
    // KTD3: a sync delivers a file in parts.
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    b.join(folder.path(), JoinKind::ChangeOfFolder);
    let complete = fs::read(settings_path(folder.path())).unwrap();
    fs::write(
        settings_path(folder.path()),
        &complete[..complete.len() / 3],
    )
    .unwrap();
    let on_b = b.refresh(folder.path());
    assert!(!on_b.writable());
    assert_eq!(on_b.fields.live_transcript, Field::Known(true));
    fs::write(settings_path(folder.path()), &complete).unwrap();
    assert!(b.refresh(folder.path()).writable());
}

#[test]
fn a_change_of_audio_removal_on_a_applies_on_b() {
    // R1: one value applies on every computer.
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    b.join(folder.path(), JoinKind::SameFolder);
    assert_eq!(b.refresh(folder.path()).audio_removal_days(), Some(30));

    a.save(
        folder.path(),
        &[SharedValue::AudioRemoval(AudioRemoval::AfterDays(90))],
    );
    assert_eq!(b.refresh(folder.path()).audio_removal_days(), Some(90));

    a.save(
        folder.path(),
        &[SharedValue::AudioRemoval(AudioRemoval::Off)],
    );
    let on_b = b.refresh(folder.path());
    assert_eq!(on_b.fields.audio_removal, Field::Known(AudioRemoval::Off));
    assert_eq!(on_b.audio_removal_days(), None);
}

#[test]
fn a_partial_settings_file_gives_no_audio_removal_delay_until_it_is_complete() {
    // AE10: B keeps its cache for display, but removes no audio.
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    b.join(folder.path(), JoinKind::ChangeOfFolder);
    let complete = fs::read(settings_path(folder.path())).unwrap();
    let partial = &complete[..complete.len() / 3];
    fs::write(settings_path(folder.path()), partial).unwrap();
    let on_b = b.refresh(folder.path());
    assert_eq!(
        on_b.fields.audio_removal,
        Field::Known(AudioRemoval::AfterDays(30))
    );
    assert_eq!(on_b.audio_removal_days(), None);
    assert_eq!(fs::read(settings_path(folder.path())).unwrap(), partial);
    fs::write(settings_path(folder.path()), &complete).unwrap();
    assert_eq!(b.refresh(folder.path()).audio_removal_days(), Some(30));
}

#[test]
fn a_background_change_on_a_shows_on_b_and_a_prompt_save_on_b_keeps_it() {
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    b.join(folder.path(), JoinKind::ChangeOfFolder);
    let standup = create_prompt(folder.path(), "Standup", "B's standup.").unwrap();

    save_background(folder.path(), "A's Background.").unwrap();
    let on_a = a.refresh(folder.path());
    assert_eq!(on_a.background, "A's Background.");
    let on_b = b.refresh(folder.path());
    assert_eq!(on_b.background, "A's Background.");
    assert!(on_b.background_writable());
    assert_eq!(
        read_cache(b.cache.path()).unwrap().background,
        "A's Background."
    );

    // B saves a prompt and a setting. A's Background stays.
    set_prompt_text(folder.path(), &standup.id, "B's new standup.").unwrap();
    b.save(folder.path(), &[SharedValue::LiveTranscript(false)]);
    assert_eq!(a.refresh(folder.path()).background, "A's Background.");
    assert_eq!(
        fs::read_to_string(folder.path().join(".longhand").join("background.md")).unwrap(),
        "A's Background."
    );
}

#[test]
fn b_keeps_the_cached_background_while_the_folder_is_missing() {
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("Notes");
    fs::create_dir(&folder).unwrap();
    let b = computer();
    b.join(&folder, JoinKind::ChangeOfFolder);
    save_background(&folder, "Kept on B.").unwrap();
    assert_eq!(b.refresh(&folder).background, "Kept on B.");
    fs::rename(&folder, parent.path().join("Away")).unwrap();
    let on_b = b.refresh(&folder);
    assert!(matches!(on_b.availability, Availability::NotAvailable(_)));
    assert_eq!(on_b.background, "Kept on B.");
    assert!(!on_b.background_writable());
}

/// The effective type names and the default type of `view`, with the type
/// folders `folders` that both computers see after the sync.
fn types_of(
    view: &SharedView,
    folders: &[shared_settings::note_types::TypeFolder],
) -> (Vec<String>, Option<String>) {
    use shared_settings::note_types::{effective_default, effective_types};
    let shared = view.fields.note_types.known().cloned().unwrap_or_default();
    let types = effective_types(&shared, folders);
    let default = view.fields.default_note_type.known().cloned().flatten();
    (
        types.iter().map(|t| t.name.clone()).collect(),
        effective_default(default.as_deref(), &types),
    )
}

#[test]
fn both_computers_show_the_same_note_types_and_the_same_default_type() {
    // R2, R7, KTD9.
    use shared_settings::note_types::{TypeFolder, add_type, default_to_save};
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    b.join(folder.path(), JoinKind::ChangeOfFolder);
    // Both computers see one type folder that is not in the shared list.
    let folders = [TypeFolder {
        name: "Customer Calls".into(),
        notes: 1,
        leaving: false,
    }];

    let (list, _) = add_type(&[], &folders, &[], "Pathfinder").unwrap();
    a.save(folder.path(), &[SharedValue::NoteTypes(list.clone())]);
    let (default, more) = default_to_save(&list, &folders, Some("pathfinder")).unwrap();
    assert_eq!(more, None);
    let on_a = a.save(folder.path(), &[SharedValue::DefaultNoteType(default)]);

    let before = fs::read(settings_path(folder.path())).unwrap();
    let on_b = b.refresh(folder.path());
    assert_eq!(fs::read(settings_path(folder.path())).unwrap(), before);
    let expected = (
        vec!["Customer Calls".to_string(), "Pathfinder".to_string()],
        Some("Pathfinder".to_string()),
    );
    assert_eq!(types_of(&on_a, &folders), expected);
    assert_eq!(types_of(&on_b, &folders), expected);

    // B sets General. A shows it after the sync.
    b.save(folder.path(), &[SharedValue::DefaultNoteType(None)]);
    assert_eq!(types_of(&a.refresh(folder.path()), &folders).1, None);
}

#[test]
fn a_change_of_folder_copies_the_note_types_into_a_folder_with_no_settings() {
    // KTD14: the new folder gets this computer's types and default type.
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let a = computer();
    a.join(old.path(), JoinKind::ChangeOfFolder);
    a.save(
        old.path(),
        &[
            SharedValue::NoteTypes(vec!["Pathfinder".into(), "One on One".into()]),
            SharedValue::DefaultNoteType(Some("One on One".into())),
        ],
    );
    let joined = a.join(new.path(), JoinKind::ChangeOfFolder);
    assert!(joined.written.settings_file);
    let fields = &joined.view.fields;
    assert_eq!(
        fields.note_types,
        Field::Known(vec!["Pathfinder".to_string(), "One on One".to_string()])
    );
    assert_eq!(
        fields.default_note_type,
        Field::Known(Some("One on One".to_string()))
    );
}

/// The prompt of the type `name` in `view`: the choice by the production
/// rule, then the prompt that the view holds for that id.
fn type_prompt<'a>(view: &'a SharedView, name: Option<&str>) -> Option<&'a ActivePrompt> {
    let choices = view.fields.type_prompt_map();
    let id = shared_settings::note_types::prompt_choice(&choices, name, true)?;
    view.type_prompts.get(id)
}

fn map(entries: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, id)| (name.to_string(), id.to_string()))
        .collect()
}

#[test]
fn a_change_of_the_type_prompts_on_a_shows_on_b_and_a_save_of_b_keeps_it() {
    // R2, KTD1.
    let folder = tempfile::tempdir().unwrap();
    let (a, b) = (computer(), computer());
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    b.join(folder.path(), JoinKind::ChangeOfFolder);
    let standup = create_prompt(folder.path(), "Standup", "A's standup.").unwrap();
    a.save(
        folder.path(),
        &[
            SharedValue::NoteTypes(vec!["Pathfinder".into()]),
            SharedValue::NoteTypePrompts(map(&[("Pathfinder", &standup.id)])),
        ],
    );
    let on_b = b.refresh(folder.path());
    match type_prompt(&on_b, Some("pathfinder")) {
        Some(ActivePrompt::Custom { prompt, from_cache }) => {
            assert_eq!(prompt.text, "A's standup.");
            assert!(!from_cache);
        }
        other => panic!("expected the Standup prompt, got {other:?}"),
    }
    b.save(folder.path(), &[SharedValue::LiveTranscript(false)]);
    let on_a = a.refresh(folder.path());
    assert_eq!(
        on_a.fields.note_type_prompts,
        Field::Known(map(&[("Pathfinder", &standup.id)]))
    );
    assert_eq!(on_a.fields.live_transcript, Field::Known(false));
}

#[test]
fn a_type_prompt_whose_file_is_gone_is_the_last_copy_that_this_computer_read() {
    // R7, KTD2.
    let folder = tempfile::tempdir().unwrap();
    let a = computer();
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    let standup = create_prompt(folder.path(), "Standup", "A's standup.").unwrap();
    a.save(
        folder.path(),
        &[
            SharedValue::NoteTypes(vec!["Pathfinder".into()]),
            SharedValue::NoteTypePrompts(map(&[("Pathfinder", &standup.id)])),
        ],
    );
    shared_settings::prompts::delete_prompt(folder.path(), &standup.id).unwrap();
    // Each read after the file went keeps the copy.
    a.refresh(folder.path());
    let view = a.refresh(folder.path());
    assert!(view.prompts.is_empty());
    match type_prompt(&view, Some("Pathfinder")) {
        Some(ActivePrompt::Custom { prompt, from_cache }) => {
            assert_eq!(prompt.text, "A's standup.");
            assert!(from_cache);
        }
        other => panic!("expected the cached Standup prompt, got {other:?}"),
    }
}

#[test]
fn a_type_prompt_that_this_computer_never_read_gives_no_prompt() {
    // R7: the caller uses the default prompt.
    let folder = tempfile::tempdir().unwrap();
    let a = computer();
    a.join(folder.path(), JoinKind::ChangeOfFolder);
    let view = a.save(
        folder.path(),
        &[
            SharedValue::NoteTypes(vec!["Pathfinder".into()]),
            SharedValue::NoteTypePrompts(map(&[
                ("Pathfinder", "never-read-id"),
                ("General", summarize::prompts::TOPIC_SUMMARY_ID),
            ])),
        ],
    );
    assert_eq!(type_prompt(&view, Some("Pathfinder")), None);
    assert_eq!(
        type_prompt(&view, None),
        Some(&ActivePrompt::BuiltIn(summarize::prompts::topic_summary()))
    );
}
