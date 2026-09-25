//! The model list of an endpoint (KTD1, KTD2). One GET of `{base}/models`
//! under the probe limits, with no redirect followed, so the list request
//! obeys the same rules as a summary request (R8). The result is a typed
//! outcome for the Settings status line (R14), not an error string.
//!
//! The parser accepts `data[].id`, then `data[].name`, then a bare array
//! of strings. A 2xx body with none of these shapes is "not listed", which
//! is different from a list with no models.

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{Client, SummarizeError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ModelList {
    /// The server listed these ids, sorted without regard to case. The
    /// list can be empty.
    Listed { ids: Vec<String> },
    /// The server refused the request, and no key was sent.
    KeyNeeded,
    /// The server refused the key (401).
    KeyRejected,
    /// The server accepted the key but refused the list (403). OpenAI
    /// restricted keys can chat but cannot list models.
    KeyCannotList,
    /// The server has no model list at this Base URL.
    NotListed,
    /// The server sent a redirect. `host` is the host that it points to.
    Redirected { host: Option<String> },
    /// Any other failure status, or a reply over the byte cap.
    ServerError { status: Option<u16> },
    /// No connection, or no reply within the deadline.
    Unreachable,
}

/// The ids in a list body, without duplicates and sorted without regard to
/// case. `None` when the body has no list.
pub fn parse_ids(body: &Value) -> Option<Vec<String>> {
    let mut ids: Vec<String> = if let Some(rows) = body.get("data").and_then(Value::as_array) {
        rows.iter()
            .filter_map(|row| {
                row.get("id")
                    .and_then(Value::as_str)
                    .or_else(|| row.get("name").and_then(Value::as_str))
            })
            .map(str::to_string)
            .collect()
    } else if let Some(items) = body.as_array() {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    } else {
        return None;
    };
    ids.retain(|id| !id.trim().is_empty());
    ids.sort_by_cached_key(|id| (id.to_lowercase(), id.clone()));
    ids.dedup();
    Some(ids)
}

/// Requests the model list of the client's endpoint and sends no chat
/// request (R5).
pub async fn list_models(client: &Client) -> ModelList {
    let url = client.api_url("models");
    let key_sent = client
        .endpoint()
        .credential
        .as_deref()
        .is_some_and(|key| !key.is_empty());
    match client.probe_json(&url).await {
        Ok(body) => match parse_ids(&body) {
            Some(ids) => ModelList::Listed { ids },
            None => ModelList::NotListed,
        },
        Err(e) => {
            log::info!("the model list request failed: {e}");
            outcome(e, key_sent, &url)
        }
    }
}

fn outcome(error: SummarizeError, key_sent: bool, url: &str) -> ModelList {
    match error {
        SummarizeError::Unauthorized(_) if !key_sent => ModelList::KeyNeeded,
        SummarizeError::Unauthorized(401) => ModelList::KeyRejected,
        SummarizeError::Unauthorized(_) => ModelList::KeyCannotList,
        SummarizeError::Http { status: 404, .. } | SummarizeError::Malformed(_) => {
            ModelList::NotListed
        }
        SummarizeError::Redirect { location, .. } => ModelList::Redirected {
            host: redirect_host(url, &location),
        },
        SummarizeError::Http { status, .. } => ModelList::ServerError {
            status: Some(status),
        },
        SummarizeError::Transport(_) | SummarizeError::Deadline(_) => ModelList::Unreachable,
        SummarizeError::TooLarge(_)
        | SummarizeError::BadUrl(_)
        | SummarizeError::ContextExceeded
        | SummarizeError::ReplyCutOff => ModelList::ServerError { status: None },
    }
}

/// The host of a redirect target. A relative `Location` resolves against
/// the request URL.
fn redirect_host(url: &str, location: &str) -> Option<String> {
    Url::parse(url)
        .and_then(|base| base.join(location))
        .ok()?
        .host_str()
        .map(str::to_string)
}
