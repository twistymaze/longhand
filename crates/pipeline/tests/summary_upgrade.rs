//! `produce` runs the summary pipeline (U4 of the summary pipeline plan):
//! the review, the read tool of the writer, cancel, Retry, the failure
//! messages, the budget, and the stored data of older builds.

mod common;
#[path = "../../summarize/tests/common/mod.rs"]
mod http;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pipeline::summarize::{BUDGET, INPUT, PROMPT, SummaryEndpoint, TITLE_REQUEST, produce};
use pipeline::{Control, SharedStore, StageError, Unattended, checkpoint};
use serde_json::{Value, json};
use session_core::Stage;
use session_core::folder::Summary;
use summarize::{Provider, Thinking};

const NOTE: &str = "n1";
const PATH: &str = "/v1/chat/completions";
const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: We ship on Friday.\n\n- [00:00:09] Speaker 2: Sam owns the release notes.\n";

/// A store with the note [`NOTE`], and its working folder with
/// [`TRANSCRIPT`]. No storage folder holds the note: a first summary runs
/// before Save.
fn note() -> (SharedStore, tempfile::TempDir) {
    let store = common::store();
    store
        .lock()
        .unwrap()
        .create_session(NOTE, common::at(0), None)
        .unwrap();
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("transcript.md"), TRANSCRIPT).unwrap();
    (store, folder)
}

fn endpoint(base: &str) -> SummaryEndpoint {
    SummaryEndpoint {
        provider: Provider::Custom,
        base_url: base.into(),
        model: "m".into(),
        credential: None,
        context_limit: Some(32_000),
        thinking: Thinking::default(),
        requests_at_once: 1,
        retry_waits: Vec::new(),
        openrouter: None,
    }
}

/// `produce` for [`NOTE`] on the blocking pool, as the runner calls it.
async fn run(
    store: &SharedStore,
    folder: &Path,
    endpoint: SummaryEndpoint,
    control: Arc<dyn Control>,
) -> Result<Summary, StageError> {
    let (store, folder) = (Arc::clone(store), folder.to_path_buf());
    tokio::task::spawn_blocking(move || {
        produce(
            &store,
            NOTE,
            &folder,
            endpoint,
            summarize::prompts::topic_summary(),
            &*control,
            std::time::Duration::ZERO,
            &mut |_, _, _| {},
        )
    })
    .await
    .unwrap()
}

/// The step of the pipeline that a request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    Timeline,
    Writer,
    Audit,
    Revise,
}

fn json_of(body: &str) -> Value {
    serde_json::from_str(body).unwrap()
}

/// The text of the user message of the request `body`. The system
/// messages name every block, so only the user message tells the steps
/// apart.
fn user_text(body: &str) -> String {
    json_of(body)["messages"][1]["content"].to_string()
}

fn turn_of(body: &str) -> Turn {
    let text = user_text(body);
    if text.contains("<findings>") {
        Turn::Revise
    } else if text.contains("<draft>") {
        Turn::Audit
    } else if text.contains("<timeline>") {
        Turn::Writer
    } else {
        Turn::Timeline
    }
}

/// The names of the tools of the request `body`.
fn tools_of(body: &str) -> Vec<String> {
    json_of(body)["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .map(|t| t["function"]["name"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The text of the reply to a request of `turn` when nothing goes wrong.
fn normal(turn: Turn) -> &'static str {
    match turn {
        Turn::Timeline => "<people>\nSam: release owner\n</people>\n- [00:00:01] Ship on Friday.",
        Turn::Writer => "The draft summary.",
        Turn::Audit => "NONE",
        Turn::Revise => r#"{"edits": []}"#,
    }
}

fn ok(text: &str) -> (u16, String) {
    (200, http::chat_reply(text, "stop"))
}

fn server_error() -> (u16, String) {
    (500, r#"{"error":"overloaded"}"#.into())
}

/// A server that answers each request with `answer`.
async fn serve(answer: impl Fn(&str) -> (u16, String) + Send + Sync + 'static) -> http::Mock {
    http::serve(HashMap::from([(
        PATH,
        http::Reply::With(Arc::new(move |body: &str| answer(body))),
    )]))
    .await
}

fn bodies(mock: &http::Mock) -> Vec<String> {
    mock.seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.path == PATH)
        .map(|s| s.body.clone())
        .collect()
}

fn turns(mock: &http::Mock) -> Vec<Turn> {
    bodies(mock).iter().map(|b| turn_of(b)).collect()
}

fn keys(store: &SharedStore) -> Vec<String> {
    checkpoint::all(store, NOTE, Stage::Summarize)
        .unwrap()
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

/// Covers R3: an auditor that fails keeps the draft. The summary
/// completes, and the run gives no failure.
#[tokio::test(flavor = "multi_thread")]
async fn an_auditor_failure_keeps_the_draft_and_the_summary_completes() {
    let (store, folder) = note();
    let mock = serve(|body| match turn_of(body) {
        Turn::Audit => server_error(),
        turn => ok(normal(turn)),
    })
    .await;
    let summary = run(
        &store,
        folder.path(),
        endpoint(&mock.base),
        Arc::new(Unattended),
    )
    .await
    .unwrap();
    assert_eq!(summary.body, "The draft summary.");
    assert_eq!(turns(&mock), [Turn::Timeline, Turn::Writer, Turn::Audit]);
    assert!(
        checkpoint::get::<Value>(&store, NOTE, Stage::Summarize, TITLE_REQUEST)
            .unwrap()
            .is_some()
    );
}

/// A chat reply with one call of `get_note` with `arguments`.
fn get_note_call(arguments: Value) -> (u16, String) {
    let reply = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "get_note", "arguments": arguments.to_string()}
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });
    (200, reply.to_string())
}

/// Covers R4, R5: the writer has the read tool for this note only. A read
/// of another note gets an error, and no text of the other note reaches
/// the provider. A read of this note gets its transcript lines.
#[tokio::test(flavor = "multi_thread")]
async fn the_writer_reads_this_note_and_no_other_note() {
    let (store, folder) = note();
    {
        let mut s = store.lock().unwrap();
        s.create_session("other", common::at(10), None).unwrap();
        pipeline::save::set_notes(&mut s, "other", "OTHER-NOTE-MARKER").unwrap();
    }
    let mock = serve(|body| {
        let calls = json_of(body)["messages"].as_array().unwrap().len();
        match turn_of(body) {
            // The first writer turn reads the other note, the second this
            // note, and the third writes the summary.
            Turn::Writer if calls == 2 => get_note_call(json!({"note_id": "other"})),
            Turn::Writer if calls == 4 => get_note_call(json!({
                "note_id": NOTE,
                "transcript": {"from_line": 1, "to_line": 5}
            })),
            turn => ok(normal(turn)),
        }
    })
    .await;
    let summary = run(
        &store,
        folder.path(),
        endpoint(&mock.base),
        Arc::new(Unattended),
    )
    .await
    .unwrap();
    assert_eq!(summary.body, "The draft summary.");
    let writers: Vec<String> = bodies(&mock)
        .into_iter()
        .filter(|b| turn_of(b) == Turn::Writer)
        .collect();
    assert_eq!(writers.len(), 3);
    for body in &bodies(&mock) {
        assert!(!body.contains("OTHER-NOTE-MARKER"), "{body}");
    }
    for writer in &writers {
        assert_eq!(tools_of(writer), ["get_note"]);
    }
    assert!(
        writers[1].contains("no note has the id other"),
        "{}",
        writers[1]
    );
    assert!(
        writers[2].contains("Sam owns the release notes."),
        "{}",
        writers[2]
    );
}

/// A control that the test cancels.
#[derive(Default)]
struct Switch(AtomicBool);

impl Control for Switch {
    fn proceed(&self) -> Result<(), StageError> {
        if self.0.load(Ordering::SeqCst) {
            Err(StageError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Covers R7: a cancel during the review gives `Cancelled`. The draft is
/// not the result, and no title request is stored.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_during_the_review_publishes_nothing() {
    let (store, folder) = note();
    let control = Arc::new(Switch::default());
    let cancel = Arc::clone(&control);
    let mock = serve(move |body| {
        let turn = turn_of(body);
        if turn == Turn::Audit {
            cancel.0.store(true, Ordering::SeqCst);
            // A finding, so that a run that missed the cancel would send
            // the reviser.
            return ok("missed: the date of the launch.");
        }
        ok(normal(turn))
    })
    .await;
    let result = run(&store, folder.path(), endpoint(&mock.base), control).await;
    assert!(matches!(result, Err(StageError::Cancelled)), "{result:?}");
    assert!(!turns(&mock).contains(&Turn::Revise));
    assert!(
        checkpoint::get::<Value>(&store, NOTE, Stage::Summarize, TITLE_REQUEST)
            .unwrap()
            .is_none()
    );
}

/// Covers R6: a run that fails at the writer keeps its timeline part. The
/// next run sends only the writer and the turns after it.
#[tokio::test(flavor = "multi_thread")]
async fn a_retry_after_a_writer_failure_sends_only_the_writer_and_later_turns() {
    let (store, folder) = note();
    let fail = Arc::new(AtomicBool::new(true));
    let failing = Arc::clone(&fail);
    let mock = serve(move |body| match turn_of(body) {
        Turn::Writer if failing.load(Ordering::SeqCst) => server_error(),
        turn => ok(normal(turn)),
    })
    .await;
    let failed = run(
        &store,
        folder.path(),
        endpoint(&mock.base),
        Arc::new(Unattended),
    )
    .await;
    let Err(StageError::Failed(message)) = failed else {
        panic!("{failed:?}");
    };
    assert!(message.contains("HTTP 500"), "{message}");
    assert_eq!(turns(&mock), [Turn::Timeline, Turn::Writer]);

    fail.store(false, Ordering::SeqCst);
    let summary = run(
        &store,
        folder.path(),
        endpoint(&mock.base),
        Arc::new(Unattended),
    )
    .await
    .unwrap();
    assert_eq!(summary.body, "The draft summary.");
    assert_eq!(
        turns(&mock)[2..],
        [Turn::Writer, Turn::Audit],
        "the timeline part comes from its saved turn"
    );
}

/// Covers R10, KTD10: a writer cut off, then a writer with no tools that
/// gives an empty reply, fails with the message for an outcome that the
/// user cannot act on, not with the cut-off message.
#[tokio::test(flavor = "multi_thread")]
async fn a_cut_off_writer_then_an_empty_writer_says_the_model_did_not_write_a_summary() {
    let (store, folder) = note();
    let mock = serve(|body| match turn_of(body) {
        Turn::Writer if !tools_of(body).is_empty() => (200, http::chat_reply("", "length")),
        Turn::Writer => ok(""),
        turn => ok(normal(turn)),
    })
    .await;
    let result = run(
        &store,
        folder.path(),
        endpoint(&mock.base),
        Arc::new(Unattended),
    )
    .await;
    let Err(StageError::Failed(message)) = result else {
        panic!("{result:?}");
    };
    assert_eq!(message, "the model did not write a summary");
    assert_eq!(turns(&mock), [Turn::Timeline, Turn::Writer, Turn::Writer]);
}

/// The value under `input` of an older build: the fingerprint of the
/// transcript and the notes, with no method tag.
fn older_input() -> String {
    session_core::folder::fingerprint(format!("{TRANSCRIPT}\0").as_bytes())
}

/// Covers R13, R15, KTD7: a run that an older build started has its plan,
/// its whole call and its layer calls. This build deletes them, starts the
/// summary again, and completes. It deletes no other key.
#[tokio::test(flavor = "multi_thread")]
async fn the_keys_of_an_older_build_are_deleted_and_the_summary_starts_again() {
    let (store, folder) = note();
    for key in ["plan", "whole", "l00-00000", "l01-00002"] {
        checkpoint::put(&store, NOTE, Stage::Summarize, key, &"an old result").unwrap();
    }
    checkpoint::put(&store, NOTE, Stage::Summarize, INPUT, &older_input()).unwrap();
    checkpoint::put(
        &store,
        NOTE,
        Stage::Summarize,
        PROMPT,
        &summarize::prompts::topic_summary(),
    )
    .unwrap();
    let mock = serve(|body| ok(normal(turn_of(body)))).await;
    let summary = run(
        &store,
        folder.path(),
        endpoint(&mock.base),
        Arc::new(Unattended),
    )
    .await
    .unwrap();
    assert_eq!(summary.body, "The draft summary.");
    assert_eq!(turns(&mock), [Turn::Timeline, Turn::Writer, Turn::Audit]);
    let keys = keys(&store);
    for old in ["plan", "whole", "l00-00000", "l01-00002"] {
        assert!(!keys.contains(&old.to_string()), "{keys:?}");
    }
    for kept in [INPUT, PROMPT, BUDGET, "source-notes", TITLE_REQUEST] {
        assert!(keys.contains(&kept.to_string()), "{kept}: {keys:?}");
    }
    assert!(keys.iter().any(|k| k.starts_with("turn-")), "{keys:?}");
}

/// Covers R14, KTD7: after a run of this build, an older build reads the
/// value under `input` as changed, so it forgets its own calls. No key of
/// this build has the form of a key that an older build reads or deletes
/// by its layer rule (`l` and two digits), so the two methods never mix.
#[tokio::test(flavor = "multi_thread")]
async fn an_older_build_reads_the_input_as_changed_and_finds_none_of_its_keys() {
    let (store, folder) = note();
    let mock = serve(|body| ok(normal(turn_of(body)))).await;
    run(
        &store,
        folder.path(),
        endpoint(&mock.base),
        Arc::new(Unattended),
    )
    .await
    .unwrap();
    let input: String = checkpoint::get(&store, NOTE, Stage::Summarize, INPUT)
        .unwrap()
        .unwrap();
    assert_ne!(input, older_input());
    for key in keys(&store) {
        assert!(key != "plan" && key != "whole", "{key}");
        // The layer rule of an older build's `forget_layers_from`.
        let layer = key
            .strip_prefix('l')
            .and_then(|k| k.get(..2))
            .and_then(|l| l.parse::<usize>().ok());
        assert_eq!(layer, None, "{key}");
    }
}

/// The saved budget of [`NOTE`].
fn saved_budget(store: &SharedStore) -> Value {
    checkpoint::get(store, NOTE, Stage::Summarize, BUDGET)
        .unwrap()
        .unwrap()
}

/// Covers R16, KTD6: an OpenAI endpoint with automatic detection uses the
/// input limit of Rig's catalog. A limit that the user set wins over it.
#[tokio::test(flavor = "multi_thread")]
async fn an_openai_model_gets_the_catalog_input_limit_and_a_set_limit_wins() {
    let catalog = summarize::context::catalog_input_limit(Provider::OpenAi, "gpt-6-luna")
        .expect("the catalog knows the model");
    for (set, limit, source) in [(None, catalog, "catalog"), (Some(50_000), 50_000, "user")] {
        let (store, folder) = note();
        let mock = serve(|body| ok(normal(turn_of(body)))).await;
        let openai = SummaryEndpoint {
            provider: Provider::OpenAi,
            model: "gpt-6-luna".into(),
            context_limit: set,
            ..endpoint(&mock.base)
        };
        run(&store, folder.path(), openai, Arc::new(Unattended))
            .await
            .unwrap();
        let budget = saved_budget(&store);
        assert_eq!(budget["budget"]["limit"], limit, "{budget}");
        assert_eq!(budget["budget"]["source"], source, "{budget}");
        // OpenAI reports no limit, so no probe is sent.
        assert!(
            mock.seen.lock().unwrap().iter().all(|s| s.path == PATH),
            "a probe was sent"
        );
    }
}

/// Covers R17: a note with nothing transcribed sends no request, and its
/// summary says so.
#[tokio::test(flavor = "multi_thread")]
async fn nothing_transcribed_sends_no_request() {
    let (store, folder) = note();
    std::fs::write(folder.path().join("transcript.md"), "# Transcript\n").unwrap();
    let mock = serve(|body| ok(normal(turn_of(body)))).await;
    let summary = run(
        &store,
        folder.path(),
        endpoint(&mock.base),
        Arc::new(Unattended),
    )
    .await
    .unwrap();
    assert_eq!(summary.body, "Nothing was transcribed in this note.");
    assert!(mock.seen.lock().unwrap().is_empty());
}
