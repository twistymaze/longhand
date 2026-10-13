//! The local cache and the folder join (U3): each computer keeps the last
//! shared settings, and a change of folder adopts or fills the folder's
//! settings (R16, R21, R22, KTD6, KTD8).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use shared_settings::cache::{Cache, read_cache, write_cache};
use shared_settings::file::{
    AudioRemoval, Field, SharedValue, Unavailable, apply_patch, default_settings,
};
use shared_settings::join::{JoinKind, join};
use shared_settings::prompts::{
    create_prompt, render_prompt_file, render_prompt_file_with_title, scan_prompts,
};
use shared_settings::view::{ActivePrompt, Availability, load};
use summarize::prompts::Prompt;

fn prompt(id: &str, name: &str, text: &str) -> Prompt {
    titled(id, name, text, "")
}

fn titled(id: &str, name: &str, text: &str, title_text: &str) -> Prompt {
    Prompt {
        id: id.into(),
        name: name.into(),
        text: text.into(),
        title_text: title_text.into(),
        background: String::new(),
    }
}

/// B's cache: live transcript off, and the custom prompts `prompts`.
fn cache_of_b(source: &Path, prompts: Vec<Prompt>) -> Cache {
    Cache {
        source: source.to_path_buf(),
        settings: apply_patch(&default_settings(), &[SharedValue::LiveTranscript(false)]),
        prompts,
        active_prompt: None,
        type_prompts: Vec::new(),
        background: String::new(),
    }
}

/// A folder with A's settings: the active prompt "recap-a", and A's
/// prompt "Recap".
fn folder_of_a() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let longhand = root.path().join(".longhand");
    fs::create_dir_all(longhand.join("prompts")).unwrap();
    let settings = apply_patch(
        &default_settings(),
        &[SharedValue::ActiveSummaryPrompt("recap-a".into())],
    );
    fs::write(
        longhand.join("settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();
    fs::write(
        longhand.join("prompts").join("Recap.md"),
        render_prompt_file("recap-a", "Recap", "A's recap."),
    )
    .unwrap();
    root
}

/// Every file under `root`, with its bytes.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                let key = path.strip_prefix(base).unwrap().display().to_string();
                out.insert(key, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn settings_on_disk(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(root.join(".longhand").join("settings.json")).unwrap())
        .unwrap()
}

fn names(prompts: &[Prompt]) -> Vec<&str> {
    prompts.iter().map(|p| p.name.as_str()).collect()
}

#[test]
fn the_cache_reads_back_what_was_written() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read_cache(dir.path()), None);
    let cache = Cache {
        active_prompt: Some(prompt("p1", "Standup", "Text.")),
        ..cache_of_b(
            Path::new("/Volumes/Notes"),
            vec![prompt("p1", "Standup", "Text.")],
        )
    };
    write_cache(dir.path(), &cache).unwrap();
    assert_eq!(read_cache(dir.path()), Some(cache));
}

#[test]
fn a_cache_that_does_not_parse_reads_as_no_cache() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join(shared_settings::cache::CACHE_FILE),
        "{\"sou",
    )
    .unwrap();
    assert_eq!(read_cache(dir.path()), None);
}

#[test]
fn a_folder_with_settings_gets_the_missing_prompts_and_is_used() {
    // AE3: B has "Standup", and the folder has A's settings.
    let root = folder_of_a();
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(
        old.path(),
        vec![prompt("standup-b", "Standup", "B's standup.")],
    );
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(!joined.written.settings_file);
    assert_eq!(joined.written.prompt_files, vec!["Standup.md".to_string()]);
    let view = &joined.view;
    assert_eq!(view.availability, Availability::Available);
    // A's settings win.
    assert_eq!(view.fields.live_transcript, Field::Known(true));
    assert_eq!(
        view.active,
        ActivePrompt::Custom {
            prompt: prompt("recap-a", "Recap", "A's recap."),
            from_cache: false
        }
    );
    assert_eq!(names(&view.prompts), vec!["Recap", "Standup"]);
    assert_eq!(
        view.prompts
            .iter()
            .find(|p| p.name == "Standup")
            .unwrap()
            .id,
        "standup-b"
    );
    // The new cache comes from this folder.
    let new_cache = joined.cache.unwrap();
    assert_eq!(new_cache.source, root.path());
    assert_eq!(names(&new_cache.prompts), vec!["Recap", "Standup"]);
}

#[test]
fn a_prompt_with_the_same_name_and_another_text_gets_a_free_name() {
    // AE4.
    let root = folder_of_a();
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(old.path(), vec![prompt("recap-b", "Recap", "B's recap.")]);
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert_eq!(joined.written.prompt_files, vec!["Recap 2.md".to_string()]);
    let recap_2 = joined
        .view
        .prompts
        .iter()
        .find(|p| p.name == "Recap 2")
        .unwrap();
    assert_eq!(recap_2.text, "B's recap.");
    assert_eq!(
        joined
            .view
            .prompts
            .iter()
            .find(|p| p.name == "Recap")
            .unwrap()
            .text,
        "A's recap."
    );
}

#[test]
fn a_prompt_with_the_same_name_and_the_same_text_is_not_added() {
    let root = folder_of_a();
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(old.path(), vec![prompt("recap-b", " recap ", "A's recap.")]);
    let before = snapshot(root.path());
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(joined.written.prompt_files.is_empty());
    assert_eq!(snapshot(root.path()), before);
}

#[test]
fn a_join_that_runs_two_times_adds_each_prompt_one_time() {
    let root = folder_of_a();
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(
        old.path(),
        vec![
            prompt("recap-b", "Recap", "B's recap."),
            prompt("standup-b", "Standup", "B's standup."),
        ],
    );
    let first = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert_eq!(first.written.prompt_files.len(), 2);
    let after_first = snapshot(root.path());
    // The same old cache again, as after a crash before the cache write.
    let second = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(second.written.prompt_files.is_empty());
    assert_eq!(snapshot(root.path()), after_first);
    // And with the new cache.
    let third = join(root.path(), first.cache.as_ref(), JoinKind::ChangeOfFolder).unwrap();
    assert!(third.written.prompt_files.is_empty());
    assert_eq!(snapshot(root.path()), after_first);
}

#[test]
fn a_cached_prompt_whose_id_the_folder_has_with_another_text_is_added_with_its_own_id() {
    let root = folder_of_a();
    let old = tempfile::tempdir().unwrap();
    // B's copy of A's prompt, with an older text.
    let cache = cache_of_b(old.path(), vec![prompt("recap-a", "Recap", "Older recap.")]);
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert_eq!(joined.written.prompt_files.len(), 1);
    let scan = scan_prompts(root.path()).unwrap();
    // Two prompts, and neither is a conflict copy of the other.
    assert_eq!(scan.prompts.len(), 2);
    assert!(scan.conflicted.is_empty());
    assert_eq!(scan.get("recap-a").unwrap().text, "A's recap.");
    let again = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(again.written.prompt_files.is_empty());
}

#[test]
fn a_prompt_with_the_same_name_and_text_and_another_title_text_is_added() {
    // KTD9: the title text is part of what makes two prompts the same.
    let root = folder_of_a();
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(
        old.path(),
        vec![titled("recap-b", "Recap", "A's recap.", "B's title.")],
    );
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert_eq!(joined.written.prompt_files, vec!["Recap 2.md".to_string()]);
    let scan = scan_prompts(root.path()).unwrap();
    assert_eq!(scan.prompts.len(), 2);
    assert_eq!(scan.get("recap-a").unwrap().title_text, "");
    assert_eq!(scan.get("recap-b").unwrap().title_text, "B's title.");
    assert_eq!(scan.get("recap-b").unwrap().text, "A's recap.");
    let again = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(again.written.prompt_files.is_empty());
}

#[test]
fn a_cached_prompt_whose_id_the_folder_has_with_another_title_text_is_added_with_its_own_id() {
    let root = folder_of_a();
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(
        old.path(),
        vec![titled("recap-a", "Recap", "A's recap.", "Older title.")],
    );
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert_eq!(joined.written.prompt_files.len(), 1);
    let scan = scan_prompts(root.path()).unwrap();
    assert_eq!(scan.prompts.len(), 2);
    assert!(scan.conflicted.is_empty());
    assert_eq!(scan.get("recap-a").unwrap().title_text, "");
    let added = scan.prompts.iter().find(|p| p.id != "recap-a").unwrap();
    assert_eq!(added.title_text, "Older title.");
}

#[test]
fn a_prompt_with_the_same_texts_and_title_text_is_not_added() {
    let root = folder_of_a();
    fs::write(
        root.path().join(".longhand/prompts/Recap.md"),
        render_prompt_file_with_title("recap-a", "Recap", "A's recap.", "A's title."),
    )
    .unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(
        old.path(),
        vec![titled("recap-b", "Recap", "A's recap.", "A's title.")],
    );
    let before = snapshot(root.path());
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(joined.written.prompt_files.is_empty());
    assert_eq!(snapshot(root.path()), before);
    // The view and the cache carry the title text.
    let recap = joined
        .view
        .prompts
        .iter()
        .find(|p| p.id == "recap-a")
        .unwrap();
    assert_eq!(recap.title_text, "A's title.");
    let cached = joined.cache.unwrap();
    assert_eq!(cached.prompts[0].title_text, "A's title.");
}

#[test]
fn an_empty_folder_gets_the_cache() {
    // AE10, and the change to a larger disk.
    let root = tempfile::tempdir().unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = Cache {
        active_prompt: Some(prompt("standup-b", "Standup", "B's standup.")),
        settings: apply_patch(
            &default_settings(),
            &[
                SharedValue::LiveTranscript(false),
                SharedValue::ActiveSummaryPrompt("standup-b".into()),
            ],
        ),
        ..cache_of_b(
            old.path(),
            vec![prompt("standup-b", "Standup", "B's standup.")],
        )
    };
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(joined.written.settings_file);
    assert_eq!(joined.written.prompt_files, vec!["Standup.md".to_string()]);
    assert_eq!(
        Value::Object(cache.settings.clone()),
        settings_on_disk(root.path())
    );
    assert_eq!(joined.view.availability, Availability::Available);
    assert_eq!(joined.view.fields.live_transcript, Field::Known(false));
    assert_eq!(
        joined.view.active,
        ActivePrompt::Custom {
            prompt: prompt("standup-b", "Standup", "B's standup."),
            from_cache: false
        }
    );
}

#[test]
fn a_folder_with_notes_and_no_settings_file_gets_the_cache() {
    // AE10.
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("2026-09-01 Standup")).unwrap();
    fs::write(
        root.path().join("2026-09-01 Standup").join("session.md"),
        "# Note\n",
    )
    .unwrap();
    fs::create_dir_all(root.path().join(".longhand")).unwrap();
    fs::write(root.path().join(".longhand").join("library"), "").unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(old.path(), vec![]);
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(joined.written.settings_file);
    assert_eq!(
        settings_on_disk(root.path())["live_transcript"],
        json!(false)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("2026-09-01 Standup").join("session.md")).unwrap(),
        "# Note\n"
    );
}

#[test]
fn a_new_install_writes_the_defaults() {
    let root = tempfile::tempdir().unwrap();
    let joined = join(root.path(), None, JoinKind::ChangeOfFolder).unwrap();
    assert!(joined.written.settings_file);
    assert_eq!(
        Value::Object(default_settings()),
        settings_on_disk(root.path())
    );
    assert_eq!(
        joined.view.active,
        ActivePrompt::BuiltIn(
            summarize::prompts::built_in(summarize::prompts::TOPIC_SUMMARY_ID).unwrap()
        )
    );
}

#[test]
fn a_folder_with_prompt_files_and_no_settings_file_keeps_its_prompts() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join(".longhand").join("prompts")).unwrap();
    fs::write(
        root.path()
            .join(".longhand")
            .join("prompts")
            .join("Recap.md"),
        render_prompt_file("recap-a", "Recap", "A's recap."),
    )
    .unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(old.path(), vec![prompt("recap-a", "Recap", "A's recap.")]);
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(joined.written.settings_file);
    assert!(joined.written.prompt_files.is_empty());
    assert_eq!(names(&joined.view.prompts), vec!["Recap"]);
}

#[test]
fn a_folder_that_does_not_exist_writes_nothing_and_gives_the_cache_read_only() {
    // AE7: the disk of the storage folder is not connected.
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("Notes");
    let cache = Cache {
        settings: apply_patch(
            &default_settings(),
            &[
                SharedValue::LiveTranscript(false),
                SharedValue::ActiveSummaryPrompt("standup-b".into()),
            ],
        ),
        ..cache_of_b(&root, vec![prompt("standup-b", "Standup", "B's standup.")])
    };
    let joined = join(&root, Some(&cache), JoinKind::SameFolder).unwrap();
    assert!(!joined.written.settings_file);
    assert!(joined.written.prompt_files.is_empty());
    assert!(joined.cache.is_none());
    assert!(!root.exists());
    let view = joined.view;
    assert_eq!(
        view.availability,
        Availability::NotAvailable(Unavailable::FolderMissing)
    );
    assert!(!view.writable());
    assert_eq!(view.fields.live_transcript, Field::Known(false));
    assert_eq!(names(&view.prompts), vec!["Standup"]);
    assert_eq!(
        view.active,
        ActivePrompt::Custom {
            prompt: prompt("standup-b", "Standup", "B's standup."),
            from_cache: true
        }
    );
}

#[test]
fn with_no_cache_a_folder_that_is_not_available_gives_the_defaults_read_only() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("Notes");
    let view = load(&root, None).view;
    assert!(!view.writable());
    assert_eq!(view.raw, default_settings());
    assert!(view.prompts.is_empty());
}

#[test]
fn a_settings_file_that_does_not_parse_stays_and_the_cache_is_read_only() {
    let root = folder_of_a();
    let path = root.path().join(".longhand").join("settings.json");
    fs::write(&path, "{\"schema_version\": 1, \"live_tr").unwrap();
    let before = snapshot(root.path());
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(
        old.path(),
        vec![prompt("standup-b", "Standup", "B's standup.")],
    );
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(!joined.written.settings_file);
    assert!(joined.written.prompt_files.is_empty());
    assert!(joined.cache.is_none());
    assert_eq!(snapshot(root.path()), before);
    assert!(matches!(
        joined.view.availability,
        Availability::NotAvailable(Unavailable::DoesNotParse(_))
    ));
    assert_eq!(joined.view.fields.live_transcript, Field::Known(false));
    assert_eq!(names(&joined.view.prompts), vec!["Standup"]);
}

#[test]
fn a_settings_file_that_names_a_prompt_that_has_not_arrived_uses_the_default_prompt() {
    let root = folder_of_a();
    // B already uses this folder, and its cache has A's prompt "Standup".
    let standup = prompt("standup-a", "Standup", "A's standup.");
    let cache = Cache {
        source: root.path().to_path_buf(),
        settings: apply_patch(&default_settings(), &[]),
        prompts: vec![standup.clone()],
        active_prompt: None,
        type_prompts: Vec::new(),
        background: String::new(),
    };
    // The settings file arrives and names "Standup". Its file has not.
    let settings = apply_patch(
        &default_settings(),
        &[SharedValue::ActiveSummaryPrompt("standup-a".into())],
    );
    fs::write(
        root.path().join(".longhand").join("settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();
    let before = snapshot(root.path());
    let joined = join(root.path(), Some(&cache), JoinKind::SameFolder).unwrap();
    // KTD8: the cache copy does not stand in for a missing file.
    assert_eq!(
        joined.view.active,
        ActivePrompt::Missing("standup-a".into())
    );
    assert_eq!(
        joined.view.missing_prompts,
        ["standup-a".to_string()].into()
    );
    assert!(!joined.written.settings_file);
    assert!(joined.written.prompt_files.is_empty());
    assert_eq!(snapshot(root.path()), before);
    // The new cache keeps the last copy that this computer read.
    assert_eq!(joined.cache.unwrap().active_prompt, Some(standup));
}

#[test]
fn a_prompt_file_that_arrives_before_the_settings_file_changes_nothing_else() {
    let root = folder_of_a();
    let first = load(root.path(), None);
    let cache = first.cache.unwrap();
    fs::write(
        root.path()
            .join(".longhand")
            .join("prompts")
            .join("Standup.md"),
        render_prompt_file("standup-a", "Standup", "A's standup."),
    )
    .unwrap();
    let before = snapshot(root.path());
    let loaded = load(root.path(), Some(&cache));
    assert_eq!(names(&loaded.view.prompts), vec!["Recap", "Standup"]);
    assert_eq!(loaded.view.fields, first.view.fields);
    assert_eq!(loaded.view.active, first.view.active);
    assert_eq!(snapshot(root.path()), before);
    let joined = join(root.path(), Some(&cache), JoinKind::SameFolder).unwrap();
    assert!(!joined.written.settings_file);
    assert!(joined.written.prompt_files.is_empty());
    assert_eq!(snapshot(root.path()), before);
}

#[test]
fn a_cache_from_another_folder_does_not_fill_a_folder_without_a_change_of_folder() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let cache = cache_of_b(
        other.path(),
        vec![prompt("standup-b", "Standup", "B's standup.")],
    );
    let joined = join(root.path(), Some(&cache), JoinKind::SameFolder).unwrap();
    assert!(!joined.written.settings_file);
    assert!(joined.written.prompt_files.is_empty());
    assert!(snapshot(root.path()).is_empty());
    assert_eq!(joined.view.availability, Availability::MissingFile);
    assert!(!joined.view.writable());
    // The same cache fills its own folder when its settings file is gone.
    let own = cache_of_b(root.path(), vec![]);
    let refilled = join(root.path(), Some(&own), JoinKind::SameFolder).unwrap();
    assert!(refilled.written.settings_file);
}

#[test]
fn a_same_folder_join_of_a_folder_with_settings_adds_no_prompt() {
    // A prompt that another computer deleted does not come back from
    // this computer's cache.
    let root = folder_of_a();
    let cache = cache_of_b(
        root.path(),
        vec![prompt("gone", "Deleted on A", "Old text.")],
    );
    let before = snapshot(root.path());
    let joined = join(root.path(), Some(&cache), JoinKind::SameFolder).unwrap();
    assert!(joined.written.prompt_files.is_empty());
    assert_eq!(snapshot(root.path()), before);
    assert_eq!(names(&joined.view.prompts), vec!["Recap"]);
}

#[test]
fn the_active_prompt_with_a_conflict_copy_uses_the_cached_text() {
    // R14: while the active id has a conflict copy, the text comes from
    // the cache.
    let root = folder_of_a();
    let first = load(root.path(), None);
    let cache = first.cache.unwrap();
    assert_eq!(
        cache.active_prompt,
        Some(prompt("recap-a", "Recap", "A's recap."))
    );
    // Both computers changed "Recap" offline. The file now has B's text,
    // and the copy has A's newer text.
    fs::write(
        root.path()
            .join(".longhand")
            .join("prompts")
            .join("Recap.md"),
        render_prompt_file("recap-a", "Recap", "B's recap."),
    )
    .unwrap();
    fs::write(
        root.path()
            .join(".longhand")
            .join("prompts")
            .join("Recap 2.md"),
        render_prompt_file("recap-a", "Recap", "A's newer recap."),
    )
    .unwrap();
    let loaded = load(root.path(), Some(&cache));
    assert_eq!(
        loaded.view.active,
        ActivePrompt::Custom {
            prompt: prompt("recap-a", "Recap", "A's recap."),
            from_cache: true
        }
    );
    assert_eq!(loaded.view.prompts.len(), 2);
    // The cached text stays for the next read.
    assert_eq!(
        loaded.cache.unwrap().active_prompt,
        Some(prompt("recap-a", "Recap", "A's recap."))
    );
}

#[test]
fn a_prompt_that_only_a_note_type_chooses_is_copied_into_the_new_folder() {
    // R7, R22: the cache keeps the prompt of a type, and a change of
    // folder copies it like the active prompt.
    let root = folder_of_a();
    let old = tempfile::tempdir().unwrap();
    let cache = Cache {
        type_prompts: vec![prompt("standup-b", "Standup", "B's standup.")],
        ..cache_of_b(old.path(), vec![])
    };
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert_eq!(joined.written.prompt_files, vec!["Standup.md".to_string()]);
    assert_eq!(names(&joined.view.prompts), vec!["Recap", "Standup"]);
    assert_eq!(
        fs::read_to_string(
            root.path()
                .join(".longhand")
                .join("prompts")
                .join("Standup.md")
        )
        .unwrap(),
        render_prompt_file("standup-b", "Standup", "B's standup.")
    );
}

#[test]
fn a_type_prompt_with_a_conflict_copy_uses_the_cached_text() {
    // R14: while the id that a type chooses has a conflict copy, the text
    // comes from the cache. The cache has the prompt only as a type prompt.
    let root = folder_of_a();
    let prompts_dir = root.path().join(".longhand").join("prompts");
    let settings = apply_patch(
        &default_settings(),
        &[SharedValue::NoteTypePrompts(BTreeMap::from([(
            "Pathfinder".to_string(),
            "standup-a".to_string(),
        )]))],
    );
    fs::write(
        root.path().join(".longhand").join("settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();
    // Both computers changed "Standup" offline. The file has B's text,
    // and the copy has A's newer text.
    fs::write(
        prompts_dir.join("Standup.md"),
        render_prompt_file("standup-a", "Standup", "B's standup."),
    )
    .unwrap();
    fs::write(
        prompts_dir.join("Standup 2.md"),
        render_prompt_file("standup-a", "Standup", "A's newer standup."),
    )
    .unwrap();
    let cache = Cache {
        settings: settings.clone(),
        type_prompts: vec![prompt("standup-a", "Standup", "A's standup.")],
        ..cache_of_b(root.path(), vec![])
    };
    let loaded = load(root.path(), Some(&cache));
    assert_eq!(
        loaded.view.type_prompts.get("standup-a"),
        Some(&ActivePrompt::Custom {
            prompt: prompt("standup-a", "Standup", "A's standup."),
            from_cache: true
        })
    );
}

#[test]
fn the_active_prompt_with_a_conflict_copy_and_no_cache_uses_the_original_file() {
    let root = folder_of_a();
    fs::write(
        root.path()
            .join(".longhand")
            .join("prompts")
            .join("Recap 2.md"),
        render_prompt_file("recap-a", "Recap", "Other text."),
    )
    .unwrap();
    let view = load(root.path(), None).view;
    assert_eq!(
        view.active,
        ActivePrompt::Custom {
            prompt: prompt("recap-a", "Recap", "A's recap."),
            from_cache: false
        }
    );
}

#[test]
fn an_active_id_that_no_prompt_has_is_missing() {
    let root = folder_of_a();
    fs::remove_file(
        root.path()
            .join(".longhand")
            .join("prompts")
            .join("Recap.md"),
    )
    .unwrap();
    let view = load(root.path(), None).view;
    assert_eq!(view.active, ActivePrompt::Missing("recap-a".into()));
}

#[test]
fn a_load_that_matches_the_cache_gives_no_new_cache() {
    let root = folder_of_a();
    let first = load(root.path(), None);
    let cache = first.cache.unwrap();
    assert!(load(root.path(), Some(&cache)).cache.is_none());
    create_prompt(root.path(), "Standup", "Text.").unwrap();
    assert!(load(root.path(), Some(&cache)).cache.is_some());
}

#[test]
fn a_load_keeps_a_cache_from_another_folder_until_a_change_of_folder_join() {
    // R21, R22: only a join merges the prompts of that cache.
    let root = folder_of_a();
    let old = tempfile::tempdir().unwrap();
    let cache = cache_of_b(old.path(), vec![prompt("standup-b", "Standup", "Text.")]);
    let loaded = load(root.path(), Some(&cache));
    assert_eq!(loaded.view.availability, Availability::Available);
    assert_eq!(names(&loaded.view.prompts), vec!["Recap"]);
    assert!(loaded.cache.is_none());
    let legacy = Cache {
        source: PathBuf::new(),
        ..cache.clone()
    };
    assert!(load(root.path(), Some(&legacy)).cache.is_none());

    let same = join(root.path(), Some(&cache), JoinKind::SameFolder).unwrap();
    assert!(same.written.prompt_files.is_empty());
    assert!(same.cache.is_none());

    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert_eq!(joined.written.prompt_files, vec!["Standup.md".to_string()]);
    let new_cache = joined.cache.unwrap();
    assert_eq!(new_cache.source, root.path());
    assert_eq!(names(&new_cache.prompts), vec!["Recap", "Standup"]);
}

#[test]
fn a_missing_settings_file_gives_the_cache_read_only() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache_of_b(root.path(), vec![prompt("standup-b", "Standup", "Text.")]);
    let loaded = load(root.path(), Some(&cache));
    assert_eq!(loaded.view.availability, Availability::MissingFile);
    assert!(!loaded.view.writable());
    assert_eq!(names(&loaded.view.prompts), vec!["Standup"]);
    assert!(loaded.cache.is_none());
    assert!(snapshot(root.path()).is_empty());
}

#[test]
fn refresh_and_join_folder_keep_the_cache_in_the_cache_folder() {
    let root = folder_of_a();
    let cache_dir = tempfile::tempdir().unwrap();
    let joined =
        shared_settings::join::join_folder(root.path(), cache_dir.path(), JoinKind::ChangeOfFolder)
            .unwrap();
    assert_eq!(joined.view.availability, Availability::Available);
    let cached = read_cache(cache_dir.path()).unwrap();
    assert_eq!(cached.source, root.path());
    assert_eq!(names(&cached.prompts), vec!["Recap"]);
    create_prompt(root.path(), "Standup", "Text.").unwrap();
    let view = shared_settings::view::refresh(root.path(), cache_dir.path());
    assert_eq!(names(&view.prompts), vec!["Recap", "Standup"]);
    assert_eq!(
        names(&read_cache(cache_dir.path()).unwrap().prompts),
        vec!["Recap", "Standup"]
    );
}

#[test]
fn a_join_with_no_cache_writes_the_default_audio_removal_delay() {
    // R2: a new storage folder gets 30 days, with no confirmation (R18).
    let root = tempfile::tempdir().unwrap();
    let joined = join(root.path(), None, JoinKind::SameFolder).unwrap();
    assert!(joined.written.settings_file);
    assert_eq!(settings_on_disk(root.path())["audio_removal"], json!(30));
    assert_eq!(joined.view.audio_removal_days(), Some(30));
}

#[test]
fn a_change_of_folder_with_a_cache_that_holds_off_writes_off_into_the_new_folder() {
    let root = tempfile::tempdir().unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = Cache {
        settings: apply_patch(
            &default_settings(),
            &[SharedValue::AudioRemoval(AudioRemoval::Off)],
        ),
        ..cache_of_b(old.path(), Vec::new())
    };
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(joined.written.settings_file);
    assert_eq!(settings_on_disk(root.path())["audio_removal"], json!("off"));
    assert_eq!(
        joined.view.fields.audio_removal,
        Field::Known(AudioRemoval::Off)
    );
    assert_eq!(joined.view.audio_removal_days(), None);
}

#[test]
fn a_change_of_folder_keeps_the_audio_removal_value_of_a_folder_with_settings() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join(".longhand")).unwrap();
    fs::write(
        root.path().join(".longhand").join("settings.json"),
        serde_json::to_vec(&json!({"schema_version": 1, "audio_removal": 200})).unwrap(),
    )
    .unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = Cache {
        settings: apply_patch(
            &default_settings(),
            &[SharedValue::AudioRemoval(AudioRemoval::Off)],
        ),
        ..cache_of_b(old.path(), Vec::new())
    };
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(!joined.written.settings_file);
    assert_eq!(settings_on_disk(root.path())["audio_removal"], json!(200));
    assert_eq!(joined.view.audio_removal_days(), Some(200));
}

// The Background file (U1, KTD2): a join writes `background.md` only with
// a new settings file, or after a change of folder into a folder that has
// no `background.md`. It never writes an empty file and never replaces a
// file.

fn background_path(root: &Path) -> PathBuf {
    root.join(".longhand").join("background.md")
}

fn with_background(cache: Cache, background: &str) -> Cache {
    Cache {
        background: background.into(),
        ..cache
    }
}

#[test]
fn a_same_folder_join_into_a_folder_with_settings_writes_no_background() {
    let root = folder_of_a();
    let cache = with_background(cache_of_b(root.path(), Vec::new()), "B's Background.");
    let before = snapshot(root.path());
    let joined = join(root.path(), Some(&cache), JoinKind::SameFolder).unwrap();
    assert!(!joined.written.background_file);
    assert_eq!(snapshot(root.path()), before);
    // The folder has settings and no file: the Background is empty.
    assert_eq!(joined.view.background, "");
}

#[test]
fn a_join_with_no_cache_into_an_empty_folder_writes_no_background() {
    let root = tempfile::tempdir().unwrap();
    let joined = join(root.path(), None, JoinKind::SameFolder).unwrap();
    assert!(joined.written.settings_file);
    assert!(!joined.written.background_file);
    assert!(!background_path(root.path()).exists());
    assert_eq!(joined.view.background, "");
}

#[test]
fn a_join_that_fills_an_empty_folder_writes_the_cached_background() {
    let root = tempfile::tempdir().unwrap();
    let cache = with_background(cache_of_b(root.path(), Vec::new()), "B's Background.");
    let joined = join(root.path(), Some(&cache), JoinKind::SameFolder).unwrap();
    assert!(joined.written.settings_file);
    assert!(joined.written.background_file);
    assert_eq!(
        fs::read_to_string(background_path(root.path())).unwrap(),
        "B's Background."
    );
    assert_eq!(joined.view.background, "B's Background.");
}

#[test]
fn a_join_that_fills_an_empty_folder_with_an_empty_background_writes_no_file() {
    let old = tempfile::tempdir().unwrap();
    for background in ["", " \n\t"] {
        let root = tempfile::tempdir().unwrap();
        let cache = with_background(cache_of_b(old.path(), Vec::new()), background);
        let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
        assert!(joined.written.settings_file);
        assert!(!joined.written.background_file);
        assert!(!background_path(root.path()).exists());
    }
}

#[test]
fn a_change_of_folder_writes_the_cached_background_into_a_folder_with_no_file() {
    let root = folder_of_a();
    let old = tempfile::tempdir().unwrap();
    let cache = with_background(cache_of_b(old.path(), Vec::new()), "B's Background.");
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(!joined.written.settings_file);
    assert!(joined.written.background_file);
    assert_eq!(
        fs::read_to_string(background_path(root.path())).unwrap(),
        "B's Background."
    );
    assert_eq!(joined.view.background, "B's Background.");
    assert_eq!(joined.cache.unwrap().background, "B's Background.");
    // A second join writes nothing.
    let before = snapshot(root.path());
    let again = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(!again.written.background_file);
    assert_eq!(snapshot(root.path()), before);
}

#[test]
fn a_change_of_folder_keeps_the_background_file_of_the_folder() {
    let root = folder_of_a();
    fs::write(background_path(root.path()), "A's Background.").unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = with_background(cache_of_b(old.path(), Vec::new()), "B's Background.");
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(!joined.written.background_file);
    assert_eq!(
        fs::read_to_string(background_path(root.path())).unwrap(),
        "A's Background."
    );
    assert_eq!(joined.view.background, "A's Background.");
}

#[test]
fn a_change_of_folder_keeps_an_empty_background_file_of_the_folder() {
    // A user who clears the text saves an empty file. A join does not
    // fill it again.
    let root = folder_of_a();
    fs::write(background_path(root.path()), "").unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = with_background(cache_of_b(old.path(), Vec::new()), "B's Background.");
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(!joined.written.background_file);
    assert_eq!(fs::read(background_path(root.path())).unwrap(), b"");
    assert_eq!(joined.view.background, "");
}

#[test]
fn a_change_of_folder_never_replaces_a_background_file_that_cannot_be_used() {
    let root = folder_of_a();
    fs::write(background_path(root.path()), [0xff, 0xfe]).unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = with_background(cache_of_b(old.path(), Vec::new()), "B's Background.");
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(!joined.written.background_file);
    assert_eq!(
        fs::read(background_path(root.path())).unwrap(),
        [0xff, 0xfe]
    );
    assert!(!joined.view.background_writable());
}

#[test]
fn a_join_into_a_folder_whose_settings_do_not_parse_writes_no_background() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join(".longhand")).unwrap();
    fs::write(root.path().join(".longhand").join("settings.json"), "{").unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = with_background(cache_of_b(old.path(), Vec::new()), "B's Background.");
    let before = snapshot(root.path());
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(!joined.written.background_file);
    assert_eq!(snapshot(root.path()), before);
    assert_eq!(joined.view.background, "B's Background.");
    assert!(!joined.view.background_writable());
}

#[test]
fn a_conflict_copy_of_the_background_does_not_stop_the_join() {
    let root = folder_of_a();
    fs::write(
        root.path().join(".longhand").join("background 2.md"),
        "A copy.",
    )
    .unwrap();
    let old = tempfile::tempdir().unwrap();
    let cache = with_background(cache_of_b(old.path(), Vec::new()), "B's Background.");
    let joined = join(root.path(), Some(&cache), JoinKind::ChangeOfFolder).unwrap();
    assert!(joined.written.background_file);
    assert_eq!(joined.view.background, "B's Background.");
    assert_eq!(
        fs::read_to_string(root.path().join(".longhand").join("background 2.md")).unwrap(),
        "A copy."
    );
}
