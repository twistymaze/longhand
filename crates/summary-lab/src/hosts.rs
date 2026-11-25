//! `check-hosts`: which OpenRouter models have a host that runs tool calls
//! (U2). Each model gets a two-turn exchange with one small tool: the
//! first reply must call the tool, and the second reply must use its
//! result. The exchange holds no note text.

use rig_core::completion::{Message, ToolDefinition};
use rig_core::message::{ToolName, ToolResultContent};
use serde_json::json;
use summarize::client::{Label, RequestKind};
use summarize::{AgentTurn, Client, Endpoint, OpenRouterRouting, SummarizeError};

/// The number that the tool returns. The second reply must contain it.
const SECRET: &str = "4817";

/// The result of the check for one model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Both turns worked.
    ToolCapable,
    /// The endpoint refused the request: no host accepts the tools or the
    /// data rule. The text is the endpoint's reason.
    Refused(String),
    /// No reply in the tries called the tool.
    NoCall,
    /// The model called the tool, but its second reply did not use the
    /// result.
    ResultNotUsed,
    /// Another error stopped the check.
    Failed(String),
}

impl Verdict {
    pub fn text(&self) -> String {
        match self {
            Self::ToolCapable => "tool-capable".into(),
            Self::Refused(reason) => format!("refused by routing: {reason}"),
            Self::NoCall => "no call".into(),
            Self::ResultNotUsed => "called, but did not use the result".into(),
            Self::Failed(reason) => format!("failed: {reason}"),
        }
    }
}

/// The check result for one model, with the serving host of each turn.
#[derive(Debug, Clone)]
pub struct Check {
    pub model: String,
    pub verdict: Verdict,
    pub hosts: Vec<String>,
}

fn tool() -> ToolDefinition {
    ToolDefinition::new(
        ToolName::new("get_number").expect("a tool name"),
        "Return the secret number for a word.",
        json!({
            "type": "object",
            "properties": {"word": {"type": "string", "description": "The word."}},
            "required": ["word"]
        }),
    )
}

fn label() -> Label {
    Label::without_note(RequestKind::AgentTurn)
}

/// The verdict for an error. A 404 or a 400 from OpenRouter means that no
/// host accepts the request.
fn verdict_of(error: &SummarizeError) -> Verdict {
    match error {
        SummarizeError::Http {
            status: 404 | 400,
            excerpt,
        } => Verdict::Refused(excerpt.clone()),
        other => Verdict::Failed(other.to_string()),
    }
}

/// Run the check for the model entry `model` on `endpoint`. The routing
/// rules apply only to an OpenRouter endpoint.
pub async fn check(
    model: &str,
    endpoint: Endpoint,
    tries: u32,
    routing: OpenRouterRouting,
) -> Check {
    let mut hosts = Vec::new();
    let verdict = match Client::new(endpoint) {
        Ok(client) => exchange(&client.with_openrouter(routing), tries, &mut hosts).await,
        Err(error) => Verdict::Failed(error.to_string()),
    };
    Check {
        model: model.to_string(),
        verdict,
        hosts,
    }
}

async fn exchange(client: &Client, tries: u32, hosts: &mut Vec<String>) -> Verdict {
    let first = AgentTurn {
        preamble: "You answer with the help of tools.".into(),
        history: Vec::new(),
        prompt: Message::user(
            "Call the get_number tool with the word \"otter\". Then reply with the number that it returns.",
        ),
        tools: vec![tool()],
        max_tokens: 2_000,
        num_ctx: None,
    };
    let mut called = None;
    for _ in 0..tries {
        let reply = match client.agent_turn(&label(), &first).await {
            Ok(reply) => reply,
            Err(error) => return verdict_of(&error),
        };
        hosts.extend(reply.served_by.clone());
        if reply.response.tool_calls().next().is_some() {
            called = Some(reply);
            break;
        }
    }
    let Some(reply) = called else {
        return Verdict::NoCall;
    };
    let Some(message) = reply.response.message() else {
        return Verdict::Failed("the reply had no message".into());
    };
    let prompt = Message::tool_results(
        reply
            .response
            .tool_calls()
            .map(|call| {
                call.result(vec![ToolResultContent::text(format!(
                    "The number is {SECRET}."
                ))])
            })
            .collect(),
    );
    let second = AgentTurn {
        history: vec![first.prompt.clone(), message],
        prompt,
        ..first
    };
    match client.agent_turn(&label(), &second).await {
        Ok(reply) => {
            hosts.extend(reply.served_by.clone());
            if reply.response.text().contains(SECRET) {
                Verdict::ToolCapable
            } else {
                Verdict::ResultNotUsed
            }
        }
        Err(error) => verdict_of(&error),
    }
}
