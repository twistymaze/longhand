//! The note source in memory (KTD2): it serves get note for one note from
//! text that the caller gives, as the note tools serve it from a storage
//! folder.

use std::path::PathBuf;

use note_tools::NoteTools;
use serde_json::{Value, json};
use summarize::prompts::Details;
use summary_agent::{MemoryNoteSource, NoteSource, ToolSource};

const NOTE: &str = "note-budget-review";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/storage-folder")
}

fn file(name: &str) -> String {
    std::fs::read_to_string(fixture().join("2026-10-01 0900 Budget review").join(name)).unwrap()
}

/// The budget review note of the fixture, in memory.
fn source(max_chars: usize) -> MemoryNoteSource {
    MemoryNoteSource::new(
        NOTE.into(),
        "2026-10-01T09:00:00-05:00".into(),
        file("transcript.md"),
        file("notes.md"),
        Details {
            note_type: None,
            tags: vec!["Finance".into(), "Team::Planning".into()],
            title: Some("Budget review".into()),
        },
        max_chars,
    )
}

async fn memory_call(source: &MemoryNoteSource, name: &str, arguments: Value) -> (Value, bool) {
    let result = source.call(name, arguments).await;
    (serde_json::from_str(&result.text).unwrap(), result.is_error)
}

fn folder_call(max_chars: usize, arguments: Value) -> Value {
    NoteTools::new(max_chars)
        .call(&fixture(), "get_note", arguments)
        .unwrap()
}

#[tokio::test]
async fn the_source_gives_the_same_result_as_get_note_on_the_storage_folder() {
    let requests = [
        json!({"note_id": NOTE}),
        json!({"note_id": NOTE, "manual_notes": true, "transcript": {}}),
        json!({"note_id": NOTE, "transcript": {"from_line": 3, "to_line": 8}}),
        json!({"note_id": NOTE, "transcript": {"from_time": "00:00:09", "to_time": "00:00:31"}}),
        json!({"note_id": NOTE, "transcript": {"pattern": "line", "context": 1}}),
    ];
    // A large cap, and a cap that cuts the transcript into pages.
    for max_chars in [100_000, 90] {
        let memory = source(max_chars);
        for request in &requests {
            let (got, is_error) = memory_call(&memory, "get_note", request.clone()).await;
            assert!(!is_error, "{got}");
            assert_eq!(got, folder_call(max_chars, request.clone()), "{request}");
        }
    }
    // The small cap gives a page and the line of the next page.
    let (page, _) = memory_call(
        &source(90),
        "get_note",
        json!({"note_id": NOTE, "transcript": {}}),
    )
    .await;
    assert_eq!(page["transcript"]["truncated"], true);
    assert_eq!(page["transcript"]["hunks"][0]["first_line"], 1);
    assert!(page["transcript"]["next_line"].as_u64().unwrap() > 1);
}

#[tokio::test]
async fn the_source_gives_the_definition_of_get_note_only() {
    let memory = source(5_000);
    let reader = NoteSource {
        root: fixture(),
        tools: NoteTools::new(5_000),
        only: Some(NOTE.into()),
    };
    let names: Vec<String> = memory
        .definitions()
        .iter()
        .map(|d| d.name.as_str().to_string())
        .collect();
    assert_eq!(names, ["get_note"]);
    assert_eq!(
        format!("{:?}", memory.definitions()),
        format!("{:?}", reader.definitions())
    );
}

#[tokio::test]
async fn another_note_is_not_found_and_list_notes_is_refused() {
    let memory = source(5_000);
    let (other, is_error) = memory_call(
        &memory,
        "get_note",
        json!({"note_id": "note-design-review", "transcript": {}}),
    )
    .await;
    assert!(is_error);
    assert_eq!(other["error"], "not_found");
    assert!(!other.to_string().contains("travel line"));

    let (list, is_error) = memory_call(&memory, "list_notes", json!({})).await;
    assert!(is_error);
    assert_eq!(list["error"], "invalid_input");
    assert!(!list.to_string().contains("Budget review"));
}

#[tokio::test]
async fn the_source_never_gives_a_summary() {
    // The note has a summary in the storage folder, and get note on the
    // folder gives it.
    let request = json!({"note_id": NOTE, "summary": true, "manual_notes": true});
    let folder = folder_call(5_000, request.clone());
    assert!(folder["summary"]["text"].is_string());

    let (got, is_error) = memory_call(&source(5_000), "get_note", request).await;
    assert!(!is_error, "{got}");
    assert!(got.get("summary").is_none(), "{got}");
    assert_eq!(got["manual_notes"], folder["manual_notes"]);
}

#[test]
fn the_source_gives_transcript_lines_with_their_raw_text() {
    let memory = source(10);
    assert_eq!(
        memory.lines(3, 4).unwrap(),
        "- [00:00:01] Speaker 1: We have three items on the budget today.\n\
         - [00:00:09] Speaker 2: The travel line is lower than last quarter."
    );
    // The size cap of a tool result does not cut the lines of a pass.
    assert_eq!(memory.lines(1, 8).unwrap().lines().count(), 8);
}
