//! The timeline flow and the review pass.

mod common;

use std::cell::RefCell;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{Fixed, Script, call, label, reply, text, tool_results};
use serde_json::json;
use summarize::client::{Label, RequestKind};
use summarize::prompts::Details;
use summarize::tokens::ReplyKind;
use summarize::{AgentReply, AgentTurn, SummarizeError};
use summary_agent::passes::{Pass, Step};
use summary_agent::{BoxFuture, Limits, MemoryNoteSource, Model, Outcome, ToolSet};
use tokio::sync::Notify;

/// The transcript of the budget review note of the fixture: 8 lines.
fn transcript() -> String {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/storage-folder/2026-10-01 0900 Budget review/transcript.md");
    std::fs::read_to_string(fixture).unwrap()
}

/// A transcript of `lines` short lines.
fn long_transcript(lines: usize) -> String {
    let mut text = String::from("# Transcript\n\n");
    for line in 1..=lines {
        text.push_str(&format!(
            "- [00:00:{:02}] Speaker 1: Line {line}.\n",
            line % 60
        ));
    }
    text
}

/// The pass of a note that is in memory only: no storage folder has it.
fn pass(model: &dyn Model, limits: Limits) -> Pass<'_> {
    pass_of(model, limits, transcript(), "budget first")
}

/// The pass of a note with `transcript` and `manual_notes`.
fn pass_of<'a>(
    model: &'a dyn Model,
    limits: Limits,
    transcript: String,
    manual_notes: &str,
) -> Pass<'a> {
    Pass {
        model,
        note: MemoryNoteSource {
            note_id: "note-budget-review".into(),
            started: "2026-10-01T09:00:00-05:00".into(),
            transcript,
            manual_notes: manual_notes.into(),
            details: Details::default(),
            max_chars: 5_000,
        },
        background: String::new(),
        text: "Write a summary.".into(),
        limits,
        label: label(),
        at_once: 1,
        progress: None,
    }
}

/// The first line of the `lines="a-b"` attribute in the prompt of `turn`.
fn first_line(turn: &AgentTurn) -> usize {
    let prompt = format!("{:?}", turn.prompt);
    let start = prompt.find("lines=\\\"").expect("a lines attribute") + "lines=\\\"".len();
    let rest = &prompt[start..];
    rest[..rest.find('-').unwrap()].parse().unwrap()
}

/// A hang guard: a test that waits for an event that never comes fails
/// here and does not stop the test run.
async fn guarded<T>(work: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), work)
        .await
        .expect("the work did not end")
}

/// A model for four audit parts and a reviser. The first auditor of each
/// pair replies only after the second auditor of the pair replied, so that
/// the replies come in the order 2, 1, 4, 3. It counts the open requests.
struct Pairs {
    calls: AtomicUsize,
    open: AtomicUsize,
    most_open: AtomicUsize,
    replied: [Notify; 4],
    seen: Mutex<Vec<AgentTurn>>,
}

impl Pairs {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            open: AtomicUsize::new(0),
            most_open: AtomicUsize::new(0),
            replied: Default::default(),
            seen: Mutex::default(),
        }
    }
}

impl Model for Pairs {
    fn turn<'a>(
        &'a self,
        _label: &'a Label,
        turn: &'a AgentTurn,
    ) -> BoxFuture<'a, Result<AgentReply, SummarizeError>> {
        self.seen.lock().unwrap().push(turn.clone());
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let open = self.open.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_open.fetch_max(open, Ordering::SeqCst);
        Box::pin(async move {
            if call >= 4 {
                self.open.fetch_sub(1, Ordering::SeqCst);
                return Ok(reply(vec![text(r#"{"edits": []}"#)], 10, 5));
            }
            // Two lines in each part.
            let part = first_line(turn).div_ceil(2);
            let first_of_pair = call.is_multiple_of(2);
            if first_of_pair {
                self.replied[call].notified().await;
            }
            self.open.fetch_sub(1, Ordering::SeqCst);
            if !first_of_pair {
                self.replied[call - 1].notify_one();
            }
            Ok(reply(vec![text(&format!("missed in part {part}"))], 10, 5))
        })
    }
}

#[tokio::test]
async fn a_part_cut_off_seven_times_fails_after_the_shrink_limit() {
    // 200 lines in one part; each cut-off halves it: 100, 50, 25, 12, 6
    // and 3 lines. The seventh cut-off ends the run.
    let model = Script::new((0..9).map(|_| Err(SummarizeError::ReplyCutOff)).collect());
    let transcript = long_transcript(200);
    let chars = transcript.chars().count();
    let tools = ToolSet::new(Vec::new()).unwrap();
    let result = pass_of(&model, Limits::default(), transcript, "")
        .timeline(&tools, 200, chars)
        .await
        .result;
    assert_eq!(result.outcome, Outcome::CutOff);
    assert!(result.summary.is_none());
    let turns = model.turns();
    assert_eq!(turns.len(), 7, "one request and six halvings");
    assert!(format!("{:?}", turns[6].prompt).contains("lines=\\\"1-3\\\""));
}

#[tokio::test]
async fn a_local_reply_kind_cut_off_at_its_ceiling_fails_after_one_request() {
    let model = Script::new(vec![Err(SummarizeError::ReplyCutOff)]);
    let tools = ToolSet::new(Vec::new()).unwrap();
    let limits = Limits {
        reply_kind: ReplyKind::Full,
        ..Limits::default()
    };
    let timeline = pass(&model, limits).timeline(&tools, 8, 400).await;
    assert_eq!(timeline.result.outcome, Outcome::CutOff);
    assert!(timeline.result.summary.is_none());
    assert!(timeline.writer.is_none(), "no writer ran");
    assert_eq!(model.turns().len(), 1);
}

#[tokio::test]
async fn auditors_run_up_to_at_once_and_their_findings_join_in_part_order() {
    // About 1,035 free tokens hold 2 lines of 1,400 bytes: four parts.
    let limits = Limits {
        context_tokens: 4_000 + 1_000 + 40,
        reply_tokens: Some(1_000),
        ..Limits::default()
    };
    let model = Pairs::new();
    let events = RefCell::new(Vec::new());
    let hook = |step, done, total| events.borrow_mut().push((step, done, total));
    let mut pass = pass(&model, limits);
    pass.at_once = 2;
    pass.progress = Some(&hook);
    let result = guarded(pass.review("Draft.", 8, 8 * 1_400)).await;
    assert_eq!(result.outcome, Outcome::Summary);
    assert_eq!(
        result.detail.as_deref(),
        Some("review: 0 of 0 edits applied")
    );
    assert_eq!(model.most_open.load(Ordering::SeqCst), 2);
    let turns = model.seen.lock().unwrap().clone();
    assert_eq!(turns.len(), 5, "four auditors and a reviser");
    let revise = format!("{:?}", turns[4].prompt);
    let at = |part: usize| revise.find(&format!("missed in part {part}")).unwrap();
    assert!(at(1) < at(2) && at(2) < at(3) && at(3) < at(4), "{revise}");
    assert_eq!(
        events.into_inner(),
        [
            (Step::Review, 0, 4),
            (Step::Review, 1, 4),
            (Step::Review, 2, 4),
            (Step::Review, 3, 4),
            (Step::Review, 4, 4),
            (Step::Review, 0, 1),
        ]
    );
}

#[tokio::test]
async fn progress_gives_the_current_part_total_after_a_halving() {
    // 997 tokens for lines of 825 bytes: parts of 4 lines, 1-4, 5-8 and
    // 9-10. Part 2 is cut off: 5-6, 7-8 and 9-10 follow.
    let limits = Limits {
        context_tokens: 3_000 + 1_000 + 2_000,
        reply_tokens: Some(1_000),
        ..Limits::default()
    };
    let part = |t: &str| Ok(reply(vec![text(t)], 10, 5));
    let model = Script::new(vec![
        part("- one"),
        Err(SummarizeError::ReplyCutOff),
        part("- two"),
        part("- three"),
        part("- four"),
        part("The summary."),
    ]);
    let events = RefCell::new(Vec::new());
    let hook = |step, done, total| events.borrow_mut().push((step, done, total));
    let mut pass = pass_of(&model, limits, long_transcript(10), "budget first");
    pass.progress = Some(&hook);
    let tools = ToolSet::new(Vec::new()).unwrap();
    let result = pass.timeline(&tools, 10, 7_500).await.result;
    assert_eq!(result.summary.as_deref(), Some("The summary."));
    let turns = model.turns();
    assert!(format!("{:?}", turns[2].prompt).contains("lines=\\\"5-6\\\""));
    assert_eq!(
        events.into_inner(),
        [
            (Step::Parts, 0, 3),
            (Step::Parts, 1, 3),
            (Step::Parts, 2, 4),
            (Step::Parts, 3, 4),
            (Step::Parts, 4, 4),
            (Step::Final, 0, 1),
        ]
    );
}

#[tokio::test]
async fn a_writer_at_the_tool_call_limit_runs_again_with_no_tools() {
    let limits = Limits {
        tool_calls: 1,
        ..Limits::default()
    };
    let model = Script::new(vec![
        Ok(reply(vec![text("- [00:00:01] budget talk")], 10, 5)),
        Ok(reply(
            vec![
                call("c1", "get_note", json!({})),
                call("c2", "get_note", json!({})),
            ],
            10,
            5,
        )),
        Ok(reply(vec![text("The summary.")], 10, 5)),
    ]);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let result = pass(&model, limits).timeline(&tools, 8, 400).await.result;
    assert_eq!(result.outcome, Outcome::Summary);
    assert_eq!(result.summary.as_deref(), Some("The summary."));
    let turns = model.turns();
    assert_eq!(turns.len(), 3);
    assert!(turns[2].tools.is_empty(), "the second try has no tools");
}

#[tokio::test]
async fn notes_near_the_budget_still_give_parts_of_the_floor() {
    // 2,000 free tokens and notes of 2,000 tokens: the lines get the
    // floor, a quarter of the free tokens. 500 tokens hold 2 lines of 770
    // bytes.
    let limits = Limits {
        context_tokens: 3_000 + 1_000 + 2_000,
        reply_tokens: Some(1_000),
        ..Limits::default()
    };
    let model = Script::new(Vec::new());
    let tools = ToolSet::new(Vec::new()).unwrap();
    let notes = "n".repeat(7_000);
    pass_of(&model, limits, transcript(), &notes)
        .timeline(&tools, 8, 8 * 700)
        .await;
    let turns = model.turns();
    assert!(
        format!("{:?}", turns[0].prompt).contains("lines=\\\"1-2\\\""),
        "{:?}",
        turns[0].prompt
    );
}

#[tokio::test]
async fn each_step_gives_its_turns_its_own_request_kind() {
    let model = Script::new(vec![
        Ok(reply(vec![text("- [00:00:01] budget talk")], 10, 5)),
        Ok(reply(vec![text("The summary.")], 10, 5)),
        Ok(reply(vec![text("missed [00:00:31] a thing")], 10, 5)),
        Ok(reply(vec![text(r#"{"edits": []}"#)], 10, 5)),
    ]);
    let tools = ToolSet::new(Vec::new()).unwrap();
    let pass = pass(&model, Limits::default());
    let timeline = pass.timeline(&tools, 8, 400).await;
    pass.review("The summary.", 8, 400).await;
    assert_eq!(
        model.kinds(),
        [
            RequestKind::Timeline,
            RequestKind::Writer,
            RequestKind::Audit,
            RequestKind::Revise
        ]
    );
    let labels = model.labels.lock().unwrap().clone();
    assert!(labels.iter().all(|l| l.note.as_deref() == Some("n1")));
    // The title request gets the writer's messages.
    let messages = timeline.writer.expect("the writer's messages");
    let turns = model.turns();
    assert_eq!(messages[0].content, turns[1].preamble);
    assert!(messages[1].content.contains("budget talk"));
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
        .await
        .result;
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
    let timeline = pass(&model, limits).timeline(&tools, 8, 5_600).await;
    let result = timeline.result;
    assert_eq!(result.summary.as_deref(), Some("The merged summary."));
    let turns = model.turns();
    assert_eq!(turns.len(), 7);
    assert!(format!("{:?}", turns[4].prompt).contains("part 1 of 2"));
    let merge = format!("{:?}", turns[6].prompt);
    assert!(merge.contains("notes one") && merge.contains("notes two"));
    assert!(merge.contains("These notes were cut here."));
    assert!(!merge.contains(&"n".repeat(1_400)));
    // The title request gets the merge step's messages.
    let messages = timeline.writer.expect("the merge step's messages");
    assert_eq!(messages[0].content, turns[6].preamble);
    assert!(messages[1].content.contains("notes two"));
    assert!(messages[1].content.contains("These notes were cut here."));
    assert_eq!(
        model.kinds(),
        [
            [RequestKind::Timeline; 4].as_slice(),
            [RequestKind::Writer; 3].as_slice()
        ]
        .concat(),
        "the group writers and the merge step are writers"
    );
}

#[tokio::test]
async fn a_failed_timeline_part_ends_the_run_with_no_summary() {
    let model = Script::new(vec![Err(summarize::SummarizeError::ContextExceeded)]);
    let source = Fixed::notes("x");
    let tools = ToolSet::new(vec![&source]).unwrap();
    let result = pass(&model, Limits::default())
        .timeline(&tools, 8, 400)
        .await
        .result;
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
    let result = pass(&model, limits)
        .timeline(&tools, 8, 8 * 790)
        .await
        .result;
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
        .await
        .result;
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
    let result = pass(&model, limits)
        .timeline(&tools, 8, 8 * 600)
        .await
        .result;
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
        .await
        .result;
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
    let result = pass.timeline(&tools, 8, 400).await.result;
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
