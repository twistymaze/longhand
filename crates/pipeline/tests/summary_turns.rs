//! The turn layer of the summary pipeline (KTD3, KTD4): a saved reply for
//! each model turn, the control before each turn, the cancel flag and the
//! typed error of a failed turn.

mod common;
#[path = "../../summarize/tests/common/mod.rs"]
mod http;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use http::{Reply, chat_reply, serve};
use pipeline::summarize::turns::{TURN_PREFIX, Turns};
use pipeline::{Control, SharedStore, StageError, Unattended, checkpoint};
use rig_core::completion::{AssistantContent, AssistantMessage, Message, ToolDefinition};
use rig_core::message::ToolName;
use serde_json::{Map, Value, json};
use session_core::Stage;
use summarize::client::{Label, Limits, RequestKind, Thinking};
use summarize::provider::ThinkingLevel;
use summarize::{AgentTurn, Client, Endpoint, Provider, SummarizeError};
use summary_agent::Model;

const NOTE: &str = "n1";
const PATH: &str = "/v1/chat/completions";

fn note_store() -> SharedStore {
    let store = common::store();
    store
        .lock()
        .unwrap()
        .create_session(NOTE, common::at(0), None)
        .unwrap();
    store
}

fn endpoint(base: &str, model: &str) -> Endpoint {
    Endpoint {
        provider: Provider::Custom,
        base_url: base.into(),
        model: model.into(),
        credential: None,
    }
}

fn client(base: &str) -> Client {
    Client::new(endpoint(base, "m")).unwrap()
}

fn label() -> Label {
    Label::note(NOTE, RequestKind::Timeline)
}

fn name(text: &str) -> ToolName {
    ToolName::new(text).unwrap()
}

/// The tool definition of `get_note` with its parameter keys in the order
/// of `keys`.
fn get_note(keys: &[&str]) -> ToolDefinition {
    let mut properties = Map::new();
    for key in keys {
        properties.insert((*key).to_string(), json!({"type": "integer"}));
    }
    ToolDefinition::new(
        name("get_note"),
        "Read lines of the note.",
        json!({"type": "object", "properties": Value::Object(properties)}),
    )
}

/// A turn after a call of `get_note` with the arguments `first` and `last`
/// in the order of `keys`.
fn turn_after_a_call(keys: &[&str]) -> AgentTurn {
    let mut arguments = Map::new();
    for key in keys {
        let value = if *key == "first" { 1 } else { 40 };
        arguments.insert((*key).to_string(), json!(value));
    }
    let call = AssistantContent::tool_call("call_1", name("get_note"), Value::Object(arguments));
    let AssistantContent::ToolCall(tool_call) = &call else {
        unreachable!()
    };
    let result = Message::tool_result(tool_call.id.clone(), name("get_note"), "1 line one");
    AgentTurn {
        preamble: "the rules".into(),
        history: vec![
            Message::user("write the timeline"),
            Message::Assistant(AssistantMessage::new(vec![call])),
        ],
        prompt: result,
        tools: vec![get_note(keys)],
        max_tokens: 500,
        num_ctx: Some(8_192),
    }
}

fn first_turn(text: &str) -> AgentTurn {
    AgentTurn {
        preamble: "the rules".into(),
        history: Vec::new(),
        prompt: Message::user(text),
        tools: vec![get_note(&["first", "last"])],
        max_tokens: 500,
        num_ctx: Some(8_192),
    }
}

fn sent(mock: &http::Mock) -> usize {
    mock.seen.lock().unwrap().len()
}

async fn answering(text: &str) -> http::Mock {
    serve(HashMap::from([(
        PATH,
        Reply::Json(200, chat_reply(text, "stop")),
    )]))
    .await
}

/// The keys of the saved summary rows of the note.
fn saved_keys(store: &SharedStore) -> Vec<String> {
    checkpoint::all(store, NOTE, Stage::Summarize)
        .unwrap()
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

#[tokio::test]
async fn a_second_run_with_the_same_inputs_sends_no_request_for_completed_turns() {
    let mock = answering("a timeline").await;
    let store = note_store();
    let client = client(&mock.base);
    let turns = [first_turn("part one"), first_turn("part two")];
    let first_run = Turns::new(&store, NOTE, &client, &Unattended);
    let mut replies = Vec::new();
    for turn in &turns {
        replies.push(first_run.turn(&label(), turn).await.unwrap());
    }
    assert_eq!(sent(&mock), 2);

    let second_run = Turns::new(&store, NOTE, &client, &Unattended);
    for (turn, reply) in turns.iter().zip(&replies) {
        let again = second_run.turn(&label(), turn).await.unwrap();
        assert_eq!(again.response.text(), reply.response.text());
    }
    assert_eq!(sent(&mock), 2, "the saved replies are used again");
}

#[tokio::test]
async fn a_changed_thinking_level_model_or_context_size_misses_every_saved_turn() {
    let mock = answering("a timeline").await;
    let store = note_store();
    let base = client(&mock.base);
    let turn = first_turn("part one");
    Turns::new(&store, NOTE, &base, &Unattended)
        .turn(&label(), &turn)
        .await
        .unwrap();
    assert_eq!(sent(&mock), 1);

    let thinking = client(&mock.base).with_thinking(Thinking {
        level: ThinkingLevel::High,
        chosen: true,
    });
    Turns::new(&store, NOTE, &thinking, &Unattended)
        .turn(&label(), &turn)
        .await
        .unwrap();
    assert_eq!(sent(&mock), 2, "another Thinking level");

    let model = Client::new(endpoint(&mock.base, "other")).unwrap();
    Turns::new(&store, NOTE, &model, &Unattended)
        .turn(&label(), &turn)
        .await
        .unwrap();
    assert_eq!(sent(&mock), 3, "another model");

    let larger = AgentTurn {
        num_ctx: Some(16_384),
        ..turn.clone()
    };
    Turns::new(&store, NOTE, &base, &Unattended)
        .turn(&label(), &larger)
        .await
        .unwrap();
    assert_eq!(sent(&mock), 4, "another context size");

    // The first turn is still saved for its own inputs.
    Turns::new(&store, NOTE, &base, &Unattended)
        .turn(&label(), &turn)
        .await
        .unwrap();
    assert_eq!(sent(&mock), 4);
}

#[test]
fn the_key_of_a_turn_does_not_change_with_the_json_key_order_of_its_messages() {
    let store = note_store();
    let client = client("http://127.0.0.1:9/v1");
    let turns = Turns::new(&store, NOTE, &client, &Unattended);
    let in_order = turn_after_a_call(&["first", "last"]);
    let reversed = turn_after_a_call(&["last", "first"]);
    // The JSON of the two turns is not the same, so a key made from it
    // would not be the same.
    assert_ne!(
        serde_json::to_string(&in_order.history).unwrap(),
        serde_json::to_string(&reversed.history).unwrap()
    );
    let key = turns.key(&in_order);
    assert!(key.starts_with(TURN_PREFIX), "{key}");
    assert_eq!(key, turns.key(&reversed));
    assert_ne!(key, turns.key(&first_turn("write the timeline")));
}

/// A control that the test cancels.
struct Cancelled;

impl Control for Cancelled {
    fn proceed(&self) -> Result<(), StageError> {
        Err(StageError::Cancelled)
    }

    fn is_cancelled(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn a_cancel_in_proceed_sets_the_flag_and_sends_no_request() {
    let mock = answering("a timeline").await;
    let store = note_store();
    let client = client(&mock.base);
    let turns = Turns::new(&store, NOTE, &client, &Cancelled);
    assert!(!turns.cancelled());
    let result = turns.turn(&label(), &first_turn("part one")).await;
    assert!(result.is_err());
    assert!(turns.cancelled());
    assert_eq!(sent(&mock), 0);
}

/// A control that holds each turn until the test resumes it, as a
/// suspension does.
struct Held {
    entered: Mutex<Sender<()>>,
    resume: Mutex<Receiver<()>>,
}

impl Control for Held {
    fn proceed(&self) -> Result<(), StageError> {
        self.entered.lock().unwrap().send(()).unwrap();
        self.resume.lock().unwrap().recv().unwrap();
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_suspended_proceed_holds_the_turn_until_it_is_resumed() {
    let mock = answering("a timeline").await;
    let store = note_store();
    let client = client(&mock.base);
    let (entered_tx, entered) = channel();
    let (resume, resume_rx) = channel();
    let control = Held {
        entered: Mutex::new(entered_tx),
        resume: Mutex::new(resume_rx),
    };
    let turns = Turns::new(&store, NOTE, &client, &control);
    let turn = first_turn("part one");
    std::thread::scope(|scope| {
        let running = scope.spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(turns.turn(&label(), &turn))
        });
        // The time limit only ends a test that fails: a turn that never
        // calls `proceed` would hold the test for ever.
        if entered.recv_timeout(Duration::from_secs(30)).is_err() {
            eprintln!("the turn did not call proceed");
            std::process::abort();
        }
        assert_eq!(sent(&mock), 0, "no request while the turn is held");
        resume.send(()).unwrap();
        running.join().unwrap().unwrap();
    });
    assert_eq!(sent(&mock), 1);
}

#[tokio::test]
async fn a_deadline_error_is_kept_as_a_typed_error_until_it_is_cleared() {
    let mock = serve(HashMap::from([(PATH, Reply::Hang)])).await;
    let quick = Limits {
        connect: Duration::from_secs(60),
        response: Duration::from_millis(300),
        idle: Duration::from_secs(60),
        total: Duration::from_secs(60),
        max_bytes: 1 << 20,
    };
    let store = note_store();
    let client = Client::with_limits(endpoint(&mock.base, "m"), quick, quick).unwrap();
    let turns = Turns::new(&store, NOTE, &client, &Unattended);
    assert!(turns.error().is_none());
    let result = turns.turn(&label(), &first_turn("part one")).await;
    assert!(matches!(result, Err(SummarizeError::Deadline(_))));
    assert!(
        matches!(turns.error(), Some(SummarizeError::Deadline("response"))),
        "{:?}",
        turns.error()
    );
    assert!(!turns.cancelled());
    turns.clear_error();
    assert!(turns.error().is_none());
}

#[tokio::test]
async fn a_saved_turn_that_does_not_decode_is_sent_again_with_no_failure() {
    let mock = answering("a timeline").await;
    let store = note_store();
    let client = client(&mock.base);
    let turns = Turns::new(&store, NOTE, &client, &Unattended);
    let turn = first_turn("part one");
    let key = turns.key(&turn);
    store
        .lock()
        .unwrap()
        .put_checkpoint(NOTE, Stage::Summarize, &key, "{\"not\": \"a reply\"}")
        .unwrap();

    let reply = turns.turn(&label(), &turn).await.unwrap();
    assert_eq!(reply.response.text(), "a timeline");
    assert_eq!(sent(&mock), 1);
    // The new reply replaced the row, so the next run sends nothing.
    Turns::new(&store, NOTE, &client, &Unattended)
        .turn(&label(), &turn)
        .await
        .unwrap();
    assert_eq!(sent(&mock), 1);
}

#[tokio::test]
async fn no_saved_key_has_the_form_of_an_older_build_layer_key() {
    let mock = answering("a timeline").await;
    let store = note_store();
    let client = client(&mock.base);
    let turns = Turns::new(&store, NOTE, &client, &Unattended);
    for text in ["part one", "part two", "part three"] {
        turns.turn(&label(), &first_turn(text)).await.unwrap();
    }
    let keys = saved_keys(&store);
    assert_eq!(keys.len(), 3, "{keys:?}");
    for key in keys {
        assert!(key.starts_with(TURN_PREFIX), "{key}");
        let older = key.len() >= 3
            && key.starts_with('l')
            && key.as_bytes()[1..3].iter().all(u8::is_ascii_digit);
        assert!(!older, "{key}");
    }
}
