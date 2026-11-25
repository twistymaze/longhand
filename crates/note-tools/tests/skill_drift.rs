//! List notes and the skill's `list_notes.py` agree on each note of the
//! test storage folder (KTD6): the busy flag, the type, the tag keys and
//! the title.
//!
//! The script needs Python. The test uses `uv run --no-project python`, or
//! `python3` when uv is not installed. With neither, the test prints why it
//! does not run and passes, except when `CI` is set: then it fails.

mod common;

use std::collections::BTreeSet;
use std::process::Command;

use chrono::Local;
use common::{DESIGN, storage};
use note_tools::list_notes::{ListNotesInput, tag_key};
use note_tools::{ListNotes, Tool};
use serde_json::Value;

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
    assert!(
        std::env::var_os("CI").is_none(),
        "the skill drift test needs uv or python3 in CI"
    );
    eprintln!("skill_drift: skipped, because neither uv nor python3 is installed");
    None
}

#[test]
fn list_notes_and_the_skill_script_agree_on_each_note() {
    let Some(python) = python() else {
        return;
    };
    let dir = storage();
    session_save::in_use_marker::write_marker(
        &dir.path().join(DESIGN),
        "run-on-another-computer",
        Local::now().fixed_offset(),
    )
    .unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/longhand/skills/longhand-notes/scripts/list_notes.py");
    let out = Command::new(python[0])
        .args(&python[1..])
        .arg(&script)
        .arg("--root")
        .arg(dir.path())
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let skill: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();

    let rust = ListNotes::default()
        .call(dir.path(), ListNotesInput::default())
        .unwrap()
        .notes;
    assert!(!rust.is_empty());
    for note in &rust {
        let other = skill
            .iter()
            .find(|s| s["note_id"] == note.note_id.as_str())
            .unwrap_or_else(|| panic!("the skill does not list {}", note.note_id));
        assert_eq!(
            other["busy"].as_array().map(|b| !b.is_empty()),
            Some(note.busy),
            "{}",
            note.note_id
        );
        assert_eq!(other["type"], note.note_type.as_str(), "{}", note.note_id);
        assert_eq!(
            other["title"].as_str(),
            note.title.as_deref(),
            "{}",
            note.note_id
        );
        let keys = |tags: Vec<String>| tags.iter().map(|t| tag_key(t)).collect::<BTreeSet<_>>();
        let skill_tags: Vec<String> = other["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            keys(skill_tags),
            keys(note.tags.clone()),
            "{}",
            note.note_id
        );
    }
}
