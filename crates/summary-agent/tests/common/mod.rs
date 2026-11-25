//! A scripted model and a scripted tool source for loop tests.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Mutex;

use rig_core::completion::{AssistantContent, CompletionResponse, Message, ToolDefinition, Usage};
use rig_core::message::{Origin, ToolName};
use serde_json::{Value, json};
use summarize::client::{Label, RequestKind};
use summarize::{AgentReply, AgentTurn, SummarizeError};
use summary_agent::{BoxFuture, CallResult, Model, ToolSource};

pub fn label() -> Label {
    Label::note("n1", RequestKind::AgentTurn)
}

/// A reply with `choice` and the usage `input`/`output`.
pub fn reply(choice: Vec<AssistantContent>, input: u64, output: u64) -> AgentReply {
    let usage = Usage::new().input_tokens(input).output_tokens(output);
    AgentReply {
        response: CompletionResponse::new(
            choice,
            usage,
            Origin::new("chat", "mock", "m"),
            json!({}),
        ),
        served_by: Some("Host".into()),
        cost: Some(0.001),
    }
}

pub fn text(t: &str) -> AssistantContent {
    AssistantContent::text(t)
}

pub fn call(id: &str, name: &str, arguments: Value) -> AssistantContent {
    AssistantContent::tool_call(id, ToolName::new(name).unwrap(), arguments)
}

/// A call whose arguments are not a JSON object.
pub fn malformed(id: &str, name: &str) -> AssistantContent {
    AssistantContent::tool_call(id, ToolName::new(name).unwrap(), json!("{not json"))
}

/// A model that gives scripted replies in order and keeps each turn that
/// it got.
#[derive(Default)]
pub struct Script {
    replies: Mutex<VecDeque<Result<AgentReply, SummarizeError>>>,
    pub seen: Mutex<Vec<AgentTurn>>,
}

impl Script {
    pub fn new(replies: Vec<Result<AgentReply, SummarizeError>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            seen: Mutex::default(),
        }
    }

    pub fn turns(&self) -> Vec<AgentTurn> {
        self.seen.lock().unwrap().clone()
    }
}

impl Model for Script {
    fn turn<'a>(
        &'a self,
        _label: &'a Label,
        turn: &'a AgentTurn,
    ) -> BoxFuture<'a, Result<AgentReply, SummarizeError>> {
        self.seen.lock().unwrap().push(turn.clone());
        let next = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(reply(vec![text("the end")], 10, 10)));
        Box::pin(async move { next })
    }
}

/// A source with the tools `names`. Each call returns `result`.
pub struct Fixed {
    pub names: Vec<&'static str>,
    pub result: String,
}

impl Fixed {
    pub fn notes(result: &str) -> Self {
        Self {
            names: vec!["get_note", "list_notes"],
            result: result.to_string(),
        }
    }
}

impl ToolSource for Fixed {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.names
            .iter()
            .map(|n| {
                ToolDefinition::new(
                    ToolName::new(*n).unwrap(),
                    "a tool",
                    json!({"type": "object"}),
                )
            })
            .collect()
    }

    fn call<'a>(&'a self, _name: &'a str, _arguments: Value) -> BoxFuture<'a, CallResult> {
        let result = self.result.clone();
        Box::pin(async move { CallResult::ok(result) })
    }
}

/// The text of every tool result in the messages of `turn`.
pub fn tool_results(turn: &AgentTurn) -> Vec<String> {
    let mut out = Vec::new();
    for message in turn.history.iter().chain([&turn.prompt]) {
        if let Message::User { content } = message {
            for item in content {
                if let rig_core::message::UserContent::ToolResult(result) = item {
                    let text: String = result
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            rig_core::message::ToolResultContent::Text(t) => Some(t.text()),
                            _ => None,
                        })
                        .collect();
                    out.push(text);
                }
            }
        }
    }
    out
}
