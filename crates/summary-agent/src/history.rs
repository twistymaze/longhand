//! The history policy (R18): what the history of an agent run sends to the
//! model. `keep_all` sends every message. `compact` sends every message
//! until the context is nearly full; the loop then compacts
//! ([`crate::compaction`]). `trim(n)` replaces the content of
//! each tool result older than the last `n` tool-result messages with a
//! short placeholder that keeps the tool name and, for a transcript read,
//! the line range. The run itself keeps every message: the policy changes
//! only what one request sends.

use std::sync::Arc;

use rig_core::completion::Message;
use rig_core::message::{ToolResultContent, UserContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The history policy of an agent run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryPolicy {
    /// Send every message. A request that does not fit ends the run.
    KeepAll,
    /// Keep the last `n` tool-result messages whole.
    Trim(usize),
    /// Send every message until the context is nearly full, then replace
    /// the older messages with notes ([`crate::compaction`]).
    #[default]
    Compact,
}

fn has_tool_result(message: &Message) -> bool {
    matches!(message, Message::User { content } if content.iter().any(|c| matches!(c, UserContent::ToolResult(_))))
}

/// The line range of a get note result, from its transcript hunks.
fn line_range(text: &str) -> Option<(u64, u64)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let hunks = value.pointer("/transcript/hunks")?.as_array()?;
    let first = hunks.first()?.get("first_line")?.as_u64()?;
    let last = hunks.last()?.get("last_line")?.as_u64()?;
    Some((first, last))
}

fn placeholder(name: &str, text: &str) -> String {
    match line_range(text) {
        Some((first, last)) => format!(
            "[This {name} result was removed to save room. It held transcript lines {first} to {last}.]"
        ),
        None => format!("[This {name} result was removed to save room.]"),
    }
}

/// The messages that a request sends under `policy`.
pub fn apply(policy: HistoryPolicy, history: &[Arc<Message>]) -> Vec<Message> {
    let keep = match policy {
        HistoryPolicy::KeepAll | HistoryPolicy::Compact => {
            return history.iter().map(|m| (**m).clone()).collect();
        }
        HistoryPolicy::Trim(n) => n,
    };
    let with_results: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, m)| has_tool_result(m))
        .map(|(i, _)| i)
        .collect();
    // With keep 0, the cut is after the last message.
    let cut_before = match with_results.len().checked_sub(keep) {
        Some(i) => with_results.get(i).copied().unwrap_or(history.len()),
        None => 0,
    };
    history
        .iter()
        .enumerate()
        .map(|(i, message)| {
            let message = (**message).clone();
            if i >= cut_before || !has_tool_result(&message) {
                return message;
            }
            let Message::User { content } = message else {
                return message;
            };
            let content = content
                .into_iter()
                .map(|item| match item {
                    UserContent::ToolResult(mut result) => {
                        let text: String = result
                            .content
                            .iter()
                            .filter_map(|c| match c {
                                ToolResultContent::Text(t) => Some(t.text()),
                                _ => None,
                            })
                            .collect();
                        result.content = vec![ToolResultContent::text(placeholder(
                            result.name.as_str(),
                            &text,
                        ))];
                        UserContent::ToolResult(result)
                    }
                    other => other,
                })
                .collect();
            Message::User { content }
        })
        .collect()
}
