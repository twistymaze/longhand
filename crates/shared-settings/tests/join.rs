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
use shared_settings::prompts::{create_prompt, render_prompt_file, scan_prompts};
use shared_settings::view::{ActivePrompt, Availability, load};
use summarize::prompts::Prompt;

fn prompt(id: &str, name: &str, text: &str) -> Prompt {
    Prompt {
        id: id.into(),
        name: name.into(),
        text: text.into(),
        title_text: String::new(),
    }
}

/// B's cache: live transcript off, and the custom prompts `prompts`.
fn cache_of_b(source: &Path, prompts: Vec<Prompt>) -> Cache {
    Cache {
        source: source.to_path_buf(),
        settings: apply_patch(&default_settings(), &[SharedValue::LiveTranscript(false)]),
        prompts,
        active_prompt: None,
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
fn a_settings_file_that_names_a_prompt_that_has_not_arrived_uses_the_cached_text() {
    let root = folder_of_a();
    // B already uses this folder, and its cache has A's prompt "Standup".
    let standup = prompt("standup-a", "Standup", "A's standup.");
    let cache = Cache {
        source: root.path().to_path_buf(),
        settings: apply_patch(&default_settings(), &[]),
        prompts: vec![standup.clone()],
        active_prompt: None,
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
    assert_eq!(
        joined.view.active,
        ActivePrompt::Custom {
            prompt: standup.clone(),
            from_cache: true
        }
    );
    assert!(!joined.written.settings_file);
    assert!(joined.written.prompt_files.is_empty());
    assert_eq!(snapshot(root.path()), before);
    // The new cache keeps the active prompt's text.
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
