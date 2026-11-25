//! The timeline flow and the review pass.

mod common;

use std::path::PathBuf;

use common::{Fixed, Script, call, label, reply, text, tool_results};
use serde_json::json;
use summarize::prompts::Details;
use summary_agent::passes::Pass;
use summary_agent::{Limits, MemoryNoteSource, Outcome, ToolSet};

/// The transcript of the budget review note of the fixture: 8 lines.
fn transcript() -> String {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/storage-folder/2026-10-01 0900 Budget review/transcript.md");
    std::fs::read_to_string(fixture).unwrap()
}

/// The pass of a note that is in memory only: no storage folder has it.
fn pass(model: &Script, limits: Limits) -> Pass<'_> {
    Pass {
        model,
        note: MemoryNoteSource {
            note_id: "note-budget-review".into(),
            started: "2026-10-01T09:00:00-05:00".into(),
            transcript: transcript(),
            manual_notes: "budget first".into(),
            details: Details::default(),
            max_chars: 5_000,
        },
        background: String::new(),
        text: "Write a summary.".into(),
        limits,
        label: label(),
    }
}

#[tokio::test]
async fn a_review_with_no_findings_keeps_the_draft() {
    let model = Script::new(vec![Ok(reply(vec![text("NONE")], 10, 2))]);
    let result = pass(&model, Limits::default())
        .review("The draft.", 8, 400)
        .await;
    assert_eq!(result.outcome, Outcome::Summary);
    assert_eq!(result.summary.as_deref(), Some("The draft."));
    assert_eq!(model.turns().len(), 1);
    let prompt = format!("{:?}", model.turns()[0].prompt);
    assert!(prompt.contains("The draft.") && prompt.contains("lines=\\\"1-8\\\""));
}

#[tokio::test]
async fn findings_go_to_a_reviser_whose_edits_change_only_their_text() {
    let edits = r#"{"edits": [
        {"find": "Budget is flat.", "replace": "Budget is flat. The training line stays."},
        {"find": "Sam owns it.", "replace": ""},
        {"find": "not in the draft", "replace": "x"}
    ]}"#;
    let model = Script::new(vec![
        Ok(reply(
            vec![text("missed [00:00:31] the training line stays")],
            10,
            5,
        )),
        Ok(reply(vec![text(edits)], 10, 5)),
    ]);
    let draft = "# Summary\n\nBudget is flat. Sam owns it.\n\n- Next review in May.";
    let result = pass(&model, Limits::default()).review(draft, 8, 400).await;
    assert_eq!(result.outcome, Outcome::Summary);
    assert_eq!(
        result.summary.as_deref(),
        Some("# Summary\n\nBudget is flat. The training line stays. \n\n- Next review in May.")
    );
    assert_eq!(
        result.detail.as_deref(),
        Some("review: 2 of 3 edits applied")
    );
    let turns = model.turns();
    assert_eq!(turns.len(), 2);
    let revise = format!("{:?}", turns[1].prompt);
    assert!(revise.contains("training line stays") && revise.contains("Budget is flat."));
    assert!(turns[1].tools.is_empty());
}

#[tokio::test]
async fn a_reviser_reply_that_is_not_json_keeps_the_draft() {
    let model = Script::new(vec![
        Ok(reply(vec![text("missed [00:00:31] a thing")], 10, 5)),
        Ok(reply(vec![text("Here is a new summary.")], 10, 5)),
    ]);
    let result = pass(&model, Limits::default())
        .review("The draft.", 8, 400)
        .await;
    assert_eq!(result.summary.as_deref(), Some("The draft."));
    assert!(result.detail.unwrap().contains("no JSON object"));
}

#[tokio::test]
async fn a_small_context_audits_the_transcript_in_parts() {
    // About 1,035 free tokens hold 5 lines of 700 bytes: two parts.
    let limits = Limits {
        context_tokens: 4_000 + 1_000 + 40,
        reply_tokens: Some(1_000),
        ..Limits::default()
    };
    let model = Script::new(vec![
        Ok(reply(vec![text("NONE")], 10, 2)),
        Ok(reply(vec![text("NONE")], 10, 2)),
    ]);
    let result = pass(&model, limits).review("Draft.", 8, 5_600).await;
    assert_eq!(result.summary.as_deref(), Some("Draft."));
    assert_eq!(model.turns().len(), 2, "two auditors");
}

#[tokio::test]
async fn a_failed_auditor_keeps_the_draft_and_says_why() {
    let model = Script::new(vec![Err(summarize::SummarizeError::Transport(
        "refused".into(),
    ))]);
    let result = pass(&model, Limits::default())
        .review("The draft.", 8, 400)
        .await;
    assert_eq!(result.outcome, Outcome::Summary);
    assert_eq!(result.summary.as_deref(), Some("The draft."));
    assert!(result.detail.unwrap().contains("auditor 1"));
}

#[tokio::test]
async fn a_timeline_that_fits_goes_to_one_writer() {
    let model = Script::new(vec![
        Ok(reply(vec![text("- [00:00:01] budget talk")], 10, 5)),
        Ok(reply(vec![text("The summary.")], 10, 5)),
    ]);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let result = pass(&model, Limits::default())
        .timeline(&tools, 8, 400)
        .await;
    assert_eq!(result.outcome, Outcome::Summary);
    assert_eq!(result.summary.as_deref(), Some("The summary."));
    let turns = model.turns();
    assert_eq!(turns.len(), 2);
    // The timeline part gets its lines and no tools.
    assert!(turns[0].tools.is_empty());
    assert!(format!("{:?}", turns[0].prompt).contains("lines=\\\"1-8\\\""));
    // The writer gets the timeline, the manual notes and the tools.
    let writer = format!("{:?}", turns[1].prompt);
    assert!(writer.contains("budget talk") && writer.contains("budget first"));
    assert!(!turns[1].tools.is_empty());
}

#[tokio::test]
async fn a_timeline_that_does_not_fit_goes_to_group_writers_and_a_merge() {
    // 1,500 free tokens, less 1,000 for the people list and 3 for the
    // notes: parts of 2 lines of 770 bytes, so 4 parts. Four timelines of
    // 1,000 bytes pass two thirds of the writer's room: groups of two.
    let limits = Limits {
        context_tokens: 3_000 + 1_000 + 1_500,
        reply_tokens: Some(1_000),
        ..Limits::default()
    };
    let part = "x".repeat(1_000);
    let mut replies: Vec<_> = (0..4)
        .map(|_| Ok(reply(vec![text(&part)], 10, 5)))
        .collect();
    // Writer 1 writes far more than its share of the merge (about 374
    // tokens, 1,309 characters): the merge gets it cut.
    let long = format!("notes one {}", "n".repeat(3_000));
    replies.push(Ok(reply(vec![text(&long)], 10, 5)));
    replies.push(Ok(reply(vec![text("notes two")], 10, 5)));
    replies.push(Ok(reply(vec![text("The merged summary.")], 10, 5)));
    let model = Script::new(replies);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let result = pass(&model, limits).timeline(&tools, 8, 5_600).await;
    assert_eq!(result.summary.as_deref(), Some("The merged summary."));
    let turns = model.turns();
    assert_eq!(turns.len(), 7);
    assert!(format!("{:?}", turns[4].prompt).contains("part 1 of 2"));
    let merge = format!("{:?}", turns[6].prompt);
    assert!(merge.contains("notes one") && merge.contains("notes two"));
    assert!(merge.contains("These notes were cut here."));
    assert!(!merge.contains(&"n".repeat(1_400)));
}

#[tokio::test]
async fn a_failed_timeline_part_ends_the_run_with_no_summary() {
    let model = Script::new(vec![Err(summarize::SummarizeError::ContextExceeded)]);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let result = pass(&model, Limits::default())
        .timeline(&tools, 8, 400)
        .await;
    assert_eq!(result.outcome, Outcome::ContextExceeded);
    assert!(result.summary.is_none());
    assert_eq!(model.turns().len(), 1);
}

#[tokio::test]
async fn the_people_list_goes_from_part_to_part_and_to_the_writer() {
    // Parts of 4 lines: 2 parts.
    let limits = Limits {
        context_tokens: 3_000 + 1_000 + 1_000 + 1_000,
        reply_tokens: Some(1_000),
        ..Limits::default()
    };
    let model = Script::new(vec![
        Ok(reply(
            vec![text(
                "<people>\nSpeaker 1: Sam, lead\n</people>\n- [00:00:01] budget",
            )],
            10,
            5,
        )),
        Ok(reply(
            vec![text(
                "<people>\nSpeaker 1: Sam, lead\nSpeaker 2: Ana\n</people>\n- [00:00:31] training",
            )],
            10,
            5,
        )),
        Ok(reply(vec![text("The summary.")], 10, 5)),
    ]);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let result = pass(&model, limits).timeline(&tools, 8, 8 * 790).await;
    assert_eq!(result.summary.as_deref(), Some("The summary."));
    let turns = model.turns();
    assert_eq!(turns.len(), 3);
    let first = format!("{:?}", turns[0].prompt);
    assert!(
        first.contains("budget first"),
        "the part has the manual notes"
    );
    assert!(!first.contains("<people>\\n"), "the first part has no list");
    // A label is only a weak hint, and the list has no labels.
    let system = format!("{:?}", turns[0].preamble);
    assert!(system.contains("weak hint"), "{system}");
    assert!(!system.contains("Keep those labels"));
    assert!(system.contains("Keep every number, amount, name"));
    assert!(first.contains("Give no speaker labels in this block"));
    assert!(format!("{:?}", turns[1].prompt).contains("Speaker 1: Sam, lead"));
    let writer = format!("{:?}", turns[2].prompt);
    assert!(writer.contains("Speaker 2: Ana") && writer.contains("training"));
    assert!(
        !writer.contains("</people>"),
        "the list is out of the timeline"
    );
}

#[tokio::test]
async fn a_cut_off_part_runs_again_with_half_the_lines() {
    let model = Script::new(vec![
        Err(summarize::SummarizeError::ReplyCutOff),
        Ok(reply(vec![text("- [00:00:01] first half")], 10, 5)),
        Ok(reply(vec![text("- [00:00:31] second half")], 10, 5)),
        Ok(reply(vec![text("The summary.")], 10, 5)),
    ]);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let result = pass(&model, Limits::default())
        .timeline(&tools, 8, 400)
        .await;
    assert_eq!(result.summary.as_deref(), Some("The summary."));
    let turns = model.turns();
    assert_eq!(turns.len(), 4);
    assert!(format!("{:?}", turns[1].prompt).contains("lines=\\\"1-4\\\""));
    assert!(format!("{:?}", turns[2].prompt).contains("lines=\\\"5-8\\\""));
}

#[tokio::test]
async fn the_next_part_repeats_about_a_tenth_of_the_part_before() {
    // About 997 free tokens hold 5 lines of 660 bytes: lines 1-5, then the
    // next part repeats line 5.
    let limits = Limits {
        context_tokens: 3_000 + 1_000 + 1_000 + 1_000,
        reply_tokens: Some(1_000),
        ..Limits::default()
    };
    let model = Script::new(vec![
        Ok(reply(vec![text("- [00:00:01] first")], 10, 5)),
        Ok(reply(vec![text("- [00:00:31] second")], 10, 5)),
        Ok(reply(vec![text("The summary.")], 10, 5)),
    ]);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let result = pass(&model, limits).timeline(&tools, 8, 8 * 600).await;
    assert_eq!(result.summary.as_deref(), Some("The summary."));
    let turns = model.turns();
    assert_eq!(turns.len(), 3);
    assert!(format!("{:?}", turns[0].prompt).contains("lines=\\\"1-5\\\""));
    assert!(format!("{:?}", turns[1].prompt).contains("lines=\\\"5-8\\\""));
    // The writer is told that the parts overlap.
    assert!(format!("{:?}", turns[2].preamble).contains("overlap slightly"));
}

#[tokio::test]
async fn a_writer_whose_context_fills_runs_again_with_no_tools() {
    let model = Script::new(vec![
        Ok(reply(vec![text("- [00:00:01] budget talk")], 10, 5)),
        Err(summarize::SummarizeError::ContextExceeded),
        Ok(reply(vec![text("The summary.")], 10, 5)),
    ]);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let result = pass(&model, Limits::default())
        .timeline(&tools, 8, 400)
        .await;
    assert_eq!(result.outcome, Outcome::Summary);
    assert_eq!(result.summary.as_deref(), Some("The summary."));
    let turns = model.turns();
    assert_eq!(turns.len(), 3);
    assert!(!turns[1].tools.is_empty());
    assert!(turns[2].tools.is_empty(), "the second try has no tools");
}

#[tokio::test]
async fn a_writer_reads_lines_of_a_note_that_no_storage_folder_has() {
    let model = Script::new(vec![
        Ok(reply(vec![text("- [00:00:09] travel")], 10, 5)),
        Ok(reply(
            vec![call(
                "c1",
                "get_note",
                json!({"note_id": "note-budget-review", "transcript": {"from_line": 4, "to_line": 4}}),
            )],
            10,
            5,
        )),
        Ok(reply(vec![text("The draft.")], 10, 5)),
    ]);
    let pass = pass(&model, Limits::default());
    let tools = ToolSet::new(vec![&pass.note]).unwrap();
    let result = pass.timeline(&tools, 8, 400).await;
    assert_eq!(result.outcome, Outcome::Summary);
    assert_eq!(result.summary.as_deref(), Some("The draft."));
    let turns = model.turns();
    assert_eq!(turns.len(), 3);
    assert!(format!("{:?}", turns[0].prompt).contains("We have three items on the budget"));
    let results = tool_results(&turns[2]);
    assert_eq!(results.len(), 1);
    assert!(results[0].contains("\"first_line\":4"), "{}", results[0]);
    assert!(
        results[0].contains("The travel line is lower"),
        "{}",
        results[0]
    );
}
