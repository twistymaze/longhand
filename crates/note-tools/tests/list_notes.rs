//! List notes on the test storage folder (R2, R3, R7, AE1, AE4, AE6).

mod common;

use std::fs;

use chrono::Local;
use common::{BUDGET, DESIGN, QUICK, copy_note, storage};
use note_tools::list_notes::{ListNotesInput, MatchPart, NoteItem};
use note_tools::storage::ProblemFolderKind;
use note_tools::{ErrorKind, ListNotes, NoteTools, Tool};
use serde_json::json;

fn list(root: &std::path::Path, input: ListNotesInput) -> Vec<NoteItem> {
    ListNotes::default().call(root, input).unwrap().notes
}

fn ids(notes: &[NoteItem]) -> Vec<&str> {
    notes.iter().map(|n| n.note_id.as_str()).collect()
}

#[test]
fn every_note_lists_with_its_details_and_the_problem_folders_with_their_kind() {
    let dir = storage();
    let out = ListNotes::default()
        .call(dir.path(), ListNotesInput::default())
        .unwrap();
    assert_eq!(
        ids(&out.notes),
        [
            "note-quick-idea",
            "note-standup",
            "note-vendor-call",
            "note-design-review",
            "note-hiring-sync",
            "note-budget-review",
        ]
    );
    let hiring = out
        .notes
        .iter()
        .find(|n| n.note_id == "note-hiring-sync")
        .unwrap();
    assert_eq!(hiring.note_type, "Meetings");
    assert_eq!(hiring.title.as_deref(), Some("Hiring sync"));
    let budget = out
        .notes
        .iter()
        .find(|n| n.note_id == "note-budget-review")
        .unwrap();
    assert_eq!(budget.note_type, "General");
    assert_eq!(budget.tags, ["Finance", "Team::Planning"]);
    assert_eq!(budget.date, "2026-10-01");
    let kinds: Vec<_> = out
        .problems
        .iter()
        .map(|p| (p.folder.as_str(), p.kind))
        .collect();
    assert!(kinds.contains(&(
        "2026-10-05 1500 Broken frontmatter",
        ProblemFolderKind::Invalid
    )));
    assert!(kinds.contains(&(
        "2026-10-08 1600 Newer layout",
        ProblemFolderKind::NewerLayout
    )));
    assert!(kinds.contains(&("2026-10-09 0930 Retro", ProblemFolderKind::WaitingForRecord)));
    assert!(!out.truncated);
}

#[test]
fn a_note_is_busy_by_a_fresh_marker_an_unsaved_recording_or_a_waiting_type_move() {
    let dir = storage();
    let busy = |id: &str| {
        list(dir.path(), ListNotesInput::default())
            .into_iter()
            .find(|n| n.note_id == id)
            .unwrap()
            .busy
    };
    assert!(!busy("note-design-review"));
    assert!(busy("note-standup"));
    assert!(busy("note-vendor-call"));
    assert!(!busy("note-budget-review"));
    session_save::in_use_marker::write_marker(
        &dir.path().join(DESIGN),
        "run-on-another-computer",
        Local::now().fixed_offset(),
    )
    .unwrap();
    assert!(busy("note-design-review"));
    let only_busy = list(
        dir.path(),
        ListNotesInput {
            busy: Some(true),
            ..Default::default()
        },
    );
    assert_eq!(
        ids(&only_busy),
        ["note-standup", "note-vendor-call", "note-design-review"]
    );
}

#[test]
fn type_and_keyword_give_only_matching_notes_and_where_the_keyword_is() {
    let dir = storage();
    let found = list(
        dir.path(),
        ListNotesInput {
            note_type: Some("general".into()),
            keyword: Some("TRAVEL".into()),
            ..Default::default()
        },
    );
    assert_eq!(ids(&found), ["note-budget-review"]);
    assert_eq!(
        found[0].keyword_in,
        Some(vec![MatchPart::Summary, MatchPart::Transcript])
    );
    let by_tag_text = list(
        dir.path(),
        ListNotesInput {
            keyword: Some("finance".into()),
            ..Default::default()
        },
    );
    assert_eq!(ids(&by_tag_text), ["note-budget-review"]);
    assert!(
        by_tag_text[0]
            .keyword_in
            .as_ref()
            .unwrap()
            .contains(&MatchPart::Tags)
    );
    let none = list(
        dir.path(),
        ListNotesInput {
            note_type: Some("Meetings".into()),
            keyword: Some("travel".into()),
            ..Default::default()
        },
    );
    assert!(none.is_empty());
}

#[test]
fn tags_match_by_key_and_a_scope_matches_each_tag_in_it() {
    let dir = storage();
    let key = list(
        dir.path(),
        ListNotesInput {
            tags: vec!["team :: planning".into()],
            ..Default::default()
        },
    );
    assert_eq!(ids(&key), ["note-budget-review"]);
    let scope = list(
        dir.path(),
        ListNotesInput {
            tags: vec!["Team::".into()],
            ..Default::default()
        },
    );
    assert_eq!(ids(&scope), ["note-hiring-sync", "note-budget-review"]);
}

#[test]
fn a_date_range_and_a_limit_select_notes() {
    let dir = storage();
    let range = list(
        dir.path(),
        ListNotesInput {
            since: Some("2026-10-02".into()),
            until: Some("2026-10-04".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        ids(&range),
        ["note-vendor-call", "note-design-review", "note-hiring-sync"]
    );
    let out = ListNotes::default()
        .call(
            dir.path(),
            ListNotesInput {
                limit: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(out.notes.len(), 2);
    assert_eq!(out.matching, 6);
    assert!(out.truncated);
}

#[test]
fn bad_input_and_a_folder_with_no_marker_give_typed_errors() {
    let dir = storage();
    let error = ListNotes::default()
        .call(
            dir.path(),
            ListNotesInput {
                since: Some("October".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidInput);
    let error = ListNotes::default()
        .call(
            dir.path(),
            ListNotesInput {
                limit: Some(0),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidInput);
    let plain = tempfile::tempdir().unwrap();
    let error = ListNotes::default()
        .call(plain.path(), ListNotesInput::default())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotAStorageFolder);
    assert!(!plain.path().join(".longhand").exists(), "the tool wrote");
}

#[test]
fn a_title_that_yaml_reads_as_a_number_or_a_bool_is_text() {
    let dir = storage();
    let note = copy_note(
        dir.path(),
        QUICK,
        "2026-10-10 1000 Odd title",
        Some("note-odd"),
    );
    fs::write(
        note.join("session.md"),
        "---\ntitle: 1.50\nlonghand_tags:\n- yes\n---\n",
    )
    .unwrap();
    let found = list(
        dir.path(),
        ListNotesInput {
            title: Some("1.50".into()),
            tags: vec!["yes".into()],
            ..Default::default()
        },
    );
    assert_eq!(ids(&found), ["note-odd"]);
}

#[test]
fn the_json_entry_point_gives_the_typed_result() {
    let dir = storage();
    let typed = ListNotes::default()
        .call(
            dir.path(),
            ListNotesInput {
                tags: vec!["Finance".into()],
                ..Default::default()
            },
        )
        .unwrap();
    let json = NoteTools::new(10_000)
        .call(dir.path(), "list_notes", json!({"tags": ["Finance"]}))
        .unwrap();
    assert_eq!(json, serde_json::to_value(typed).unwrap());
    let error = NoteTools::new(10_000)
        .call(dir.path(), "list_notes", json!({"colour": "red"}))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidInput);
}

#[cfg(unix)]
#[test]
fn a_settings_folder_that_is_a_link_is_refused() {
    let dir = storage();
    let outside = tempfile::tempdir().unwrap();
    fs::rename(
        dir.path().join(".longhand"),
        outside.path().join(".longhand"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        outside.path().join(".longhand"),
        dir.path().join(".longhand"),
    )
    .unwrap();
    let error = ListNotes::default()
        .call(dir.path(), ListNotesInput::default())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotAStorageFolder);
}

#[cfg(unix)]
#[test]
fn a_linked_note_definition_in_a_subfolder_of_a_note_folder_is_not_read() {
    let dir = storage();
    let outside = tempfile::tempdir().unwrap();
    let away = copy_note(dir.path(), BUDGET, "away", Some("note-outside"));
    fs::rename(&away, outside.path().join("away")).unwrap();
    // The host's own definition gives no note, so the scan reads it as a
    // type folder and reaches its subfolder.
    let host = copy_note(dir.path(), DESIGN, "2026-10-12 Host", Some("note-host"));
    fs::write(host.join("session.md"), "not a note definition").unwrap();
    let _ = fs::remove_file(host.join("longhand.json"));
    let inner = host.join("inner");
    fs::create_dir(&inner).unwrap();
    for name in ["session.md", "longhand.json"] {
        let target = outside.path().join("away").join(name);
        if target.exists() {
            std::os::unix::fs::symlink(&target, inner.join(name)).unwrap();
        }
    }
    let out = ListNotes::default()
        .call(dir.path(), ListNotesInput::default())
        .unwrap();
    assert!(!ids(&out.notes).contains(&"note-outside"));
}

#[cfg(unix)]
#[test]
fn a_note_folder_that_links_outside_the_storage_folder_is_not_read() {
    let dir = storage();
    let outside = tempfile::tempdir().unwrap();
    let away = copy_note(dir.path(), BUDGET, "away", Some("note-outside"));
    fs::rename(&away, outside.path().join("away")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("away"),
        dir.path().join("2026-10-11 Linked"),
    )
    .unwrap();
    let out = ListNotes::default()
        .call(dir.path(), ListNotesInput::default())
        .unwrap();
    assert!(!ids(&out.notes).contains(&"note-outside"));
    let link = out
        .problems
        .iter()
        .find(|p| p.folder == "2026-10-11 Linked")
        .unwrap();
    assert_eq!(link.kind, ProblemFolderKind::Link);
}
