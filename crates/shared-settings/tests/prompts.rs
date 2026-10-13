//! Prompt files (U2): each custom prompt is a markdown file with its id in
//! the file, and a sync conflict copy shows as one more prompt (R14, KTD4,
//! KTD5).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use shared_settings::file::{SharedValue, Unavailable, default_settings, save_settings};
use shared_settings::prompts::{
    PromptError, PromptScan, TITLE_MARKER, create_prompt, create_prompt_with_title, delete_prompt,
    free_prompt_name, rename_prompt, render_prompt_file, render_prompt_file_with_title,
    repair_prompts, safe_file_name, scan_prompts, set_prompt_text, set_prompt_title_text,
};
use summarize::prompts::{PROMPT_TEXT_LIMIT, TITLE_TEXT_LIMIT};

/// A storage folder with the default shared settings.
fn folder() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let longhand = root.path().join(".longhand");
    fs::create_dir_all(longhand.join("prompts")).unwrap();
    fs::write(
        longhand.join("settings.json"),
        serde_json::to_vec(&default_settings()).unwrap(),
    )
    .unwrap();
    root
}

fn prompts_dir(root: &Path) -> PathBuf {
    root.join(".longhand").join("prompts")
}

fn scan(root: &Path) -> PromptScan {
    scan_prompts(root).unwrap()
}

fn file_names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(prompts_dir(root))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn write_file(root: &Path, name: &str, contents: &str) {
    fs::write(prompts_dir(root).join(name), contents).unwrap();
}

#[test]
fn a_new_prompt_is_one_file_that_reads_back() {
    let root = folder();
    let made = create_prompt(root.path(), "Standup", "List what each person did.").unwrap();
    assert_eq!(file_names(root.path()), vec!["Standup.md"]);
    let scan = scan(root.path());
    assert_eq!(scan.prompts.len(), 1);
    let prompt = &scan.prompts[0];
    assert_eq!(prompt.id, made.id);
    assert_eq!(prompt.name, "Standup");
    assert_eq!(prompt.text, "List what each person did.");
    assert!(scan.conflicted.is_empty());
}

#[test]
fn a_name_with_a_colon_gets_a_valid_windows_file_name_and_shows_as_typed() {
    let root = folder();
    create_prompt(root.path(), "1:1 notes", "Text.").unwrap();
    let names = file_names(root.path());
    assert_eq!(names.len(), 1);
    let bad = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    assert!(!names[0].contains(bad), "{}", names[0]);
    assert_eq!(scan(root.path()).prompts[0].name, "1:1 notes");
}

#[test]
fn a_safe_file_name_avoids_reserved_names_and_trailing_dots() {
    assert_eq!(safe_file_name("Recap"), "Recap.md");
    assert_eq!(safe_file_name("a/b\\c"), "a-b-c.md");
    assert_eq!(safe_file_name("Notes. . "), "Notes.md");
    assert_eq!(safe_file_name("CON"), "_CON.md");
    assert_eq!(safe_file_name("com1"), "_com1.md");
    assert_eq!(safe_file_name("lpt9.draft"), "_lpt9.draft.md");
    assert_eq!(safe_file_name("Console"), "Console.md");
    // A file name that the scan skips is never made.
    assert_eq!(safe_file_name(".hidden"), "hidden.md");
    assert_eq!(safe_file_name("~$lock"), "~-lock.md");
    assert_eq!(safe_file_name("   "), "Prompt.md");
    assert_eq!(safe_file_name("tab\there"), "tab-here.md");
    let long = "x".repeat(400);
    assert!(safe_file_name(&long).len() <= 200);
}

#[test]
fn two_names_with_one_file_name_get_two_files() {
    let root = folder();
    let a = create_prompt(root.path(), "a/b", "One.").unwrap();
    let b = create_prompt(root.path(), "a:b", "Two.").unwrap();
    assert_eq!(file_names(root.path()), vec!["a-b 2.md", "a-b.md"]);
    let scan = scan(root.path());
    assert_eq!(scan.get(&a.id).unwrap().name, "a/b");
    assert_eq!(scan.get(&b.id).unwrap().name, "a:b");
}

#[test]
fn a_new_prompt_gets_a_free_name() {
    let root = folder();
    create_prompt(root.path(), "New prompt", "One.").unwrap();
    let second = create_prompt(root.path(), "New prompt", "Two.").unwrap();
    assert_eq!(second.name, "New prompt 2");
    // A built-in name is taken too.
    let third = create_prompt(root.path(), "Topic summary", "Three.").unwrap();
    assert_eq!(third.name, "Topic summary 2");
}

#[test]
fn free_prompt_name_compares_names_trimmed_and_without_case() {
    assert_eq!(free_prompt_name("Recap", ["other"].into_iter()), "Recap");
    assert_eq!(
        free_prompt_name("Recap", [" recap "].into_iter()),
        "Recap 2"
    );
    assert_eq!(
        free_prompt_name("Recap", ["Recap", "RECAP 2"].into_iter()),
        "Recap 3"
    );
}

#[test]
fn a_file_renamed_outside_longhand_keeps_its_id_and_name() {
    let root = folder();
    let made = create_prompt(root.path(), "Standup", "Text.").unwrap();
    fs::rename(
        prompts_dir(root.path()).join("Standup.md"),
        prompts_dir(root.path()).join("Daily.md"),
    )
    .unwrap();
    let scan = scan(root.path());
    assert_eq!(scan.prompts.len(), 1);
    assert_eq!(scan.prompts[0].id, made.id);
    assert_eq!(scan.prompts[0].name, "Standup");
    assert_eq!(scan.prompts[0].file_name, "Daily.md");
}

#[test]
fn a_file_with_no_front_matter_is_a_prompt_named_for_its_file() {
    let root = folder();
    write_file(root.path(), "Weekly review.md", "Summarize the week.\n");
    let first = scan(root.path());
    assert_eq!(first.prompts.len(), 1);
    assert_eq!(first.prompts[0].name, "Weekly review");
    assert_eq!(first.prompts[0].text, "Summarize the week.");
    // The id comes from the file name, so each scan gives the same id.
    assert_eq!(scan(root.path()).prompts[0].id, first.prompts[0].id);
    // The scan wrote nothing.
    assert_eq!(
        fs::read_to_string(prompts_dir(root.path()).join("Weekly review.md")).unwrap(),
        "Summarize the week.\n"
    );
}

#[test]
fn a_change_of_a_file_with_no_front_matter_writes_its_id_first() {
    let root = folder();
    write_file(root.path(), "Weekly.md", "Summarize the week.");
    let id = scan(root.path()).prompts[0].id.clone();
    rename_prompt(root.path(), &id, "Monthly").unwrap();
    assert_eq!(file_names(root.path()), vec!["Monthly.md"]);
    let after = scan(root.path());
    assert_eq!(after.prompts[0].id, id);
    assert_eq!(after.prompts[0].name, "Monthly");
    assert_eq!(after.prompts[0].text, "Summarize the week.");
}

#[test]
fn two_files_with_one_id_and_different_texts_show_as_two_prompts() {
    // AE8: A and B changed one prompt offline, and the sync made a copy.
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Text from A.").unwrap();
    write_file(
        root.path(),
        "Recap 2.md",
        &render_prompt_file(&made.id, "Recap", "Text from B."),
    );
    let scan = scan(root.path());
    assert_eq!(scan.prompts.len(), 2);
    assert!(scan.conflicted.contains(&made.id));
    let original = scan.get(&made.id).unwrap();
    assert_eq!(original.text, "Text from A.");
    assert_eq!(original.file_name, "Recap.md");
    assert_eq!(original.name, "Recap");
    let copy = scan.prompts.iter().find(|p| p.id != made.id).unwrap();
    assert_eq!(copy.text, "Text from B.");
    assert_eq!(copy.file_name, "Recap 2.md");
    // A shown name that another prompt has gets a free name for display.
    assert_eq!(copy.name, "Recap 2");
    assert_eq!(copy.stored_name, "Recap");
}

#[test]
fn two_computers_give_each_file_of_a_conflict_the_same_id() {
    let a = folder();
    let made = create_prompt(a.path(), "Recap", "Text from A.").unwrap();
    write_file(
        a.path(),
        "Recap (conflict).md",
        &render_prompt_file(&made.id, "Recap", "Text from B."),
    );
    // The second computer has a copy of the same files.
    let b = folder();
    for name in file_names(a.path()) {
        fs::copy(
            prompts_dir(a.path()).join(&name),
            prompts_dir(b.path()).join(&name),
        )
        .unwrap();
    }
    let ids = |root: &Path| {
        let mut ids: Vec<(String, String)> = scan(root)
            .prompts
            .into_iter()
            .map(|p| (p.file_name, p.id))
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(a.path()), ids(b.path()));
    // The file whose name matches its prompt name keeps the id, although
    // the copy sorts first.
    assert_eq!(scan(a.path()).get(&made.id).unwrap().file_name, "Recap.md");
}

#[test]
fn when_no_file_matches_its_name_the_first_file_name_keeps_the_id() {
    let root = folder();
    let id = "4d1b7a4e-1111-4222-8333-444455556666";
    write_file(
        root.path(),
        "b.md",
        &render_prompt_file(id, "Recap", "B text."),
    );
    write_file(
        root.path(),
        "a.md",
        &render_prompt_file(id, "Recap", "A text."),
    );
    let scan = scan(root.path());
    assert_eq!(scan.get(id).unwrap().file_name, "a.md");
}

#[test]
fn two_files_with_one_id_and_the_same_text_show_as_one_prompt() {
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Same text.").unwrap();
    write_file(
        root.path(),
        "Recap 2.md",
        &render_prompt_file(&made.id, "Recap", "Same text."),
    );
    let scan = scan(root.path());
    assert_eq!(scan.prompts.len(), 1);
    assert!(scan.conflicted.is_empty());
    assert_eq!(scan.prompts[0].file_name, "Recap.md");
    assert_eq!(scan.prompts[0].duplicates, vec!["Recap 2.md".to_string()]);
}

#[test]
fn a_delete_removes_each_file_of_the_prompt() {
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Same text.").unwrap();
    write_file(
        root.path(),
        "Recap 2.md",
        &render_prompt_file(&made.id, "Recap", "Same text."),
    );
    create_prompt(root.path(), "Other", "Other text.").unwrap();
    delete_prompt(root.path(), &made.id).unwrap();
    assert_eq!(file_names(root.path()), vec!["Other.md"]);
    assert!(matches!(
        delete_prompt(root.path(), &made.id),
        Err(PromptError::NoSuchPrompt)
    ));
}

#[test]
fn a_delete_of_the_original_keeps_the_id_of_each_conflict_copy() {
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Text from A.").unwrap();
    write_file(
        root.path(),
        "Recap 2.md",
        &render_prompt_file(&made.id, "Recap", "Text from B."),
    );
    let copy_id = scan(root.path())
        .prompts
        .into_iter()
        .find(|p| p.id != made.id)
        .unwrap()
        .id;
    delete_prompt(root.path(), &made.id).unwrap();
    let after = scan(root.path());
    assert_eq!(after.prompts.len(), 1);
    assert_eq!(after.prompts[0].id, copy_id);
    assert_eq!(after.prompts[0].text, "Text from B.");
    assert!(after.get(&made.id).is_none());
}

#[test]
fn a_change_of_text_writes_one_file() {
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Old text.").unwrap();
    let before = fs::read_to_string(prompts_dir(root.path()).join("Recap.md")).unwrap();
    set_prompt_text(root.path(), &made.id, "New text.").unwrap();
    assert_eq!(file_names(root.path()), vec!["Recap.md"]);
    let after = fs::read_to_string(prompts_dir(root.path()).join("Recap.md")).unwrap();
    assert_ne!(before, after);
    let prompt = scan(root.path()).prompts.remove(0);
    assert_eq!(prompt.id, made.id);
    assert_eq!(prompt.text, "New text.");
}

#[test]
fn a_text_keeps_its_leading_and_trailing_lines() {
    let root = folder();
    let text = "\n# Heading\n\nBody.\n\n";
    let made = create_prompt(root.path(), "Spacing", text).unwrap();
    assert_eq!(scan(root.path()).get(&made.id).unwrap().text, text);
}

#[test]
fn a_rename_writes_the_name_into_the_file_and_renames_it() {
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Text.").unwrap();
    rename_prompt(root.path(), &made.id, "  Standup  ").unwrap();
    assert_eq!(file_names(root.path()), vec!["Standup.md"]);
    let prompt = scan(root.path()).prompts.remove(0);
    assert_eq!(prompt.id, made.id);
    assert_eq!(prompt.name, "Standup");
    assert_eq!(prompt.text, "Text.");
}

#[test]
fn a_rename_that_changes_only_the_case_keeps_one_file() {
    let root = folder();
    let made = create_prompt(root.path(), "recap", "Text.").unwrap();
    rename_prompt(root.path(), &made.id, "Recap").unwrap();
    assert_eq!(file_names(root.path()), vec!["Recap.md"]);
    assert_eq!(scan(root.path()).prompts[0].name, "Recap");
}

#[test]
fn a_crash_during_a_rename_leaves_one_file_with_the_id() {
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Text.").unwrap();
    // The first step of a rename writes the new name into the old file.
    write_file(
        root.path(),
        "Recap.md",
        &render_prompt_file(&made.id, "Standup", "Text."),
    );
    let scan_after_crash = scan(root.path());
    assert_eq!(scan_after_crash.prompts.len(), 1);
    assert_eq!(scan_after_crash.prompts[0].id, made.id);
    assert_eq!(scan_after_crash.prompts[0].name, "Standup");
    // A rename again finishes the second step.
    rename_prompt(root.path(), &made.id, "Standup").unwrap();
    assert_eq!(file_names(root.path()), vec!["Standup.md"]);
}

#[test]
fn a_rename_of_a_conflict_copy_keeps_its_made_id() {
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Text from A.").unwrap();
    write_file(
        root.path(),
        "Recap 2.md",
        &render_prompt_file(&made.id, "Recap", "Text from B."),
    );
    let copy_id = scan(root.path())
        .prompts
        .into_iter()
        .find(|p| p.id != made.id)
        .unwrap()
        .id;
    rename_prompt(root.path(), &copy_id, "Recap from B").unwrap();
    let after = scan(root.path());
    assert_eq!(after.prompts.len(), 2);
    assert!(after.conflicted.is_empty());
    assert_eq!(after.get(&copy_id).unwrap().name, "Recap from B");
    assert_eq!(after.get(&copy_id).unwrap().file_name, "Recap from B.md");
    assert_eq!(after.get(&made.id).unwrap().text, "Text from A.");
}

#[test]
fn a_rename_to_a_taken_name_is_refused() {
    let root = folder();
    create_prompt(root.path(), "Recap", "One.").unwrap();
    let other = create_prompt(root.path(), "Other", "Two.").unwrap();
    assert!(matches!(
        rename_prompt(root.path(), &other.id, " recap "),
        Err(PromptError::NameTaken)
    ));
    assert!(matches!(
        rename_prompt(root.path(), &other.id, "Topic summary"),
        Err(PromptError::NameTaken)
    ));
    assert!(matches!(
        rename_prompt(root.path(), &other.id, "   "),
        Err(PromptError::EmptyName)
    ));
    assert_eq!(file_names(root.path()), vec!["Other.md", "Recap.md"]);
}

#[test]
fn a_text_that_is_empty_or_too_long_is_refused() {
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Text.").unwrap();
    assert!(matches!(
        set_prompt_text(root.path(), &made.id, "  \n"),
        Err(PromptError::EmptyText)
    ));
    let long = "x".repeat(summarize::prompts::PROMPT_TEXT_LIMIT + 1);
    let refused = set_prompt_text(root.path(), &made.id, &long).unwrap_err();
    assert!(matches!(refused, PromptError::TextTooLong));
    assert_eq!(
        refused.to_string(),
        "The prompt text is too long. Use 4,000 characters or fewer."
    );
    assert!(matches!(
        create_prompt(root.path(), "Other", ""),
        Err(PromptError::EmptyText)
    ));
}

#[test]
fn a_built_in_prompt_cannot_be_changed() {
    let root = folder();
    let id = summarize::prompts::TOPIC_SUMMARY_ID;
    assert!(matches!(
        rename_prompt(root.path(), id, "Mine"),
        Err(PromptError::BuiltIn)
    ));
    assert!(matches!(
        set_prompt_text(root.path(), id, "Mine"),
        Err(PromptError::BuiltIn)
    ));
    assert!(matches!(
        delete_prompt(root.path(), id),
        Err(PromptError::BuiltIn)
    ));
}

#[test]
fn a_file_that_names_a_built_in_id_does_not_replace_the_built_in_prompt() {
    let root = folder();
    write_file(
        root.path(),
        "Mine.md",
        &render_prompt_file(summarize::prompts::TOPIC_SUMMARY_ID, "Mine", "Text."),
    );
    let scan = scan(root.path());
    assert_eq!(scan.prompts.len(), 1);
    assert_ne!(scan.prompts[0].id, summarize::prompts::TOPIC_SUMMARY_ID);
}

#[test]
fn files_that_are_not_prompts_are_skipped() {
    let root = folder();
    create_prompt(root.path(), "Recap", "Text.").unwrap();
    write_file(root.path(), "~$Recap.md", "Lock file.");
    write_file(root.path(), ".Recap.md.0123456789abcdef.tmp", "Partial.");
    write_file(root.path(), ".Recap.md", "Hidden.");
    write_file(root.path(), "notes.txt", "Not markdown.");
    fs::create_dir(prompts_dir(root.path()).join("Folder.md")).unwrap();
    let scan = scan(root.path());
    let names: Vec<&str> = scan.prompts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["Recap"]);
}

#[test]
fn a_file_with_no_text_is_not_a_prompt_and_is_not_deleted() {
    let root = folder();
    write_file(root.path(), "Empty.md", "");
    write_file(
        root.path(),
        "Blank.md",
        &render_prompt_file("4d1b7a4e-1111-4222-8333-444455556666", "Blank", "  \n "),
    );
    assert!(scan(root.path()).prompts.is_empty());
    create_prompt(root.path(), "Recap", "Text.").unwrap();
    assert_eq!(
        file_names(root.path()),
        vec!["Blank.md", "Empty.md", "Recap.md"]
    );
}

#[test]
fn a_file_with_no_closing_front_matter_line_is_all_text() {
    let root = folder();
    write_file(root.path(), "Odd.md", "---\nid: x\nText with no end line.");
    let scan = scan(root.path());
    assert_eq!(scan.prompts.len(), 1);
    assert_eq!(scan.prompts[0].name, "Odd");
    assert_eq!(scan.prompts[0].text, "---\nid: x\nText with no end line.");
}

#[test]
fn front_matter_with_crlf_lines_and_extra_keys_parses() {
    let root = folder();
    write_file(
        root.path(),
        "Win.md",
        "---\r\nid: abc\r\nname: Windows prompt\r\ncolour: blue\r\n---\r\n\r\nBody.\r\n",
    );
    let scan = scan(root.path());
    assert_eq!(scan.prompts[0].id, "abc");
    assert_eq!(scan.prompts[0].name, "Windows prompt");
    assert_eq!(scan.prompts[0].text, "Body.");
    // A change keeps the key that this build does not know.
    set_prompt_text(root.path(), "abc", "New body.").unwrap();
    let contents = fs::read_to_string(prompts_dir(root.path()).join("Win.md")).unwrap();
    assert!(contents.contains("colour: blue"), "{contents}");
}

#[test]
fn a_folder_with_no_prompts_folder_has_no_prompts() {
    let root = tempfile::tempdir().unwrap();
    assert!(scan(root.path()).prompts.is_empty());
}

#[test]
fn a_folder_that_does_not_exist_has_no_prompts_and_refuses_changes() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("gone");
    assert!(matches!(
        scan_prompts(&root),
        Err(Unavailable::FolderMissing)
    ));
    assert!(matches!(
        create_prompt(&root, "Recap", "Text."),
        Err(PromptError::NotAvailable(Unavailable::FolderMissing))
    ));
    assert!(!root.exists());
}

#[test]
fn a_settings_file_that_does_not_parse_stays_and_the_prompts_still_change() {
    // R22: a change of a prompt does not write the settings file.
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Text.").unwrap();
    let settings = root.path().join(".longhand").join("settings.json");
    fs::write(&settings, "{\"sche").unwrap();
    set_prompt_text(root.path(), &made.id, "New.").unwrap();
    assert_eq!(scan(root.path()).get(&made.id).unwrap().text, "New.");
    assert_eq!(delete_prompt(root.path(), &made.id).unwrap(), None);
    assert!(file_names(root.path()).is_empty());
    assert_eq!(fs::read_to_string(&settings).unwrap(), "{\"sche");
}

#[test]
fn a_rename_of_the_original_keeps_the_id_of_each_conflict_copy() {
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Text from A.").unwrap();
    write_file(
        root.path(),
        "Recap (conflict).md",
        &render_prompt_file(&made.id, "Recap", "Text from B."),
    );
    let copy_id = scan(root.path())
        .prompts
        .into_iter()
        .find(|p| p.id != made.id)
        .unwrap()
        .id;
    // The copy shows as "Recap 2", so the original can keep "Recap".
    rename_prompt(root.path(), &made.id, "recap").unwrap();
    rename_prompt(root.path(), &made.id, "Weekly recap").unwrap();
    let after = scan(root.path());
    assert!(after.conflicted.is_empty());
    assert_eq!(after.get(&made.id).unwrap().text, "Text from A.");
    assert_eq!(after.get(&made.id).unwrap().name, "Weekly recap");
    assert_eq!(after.get(&copy_id).unwrap().text, "Text from B.");
}

// The title text (title instructions U2, KTD8, KTD9).

/// The contents of the prompt file `name`.
fn read_file(root: &Path, name: &str) -> String {
    fs::read_to_string(prompts_dir(root).join(name)).unwrap()
}

#[test]
fn a_file_with_a_title_text_reads_both_texts_and_renders_the_same_bytes() {
    let root = folder();
    let contents = render_prompt_file_with_title("abc", "Recap", "Summary text.", "Title text.");
    assert_eq!(
        contents,
        format!(
            "---\nid: abc\nname: \"Recap\"\n---\n\nSummary text.\n{TITLE_MARKER}\nTitle text.\n"
        )
    );
    write_file(root.path(), "Recap.md", &contents);
    let prompt = scan(root.path()).prompts.remove(0);
    assert_eq!(prompt.text, "Summary text.");
    assert_eq!(prompt.title_text, "Title text.");
    let again = render_prompt_file_with_title(
        &prompt.id,
        &prompt.stored_name,
        &prompt.text,
        &prompt.title_text,
    );
    assert_eq!(again, contents);
    let converted = summarize::Prompt::from(&prompt);
    assert_eq!(converted.title_text, "Title text.");
}

#[test]
fn a_file_with_no_marker_has_an_empty_title_text() {
    let root = folder();
    write_file(
        root.path(),
        "Recap.md",
        &render_prompt_file("abc", "Recap", "Summary text."),
    );
    write_file(root.path(), "Plain.md", "Just a summary text.\n");
    let scanned = scan(root.path());
    assert_eq!(scanned.prompts.len(), 2);
    for prompt in scanned.prompts {
        assert_eq!(prompt.title_text, "", "{}", prompt.file_name);
    }
    // An empty title text writes no marker.
    assert_eq!(
        render_prompt_file_with_title("abc", "Recap", "Summary text.", ""),
        render_prompt_file("abc", "Recap", "Summary text."),
    );
    assert!(!render_prompt_file("abc", "Recap", "Summary text.").contains(TITLE_MARKER));
}

#[test]
fn a_title_text_with_headings_and_blank_lines_survives_a_round_trip() {
    let root = folder();
    let text = "\n# Summary\n\nBody.\n\n";
    let title = "\n# Title\n\nUse the meeting type.\n\n- One\n- Two\n\n";
    let made = create_prompt_with_title(root.path(), "Spacing", text, title).unwrap();
    assert_eq!(made.title_text, title);
    let prompt = scan(root.path()).get(&made.id).unwrap().clone();
    assert_eq!(prompt.text, text);
    assert_eq!(prompt.title_text, title);
}

#[test]
fn a_file_with_crlf_lines_and_a_marker_reads_both_texts() {
    let root = folder();
    write_file(
        root.path(),
        "Win.md",
        &format!("---\r\nid: abc\r\nname: Win\r\n---\r\n\r\nBody.\r\n{TITLE_MARKER}\r\nTitle.\r\n"),
    );
    let prompt = scan(root.path()).prompts.remove(0);
    assert_eq!(prompt.text, "Body.");
    assert_eq!(prompt.title_text, "Title.");
}

#[test]
fn a_change_of_the_title_text_changes_only_the_part_after_the_marker() {
    let root = folder();
    write_file(
        root.path(),
        "Recap.md",
        &format!(
            "---\nid: abc\nname: \"Recap\"\ncolour: blue\n---\n\nSummary text.\n{TITLE_MARKER}\nOld title.\n"
        ),
    );
    let before = read_file(root.path(), "Recap.md");
    let changed = set_prompt_title_text(root.path(), "abc", "New title.").unwrap();
    assert_eq!(changed.title_text, "New title.");
    assert_eq!(changed.text, "Summary text.");
    let after = read_file(root.path(), "Recap.md");
    assert_eq!(after, before.replace("Old title.", "New title."));
    assert_eq!(file_names(root.path()), vec!["Recap.md"]);
    // An empty title text removes the marker and keeps the rest.
    set_prompt_title_text(root.path(), "abc", "").unwrap();
    assert_eq!(
        read_file(root.path(), "Recap.md"),
        "---\nid: abc\nname: \"Recap\"\ncolour: blue\n---\n\nSummary text.\n"
    );
    assert_eq!(scan(root.path()).prompts[0].title_text, "");
}

#[test]
fn a_change_of_the_summary_text_or_the_name_keeps_the_title_text() {
    let root = folder();
    let made = create_prompt_with_title(root.path(), "Recap", "Old.", "Title.").unwrap();
    let changed = set_prompt_text(root.path(), &made.id, "New.").unwrap();
    assert_eq!(changed.title_text, "Title.");
    let renamed = rename_prompt(root.path(), &made.id, "Weekly").unwrap();
    assert_eq!(renamed.title_text, "Title.");
    let prompt = scan(root.path()).get(&made.id).unwrap().clone();
    assert_eq!(prompt.name, "Weekly");
    assert_eq!(prompt.text, "New.");
    assert_eq!(prompt.title_text, "Title.");
}

#[test]
fn a_title_text_over_the_limit_is_refused_and_an_empty_one_is_accepted() {
    let root = folder();
    let made = create_prompt_with_title(root.path(), "Recap", "Text.", "").unwrap();
    assert_eq!(made.title_text, "");
    let long = "x".repeat(TITLE_TEXT_LIMIT + 1);
    let refused = set_prompt_title_text(root.path(), &made.id, &long).unwrap_err();
    assert!(
        matches!(refused, PromptError::TitleTextTooLong),
        "{refused}"
    );
    assert_eq!(
        refused.to_string(),
        "The title instructions are too long. Use 1,000 characters or fewer."
    );
    assert!(matches!(
        create_prompt_with_title(root.path(), "Other", "Text.", &long),
        Err(PromptError::TitleTextTooLong)
    ));
    set_prompt_title_text(root.path(), &made.id, &"x".repeat(TITLE_TEXT_LIMIT)).unwrap();
    assert_eq!(
        scan(root.path()).get(&made.id).unwrap().title_text.len(),
        TITLE_TEXT_LIMIT
    );
    set_prompt_title_text(root.path(), &made.id, "").unwrap();
    assert_eq!(scan(root.path()).get(&made.id).unwrap().title_text, "");
}

#[test]
fn a_built_in_title_text_cannot_be_changed() {
    let root = folder();
    assert!(matches!(
        set_prompt_title_text(root.path(), summarize::prompts::TOPIC_SUMMARY_ID, "Mine"),
        Err(PromptError::BuiltIn)
    ));
}

#[test]
fn a_summary_text_with_the_marker_line_is_refused() {
    let root = folder();
    let text = format!("Summary.\n{TITLE_MARKER}\nMore.");
    assert!(matches!(
        create_prompt(root.path(), "Recap", &text),
        Err(PromptError::TextHasMarker)
    ));
    let made = create_prompt(root.path(), "Recap", "Summary.").unwrap();
    let refused =
        set_prompt_text(root.path(), &made.id, &format!("A.\n  {TITLE_MARKER}  ")).unwrap_err();
    assert!(matches!(refused, PromptError::TextHasMarker), "{refused}");
    assert_eq!(scan(root.path()).get(&made.id).unwrap().text, "Summary.");
    // The marker inside a line is only text.
    let inline = format!("Keep {TITLE_MARKER} here.");
    set_prompt_text(root.path(), &made.id, &inline).unwrap();
    let prompt = scan(root.path()).get(&made.id).unwrap().clone();
    assert_eq!(prompt.text, inline);
    assert_eq!(prompt.title_text, "");
}

#[test]
fn the_longest_texts_of_four_byte_characters_fit_and_round_trip() {
    let root = folder();
    let text = "😀".repeat(PROMPT_TEXT_LIMIT);
    let title = "😀".repeat(TITLE_TEXT_LIMIT);
    let made = create_prompt_with_title(root.path(), &"n".repeat(150), &text, &title).unwrap();
    let prompt = scan(root.path()).get(&made.id).unwrap().clone();
    assert_eq!(prompt.text, text);
    assert_eq!(prompt.title_text, title);
}

#[test]
fn two_files_with_one_id_and_one_summary_text_but_two_title_texts_are_a_conflict() {
    let root = folder();
    let made = create_prompt_with_title(root.path(), "Recap", "Same.", "Title from A.").unwrap();
    write_file(
        root.path(),
        "Recap 2.md",
        &render_prompt_file_with_title(&made.id, "Recap", "Same.", "Title from B."),
    );
    let scanned = scan(root.path());
    assert_eq!(scanned.prompts.len(), 2);
    assert!(scanned.conflicted.contains(&made.id));
    assert_eq!(scanned.get(&made.id).unwrap().title_text, "Title from A.");
    let copy = scanned.prompts.iter().find(|p| p.id != made.id).unwrap();
    assert_eq!(copy.title_text, "Title from B.");
    assert!(copy.duplicates.is_empty());
    // A change of the original writes the made id into the copy and keeps
    // the copy's title text.
    let copy_id = copy.id.clone();
    set_prompt_title_text(root.path(), &made.id, "New title from A.").unwrap();
    let after = scan(root.path());
    assert!(after.conflicted.is_empty());
    assert_eq!(after.get(&made.id).unwrap().title_text, "New title from A.");
    assert_eq!(after.get(&copy_id).unwrap().title_text, "Title from B.");
}

#[test]
fn a_file_with_a_title_text_over_the_limit_is_listed_as_over_the_limit() {
    // R21: an outside edit made the title instructions too long.
    let root = folder();
    let title = "x".repeat(TITLE_TEXT_LIMIT + 1);
    write_file(
        root.path(),
        "Long.md",
        &render_prompt_file_with_title("abc", "Long", "Text.", &title),
    );
    let scan = scan(root.path());
    let prompt = scan.get("abc").unwrap();
    assert!(prompt.over_limit());
    assert!(!prompt.too_large);
    assert_eq!(prompt.title_text, title);
}

// Outside edits of the prompt files (U6 of the safe agent edits plan: R17
// to R23, KTD8).

#[test]
fn a_prompt_text_over_the_limit_is_listed_with_its_whole_text() {
    // R21: a 5,000-character prompt from an outside edit.
    let root = folder();
    let text = "y".repeat(5000);
    write_file(
        root.path(),
        "Long.md",
        &render_prompt_file("long-id", "Long", &text),
    );
    let scan = scan(root.path());
    let prompt = scan.get("long-id").unwrap();
    assert!(prompt.over_limit());
    assert_eq!(prompt.text, text);
    let made = create_prompt(root.path(), "Short", "Short text.").unwrap();
    assert!(
        !scan_prompts(root.path())
            .unwrap()
            .get(&made.id)
            .unwrap()
            .over_limit()
    );
}

#[test]
fn a_file_over_the_size_limit_is_listed_with_no_text_and_can_be_renamed() {
    let root = folder();
    let text = "z".repeat(60_000);
    write_file(
        root.path(),
        "Huge.md",
        &render_prompt_file("huge-id", "Huge", &text),
    );
    let scan = scan(root.path());
    let prompt = scan.get("huge-id").unwrap();
    assert!(prompt.too_large);
    assert!(prompt.over_limit());
    assert_eq!(prompt.name, "Huge");
    assert_eq!(prompt.text, "");
    // A rename keeps the whole text in the file.
    rename_prompt(root.path(), "huge-id", "Huge notes").unwrap();
    assert_eq!(file_names(root.path()), vec!["Huge notes.md"]);
    assert_eq!(
        read_file(root.path(), "Huge notes.md"),
        render_prompt_file("huge-id", "Huge notes", &text)
    );
    delete_prompt(root.path(), "huge-id").unwrap();
    assert!(file_names(root.path()).is_empty());
}

#[test]
fn a_prompt_file_with_no_text_is_not_missing() {
    // KTD8: a file that a sync delivers in parts exists, so the prompt is
    // not missing.
    let root = folder();
    write_file(
        root.path(),
        "Recap.md",
        &render_prompt_file("recap-id", "Recap", " "),
    );
    let scan = scan(root.path());
    assert!(scan.get("recap-id").is_none());
    assert!(scan.unusable.contains("recap-id"));
    assert!(!scan.is_missing("recap-id"));
    assert!(scan.is_missing("other-id"));
}

/// The id that the scan gives the file `file_name` with no id in it.
fn id_of(root: &Path, file_name: &str) -> String {
    let scan = scan(root);
    let prompt = scan
        .prompts
        .iter()
        .find(|p| p.file_name == file_name)
        .unwrap();
    assert_eq!(prompt.id_in_file, None);
    prompt.id.clone()
}

#[test]
fn a_repair_writes_the_id_of_a_file_with_no_id_and_a_rename_then_keeps_it() {
    // R18.
    let root = folder();
    write_file(root.path(), "Recap.md", "Write a recap.");
    let id = id_of(root.path(), "Recap.md");
    let repaired = repair_prompts(root.path(), &BTreeSet::new()).unwrap();
    assert_eq!(repaired.ids_written, vec!["Recap.md"]);
    assert_eq!(
        read_file(root.path(), "Recap.md"),
        render_prompt_file(&id, "Recap", "Write a recap.")
    );
    fs::rename(
        prompts_dir(root.path()).join("Recap.md"),
        prompts_dir(root.path()).join("Weekly.md"),
    )
    .unwrap();
    let after = scan(root.path());
    assert_eq!(after.get(&id).unwrap().file_name, "Weekly.md");
}

#[test]
fn a_repair_gives_a_file_the_file_name_of_its_name_and_keeps_the_id() {
    // R19: an agent changed `name`.
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Text.").unwrap();
    write_file(
        root.path(),
        "Recap.md",
        &render_prompt_file(&made.id, "Weekly recap", "Text."),
    );
    assert_eq!(
        scan(root.path()).get(&made.id).unwrap().name,
        "Weekly recap"
    );
    let repaired = repair_prompts(root.path(), &BTreeSet::new()).unwrap();
    assert_eq!(
        repaired.renamed,
        vec![("Recap.md".to_string(), "Weekly recap.md".to_string())]
    );
    assert_eq!(file_names(root.path()), vec!["Weekly recap.md"]);
    assert_eq!(
        scan(root.path()).get(&made.id).unwrap().name,
        "Weekly recap"
    );
    // A second repair changes nothing.
    assert!(
        repair_prompts(root.path(), &BTreeSet::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_file_with_no_id_and_another_name_gets_its_id_before_its_rename() {
    // R18, R19: the id is the id that the settings name, also after the
    // rename.
    let root = folder();
    write_file(root.path(), "Notes.md", "---\nname: Recap\n---\n\nText.");
    let id = id_of(root.path(), "Notes.md");
    repair_prompts(root.path(), &BTreeSet::new()).unwrap();
    assert_eq!(file_names(root.path()), vec!["Recap.md"]);
    assert_eq!(scan(root.path()).get(&id).unwrap().name, "Recap");
}

#[test]
fn a_copy_with_a_new_id_and_name_is_a_second_prompt_and_the_original_stays() {
    // AE5, R17.
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Recap text.").unwrap();
    let original = read_file(root.path(), "Recap.md");
    write_file(
        root.path(),
        "Recap copy.md",
        &render_prompt_file("new-id", "Weekly", "Weekly text."),
    );
    let after = scan(root.path());
    assert_eq!(after.prompts.len(), 2);
    assert!(after.conflicted.is_empty());
    assert_eq!(after.get("new-id").unwrap().name, "Weekly");
    assert_eq!(after.get(&made.id).unwrap().text, "Recap text.");
    repair_prompts(root.path(), &BTreeSet::new()).unwrap();
    assert_eq!(file_names(root.path()), vec!["Recap.md", "Weekly.md"]);
    assert_eq!(read_file(root.path(), "Recap.md"), original);
}

#[test]
fn a_repair_leaves_a_conflict_for_the_user() {
    // R17: two files with one id and different texts.
    let root = folder();
    let made = create_prompt(root.path(), "Recap", "Text from A.").unwrap();
    write_file(
        root.path(),
        "Recap 2.md",
        &render_prompt_file(&made.id, "Recap", "Text from B."),
    );
    let before: Vec<(String, String)> = file_names(root.path())
        .into_iter()
        .map(|n| (read_file(root.path(), &n), n))
        .collect();
    assert!(
        repair_prompts(root.path(), &BTreeSet::new())
            .unwrap()
            .is_empty()
    );
    let after: Vec<(String, String)> = file_names(root.path())
        .into_iter()
        .map(|n| (read_file(root.path(), &n), n))
        .collect();
    assert_eq!(after, before);
    assert!(scan(root.path()).conflicted.contains(&made.id));
}

fn settings_on_disk(root: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(root.join(".longhand").join("settings.json")).unwrap())
        .unwrap()
}

#[test]
fn a_repair_clears_the_settings_that_name_a_missing_prompt() {
    // R20: the rules of a delete in the app.
    let root = folder();
    let kept = create_prompt(root.path(), "Kept", "Kept text.").unwrap();
    save_settings(
        root.path(),
        &[
            SharedValue::ActiveSummaryPrompt("gone-id".into()),
            SharedValue::NoteTypePrompts(
                [
                    ("Pathfinder".to_string(), "gone-id".to_string()),
                    ("Standup".to_string(), kept.id.clone()),
                ]
                .into(),
            ),
        ],
    )
    .unwrap();
    let missing: BTreeSet<String> = ["gone-id".to_string(), kept.id.clone()].into();
    let repaired = repair_prompts(root.path(), &missing).unwrap();
    assert_eq!(repaired.cleared, vec!["gone-id"]);
    let on_disk = settings_on_disk(root.path());
    assert_eq!(
        on_disk["active_summary_prompt"],
        summarize::prompts::TOPIC_SUMMARY_ID
    );
    assert_eq!(
        on_disk["note_type_prompts"],
        serde_json::json!({ "Standup": kept.id })
    );
}

#[test]
fn a_repair_keeps_the_settings_of_a_prompt_whose_file_cannot_be_used() {
    let root = folder();
    write_file(
        root.path(),
        "Recap.md",
        &render_prompt_file("recap-id", "Recap", ""),
    );
    save_settings(
        root.path(),
        &[SharedValue::ActiveSummaryPrompt("recap-id".into())],
    )
    .unwrap();
    let missing: BTreeSet<String> = ["recap-id".to_string()].into();
    assert!(repair_prompts(root.path(), &missing).unwrap().is_empty());
    assert_eq!(
        settings_on_disk(root.path())["active_summary_prompt"],
        "recap-id"
    );
}
