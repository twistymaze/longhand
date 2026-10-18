//! The model list of an endpoint (KTD5), and the connection check of the
//! Settings pane (KTD9).
//!
//! Rig sends the list request of each provider through the Longhand
//! [`crate::Transport`] under the probe limits, with no redirect followed,
//! so the list request obeys the same rules as a summary request (R3). The
//! result is a typed outcome for the Settings status row (R17), not an
//! error string.
//!
//! For an OpenAI-compatible server, Longhand reads the ids from the reply
//! body itself, because Rig reads only `data[].id`. The parser accepts
//! `data[].id`, then `data[].name`, then a bare array of strings. A 2xx
//! body with none of these shapes is "not listed", which is different from
//! a list with no models. For Anthropic, Gemini and Ollama, Longhand uses
//! the ids that Rig reads.

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{Client, Endpoint, Label, NO_LOCATION, RequestKind, SummarizeError};
use crate::provider::{Provider, Route};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ModelList {
    /// The server listed these ids, sorted without regard to case. The
    /// list can be empty.
    Listed { ids: Vec<String> },
    /// The server refused the request, and no key was sent.
    KeyNeeded,
    /// The server refused the key (401, or Gemini's invalid key reply).
    KeyRejected,
    /// The server accepted the key but refused the list (403). OpenAI
    /// restricted keys can chat but cannot list models.
    KeyCannotList,
    /// The server has no model list at this Base URL.
    NotListed,
    /// The server has no model list, but its chat route answers. The Model
    /// row is then a text field (R17).
    ChatOnly,
    /// The server sent a redirect. `host` is the host that it points to.
    Redirected { host: Option<String> },
    /// Any other failure status, or a reply over the byte cap.
    ServerError { status: Option<u16> },
    /// No connection, or no reply within the deadline.
    Unreachable,
}

/// The result of the connection check (KTD9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub list: ModelList,
    /// The address that worked: the address as typed, or for Custom the
    /// address with `/v1` added (R14). `None` when no address worked.
    pub address: Option<String>,
}

/// The ids in a list body, without duplicates and sorted without regard to
/// case. `None` when the body has no list.
pub fn parse_ids(body: &Value) -> Option<Vec<String>> {
    let ids: Vec<String> = if let Some(rows) = body.get("data").and_then(Value::as_array) {
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
    Some(tidy(ids))
}

/// `ids` with no blank id and no duplicate, sorted without regard to case.
fn tidy(mut ids: Vec<String>) -> Vec<String> {
    ids.retain(|id| !id.trim().is_empty());
    ids.sort_by_cached_key(|id| (id.to_lowercase(), id.clone()));
    ids.dedup();
    ids
}

/// Requests the model list of the client's endpoint at its address, and
/// sends no chat request (R5).
pub async fn list_models(client: &Client) -> ModelList {
    let provider = client.endpoint().provider;
    let listing = client
        .rig_models(&Label::without_note(RequestKind::Models))
        .await;
    if provider.route() == Route::OpenAiChat
        && let Some(body) = &listing.body
    {
        let ids = serde_json::from_slice::<Value>(body)
            .ok()
            .as_ref()
            .and_then(parse_ids);
        return match ids {
            Some(ids) => ModelList::Listed { ids },
            None => ModelList::NotListed,
        };
    }
    match listing.models {
        Ok(list) => ModelList::Listed {
            ids: tidy(list.into_iter().map(|model| model.id).collect()),
        },
        Err(e) => {
            log::info!("the model list request failed: {e}");
            outcome(e, provider, client.sends_key(), &client.api_url("models"))
        }
    }
}

/// The connection check of the Settings pane (KTD9). It lists the models
/// at the address. For Custom, when that address has no model list and
/// does not end in `/v1`, it lists the models at the address with `/v1`
/// added. When neither address has a model list, it sends one chat
/// request with a one-token cap, and uses the address whose chat route
/// does not answer 404. An address that works as typed is never changed
/// (R14). A redirect, a TLS failure or a refused key starts no retry.
pub async fn check(client: &Client) -> Checked {
    let typed = client.address().to_string();
    let connected = |list, address: &str| Checked {
        list,
        address: Some(address.to_string()),
    };
    let failed = |list| Checked {
        list,
        address: None,
    };
    match list_models(client).await {
        list @ ModelList::Listed { .. } => return connected(list, &typed),
        ModelList::NotListed if client.endpoint().provider == Provider::Custom => {}
        list => return failed(list),
    }
    let with_v1 = (!typed.trim_end_matches('/').ends_with("/v1"))
        .then(|| format!("{}/v1", typed.trim_end_matches('/')));
    if let Some(v1) = &with_v1 {
        let Some(other) = probe_client(client, v1) else {
            return failed(ModelList::NotListed);
        };
        match list_models(&other).await {
            list @ ModelList::Listed { .. } => return connected(list, v1),
            ModelList::NotListed => {}
            list => return failed(list),
        }
    }
    match chat(client, &typed).await {
        Chat::Answered => connected(ModelList::ChatOnly, &typed),
        Chat::Failed(list) => failed(list),
        Chat::NotFound => match &with_v1 {
            Some(v1) => match chat(client, v1).await {
                Chat::Answered => connected(ModelList::ChatOnly, v1),
                Chat::Failed(list) => failed(list),
                Chat::NotFound => failed(ModelList::NotListed),
            },
            None => failed(ModelList::NotListed),
        },
    }
}

/// A client for the endpoint of `client` at `address`, with the probe
/// limits for each request. `None` when `address` is not a valid URL.
fn probe_client(client: &Client, address: &str) -> Option<Client> {
    let limits = client.probe_limits();
    let endpoint = Endpoint {
        base_url: address.to_string(),
        ..client.endpoint().clone()
    };
    Client::with_limits(endpoint, limits, limits).ok()
}

/// How the chat route of an address answered the check request.
enum Chat {
    /// The route answered with a status that is not 404.
    Answered,
    /// The route answered 404.
    NotFound,
    /// The request failed before an answer, or the key was refused.
    Failed(ModelList),
}

async fn chat(client: &Client, address: &str) -> Chat {
    let Some(client) = probe_client(client, address) else {
        return Chat::Failed(ModelList::Unreachable);
    };
    let result = client
        .check_chat(&Label::without_note(RequestKind::Check))
        .await;
    match result {
        Err(SummarizeError::Http { status: 404, .. }) => Chat::NotFound,
        Err(SummarizeError::Unauthorized(_)) if !client.sends_key() => {
            Chat::Failed(ModelList::KeyNeeded)
        }
        Err(SummarizeError::Unauthorized(_)) => Chat::Failed(ModelList::KeyRejected),
        Err(
            e @ (SummarizeError::Redirect { .. }
            | SummarizeError::Transport(_)
            | SummarizeError::Deadline(_)
            | SummarizeError::TooLarge(_)
            | SummarizeError::BadUrl(_)),
        ) => {
            log::info!("the check request failed: {e}");
            Chat::Failed(outcome(
                e,
                Provider::Custom,
                client.sends_key(),
                &client.api_url("chat/completions"),
            ))
        }
        // A reply, a cut-off reply, an unreadable reply or another status:
        // the chat route is there.
        Ok(_)
        | Err(
            SummarizeError::Http { .. }
            | SummarizeError::ContextExceeded
            | SummarizeError::ReplyCutOff
            | SummarizeError::Malformed(_),
        ) => Chat::Answered,
    }
}

fn outcome(error: SummarizeError, provider: Provider, key_sent: bool, url: &str) -> ModelList {
    match error {
        SummarizeError::Unauthorized(_) if !key_sent => ModelList::KeyNeeded,
        // Gemini refuses a key that is not valid with 400 or 403.
        SummarizeError::Unauthorized(_) if provider == Provider::Gemini => ModelList::KeyRejected,
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
/// the request URL. A redirect with no `Location` header gives no host.
fn redirect_host(url: &str, location: &str) -> Option<String> {
    if location == NO_LOCATION {
        return None;
    }
    Url::parse(url)
        .and_then(|base| base.join(location))
        .ok()?
        .host_str()
        .map(str::to_string)
}
