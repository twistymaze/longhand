//! The `readers` and `steps` agent shapes (U5, KTD13).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fixed, Script, call, label, reply, text};
use note_tools::NoteTools;
use serde_json::json;
use summarize::SummarizeError;
use summarize::chunk::{Chunk, Line, Turn};
use summarize::prompts::{Details, UserText};
use summary_agent::shapes::{ReadPart, Steps, largest_part, run_steps, transcript_fits};
use summary_agent::{Agent, HistoryPolicy, Limits, NoteSource, Outcome, ToolSet, TurnKind};

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn storage() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/storage-folder");
    copy_dir(&fixture, dir.path());
    dir
}

fn read_part<'a>(model: &'a Script, root: &Path, reader: Limits, largest: usize) -> ReadPart<'a> {
    ReadPart::new(
        model,
        root.to_path_buf(),
        "note-budget-review".into(),
        String::new(),
        Details::default(),
        reader,
        20_000,
        largest,
        label(),
    )
}

async fn lead(model: &Script, part: &ReadPart<'_>) -> summary_agent::RunResult {
    let tools = ToolSet::new(vec![part]).unwrap();
    Agent {
        model,
        tools: &tools,
        preamble: "lead rules".into(),
        prompt: "summarize".into(),
        limits: Limits::default(),
        history: HistoryPolicy::KeepAll,
        label: label(),
        name: "lead".into(),
    }
    .run()
    .await
}

#[tokio::test]
async fn a_lead_that_calls_two_readers_ends_with_a_summary_and_counts_their_turns() {
    let dir = storage();
    let model = Script::new(vec![
        // lead
        Ok(reply(
            vec![call(
                "l1",
                "read_part",
                json!({"first_line": 1, "last_line": 4, "instruction": "money"}),
            )],
            100,
            10,
        )),
        // reader 1
        Ok(reply(vec![text("- [00:00:01] three items")], 60, 10)),
        // lead
        Ok(reply(
            vec![call(
                "l2",
                "read_part",
                json!({"first_line": 5, "last_line": 8}),
            )],
            120,
            10,
        )),
        // reader 2
        Ok(reply(
            vec![text("- [00:00:31] training line stays")],
            40,
            10,
        )),
        // lead
        Ok(reply(vec![text("The budget review summary.")], 150, 30)),
    ]);
    let part = read_part(&model, dir.path(), Limits::default(), 100);
    let result = lead(&model, &part).await;
    assert_eq!(result.outcome, Outcome::Summary);
    assert_eq!(result.totals.model_calls, 5);
    assert_eq!(result.totals.tool_calls, 2);
    let agents: Vec<&str> = result.turns.iter().map(|t| t.agent.as_str()).collect();
    assert!(agents.contains(&"reader 1") && agents.contains(&"reader 2"));
    let turns = model.turns();
    // A reader starts with a new context, gets its lines in its prompt and
    // has no tools.
    assert!(turns[1].history.is_empty());
    assert!(turns[1].tools.is_empty());
    let prompt = format!("{:?}", turns[1].prompt);
    assert!(prompt.contains("transcript lines=\\\"1-4\\\""), "{prompt}");
    assert!(prompt.contains("money"));
    // The reader's notes reach the lead as a tool result.
    assert!(format!("{:?}", turns[2].prompt).contains("three items"));
}

#[tokio::test]
async fn a_reader_that_fails_is_an_error_result_for_the_lead() {
    let dir = storage();
    let model = Script::new(vec![
        Ok(reply(
            vec![call(
                "l1",
                "read_part",
                json!({"first_line": 1, "last_line": 4}),
            )],
            10,
            10,
        )),
        Err(SummarizeError::Transport("connection refused".into())),
        Ok(reply(vec![text("Summary without that part.")], 10, 10)),
    ]);
    let part = read_part(&model, dir.path(), Limits::default(), 100);
    let result = lead(&model, &part).await;
    assert_eq!(result.outcome, Outcome::Summary);
    let read = result
        .turns
        .iter()
        .find(|t| t.tool.as_deref() == Some("read_part"))
        .unwrap();
    assert!(read.error);
    assert!(read.result.as_deref().unwrap().contains("Transport"));
}

#[tokio::test]
async fn notes_longer_than_twice_the_size_are_cut_with_a_marker() {
    let dir = storage();
    let model = Script::new(vec![
        Ok(reply(
            vec![call(
                "l1",
                "read_part",
                json!({"first_line": 1, "last_line": 4}),
            )],
            10,
            10,
        )),
        Ok(reply(vec![text(&"note ".repeat(200))], 10, 10)),
        Ok(reply(vec![text("Summary.")], 10, 10)),
    ]);
    let part = read_part(&model, dir.path(), Limits::default(), 100).with_notes_tokens(10);
    let result = lead(&model, &part).await;
    assert_eq!(result.outcome, Outcome::Summary);
    let turns = model.turns();
    assert!(format!("{:?}", turns[1].prompt).contains("at most about 7 words"));
    let read = result
        .turns
        .iter()
        .find(|t| t.tool.as_deref() == Some("read_part"))
        .unwrap();
    let notes = read.result.as_deref().unwrap();
    assert!(notes.contains("notes were cut here"));
    assert!(notes.len() < 200, "{} characters", notes.len());
}

#[tokio::test]
async fn a_reader_reads_only_its_note_and_a_too_large_range_starts_no_reader() {
    let dir = storage();
    let source = NoteSource {
        root: dir.path().to_path_buf(),
        tools: NoteTools::new(1_000),
        only: Some("note-budget-review".into()),
    };
    use summary_agent::ToolSource;
    let names: Vec<String> = source
        .definitions()
        .iter()
        .map(|d| d.name.as_str().to_string())
        .collect();
    assert_eq!(names, ["get_note"]);
    let other = source
        .call("get_note", json!({"note_id": "note-hiring-sync"}))
        .await;
    assert!(other.is_error && other.text.contains("only the note note-budget-review"));

    let model = Script::new(vec![
        Ok(reply(
            vec![call(
                "l1",
                "read_part",
                json!({"first_line": 1, "last_line": 500}),
            )],
            10,
            10,
        )),
        Ok(reply(vec![text("done")], 10, 10)),
    ]);
    let part = read_part(&model, dir.path(), Limits::default(), 100);
    let result = lead(&model, &part).await;
    assert_eq!(model.turns().len(), 2, "a reader started");
    let read = result
        .turns
        .iter()
        .find(|t| t.kind == TurnKind::Tool)
        .unwrap();
    assert!(
        read.result
            .as_deref()
            .unwrap()
            .contains("at most 100 lines")
    );
}

#[test]
fn the_largest_part_fits_the_reader_budget() {
    let limits = Limits {
        context_tokens: 16_000,
        reply_tokens: Some(2_000),
        ..Limits::default()
    };
    // 60-byte lines: 11,000 free tokens hold about 535 lines.
    let lines = largest_part(60_000, 1_000, &limits);
    assert!((450..=600).contains(&lines), "{lines}");
}

#[test]
fn a_transcript_fits_when_it_takes_at_most_the_results_room() {
    // Room for results: (32,000 - 8,000) / 2 = 12,000 tokens, about
    // 35,000 characters at 1.2 times 3.5 characters a token.
    let small = Limits {
        context_tokens: 32_000,
        reply_tokens: Some(8_000),
        ..Limits::default()
    };
    assert!(transcript_fits(30_000, &small));
    assert!(!transcript_fits(200_000, &small));
    // A three-hour transcript fits in a large context.
    let large = Limits {
        context_tokens: 1_000_000,
        ..Limits::default()
    };
    assert!(transcript_fits(200_000, &large));
}

fn chunk(text: &str, at: u64) -> Chunk {
    Chunk {
        lines: vec![Line::Turn(Turn {
            position_ms: at,
            speaker: Some(1),
            text: text.into(),
        })],
        first_new: 0,
    }
}

#[tokio::test]
async fn steps_run_one_step_per_part_and_a_merge() {
    let chunks = [
        chunk("part one words", 0),
        chunk("part two words", 60_000),
        chunk("part three words", 120_000),
    ];
    let model = Script::new(vec![
        Ok(reply(vec![text("notes 1")], 10, 10)),
        Ok(reply(vec![text("notes 2")], 10, 10)),
        Ok(reply(vec![text("notes 3")], 10, 10)),
        Ok(reply(vec![text("The summary.")], 10, 10)),
    ]);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let details = Details::default();
    let result = run_steps(Steps {
        model: &model,
        tools: &tools,
        chunks: &chunks,
        note_id: "n1",
        text: "Summary text.",
        user: UserText {
            background: "",
            details: &details,
        },
        limits: Limits::default(),
        label: label(),
    })
    .await;
    assert_eq!(result.outcome, Outcome::Summary);
    assert_eq!(result.summary.as_deref(), Some("The summary."));
    let turns = model.turns();
    assert_eq!(turns.len(), 4);
    let first = format!("{:?}", turns[0].prompt);
    assert!(first.contains("part one words") && !first.contains("part two words"));
    let merge = format!("{:?}", turns[3].prompt);
    assert!(merge.contains("notes 3") && !merge.contains("part one words"));
}

#[tokio::test]
async fn a_failed_step_ends_the_run_with_its_outcome_and_no_merge() {
    let chunks = [chunk("one", 0), chunk("two", 60_000)];
    let model = Script::new(vec![Err(SummarizeError::ContextExceeded)]);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let details = Details::default();
    let result = run_steps(Steps {
        model: &model,
        tools: &tools,
        chunks: &chunks,
        note_id: "n1",
        text: "Summary text.",
        user: UserText {
            background: "",
            details: &details,
        },
        limits: Limits::default(),
        label: label(),
    })
    .await;
    assert_eq!(result.outcome, Outcome::ContextExceeded);
    assert_eq!(model.turns().len(), 1);
}
