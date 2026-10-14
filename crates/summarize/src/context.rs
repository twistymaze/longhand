//! The context budget (KTD16). No field of `/v1/models` is portable, so the
//! limit is probed across the servers people actually run, in order, and
//! each answer is read defensively: a gateway may list the model with no
//! context metadata at all (an alias row, a locally served model), and a
//! partial answer is a miss, not an error. The user's setting wins; with
//! neither, 8,192, the smallest window a current server ships with.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{Client, SummarizeError};

pub const DEFAULT_LIMIT: u32 = 8_192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetSource {
    User,
    /// A server rejected a prompt the estimate believed would fit.
    Learned,
    Litellm,
    LmStudio,
    Vllm,
    LlamaCpp,
    Openrouter,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub limit: u32,
    pub source: BudgetSource,
}

/// The setting, else the probe, else the default; a learned limit below
/// the result caps it, since the server has shown it takes no more.
pub fn resolve(user: Option<u32>, probed: Option<Budget>, learned: Option<u32>) -> Budget {
    let base = match (user, probed) {
        (Some(limit), _) => Budget {
            limit,
            source: BudgetSource::User,
        },
        (None, Some(probed)) => probed,
        (None, None) => Budget {
            limit: DEFAULT_LIMIT,
            source: BudgetSource::Default,
        },
    };
    match learned {
        Some(limit) if limit < base.limit => Budget {
            limit,
            source: BudgetSource::Learned,
        },
        _ => base,
    }
}

fn positive(v: Option<&Value>) -> Option<u32> {
    v.and_then(Value::as_u64)
        .filter(|&n| n > 0)
        .map(|n| n.min(u32::MAX as u64) as u32)
}

fn rows(body: &Value) -> impl Iterator<Item = &Value> {
    body.get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// LiteLLM `/model/info`: `model_info.max_input_tokens`, never
/// `max_tokens`, which is the model's output cap.
pub fn parse_litellm(body: &Value, model: &str) -> Option<u32> {
    rows(body)
        .filter(|row| {
            row.get("model_name").and_then(Value::as_str) == Some(model)
                || row.pointer("/model_info/key").and_then(Value::as_str) == Some(model)
        })
        .find_map(|row| positive(row.pointer("/model_info/max_input_tokens")))
}

/// LM Studio `/api/v0/models`: the context the model is loaded with.
pub fn parse_lm_studio(body: &Value, model: &str) -> Option<u32> {
    rows(body)
        .filter(|row| row.get("id").and_then(Value::as_str) == Some(model))
        .find_map(|row| positive(row.get("loaded_context_length")))
}

/// vLLM `/v1/models`: `max_model_len`.
pub fn parse_vllm(body: &Value, model: &str) -> Option<u32> {
    rows(body)
        .filter(|row| row.get("id").and_then(Value::as_str) == Some(model))
        .find_map(|row| positive(row.get("max_model_len")))
}

/// OpenRouter `/v1/models`: `context_length`.
pub fn parse_openrouter(body: &Value, model: &str) -> Option<u32> {
    rows(body)
        .filter(|row| row.get("id").and_then(Value::as_str) == Some(model))
        .find_map(|row| {
            positive(row.get("context_length"))
                .or_else(|| positive(row.pointer("/top_provider/context_length")))
        })
}

/// llama.cpp `/props`: the server's `n_ctx` (one model per server).
pub fn parse_llama_cpp(body: &Value) -> Option<u32> {
    positive(body.pointer("/default_generation_settings/n_ctx"))
        .or_else(|| positive(body.get("n_ctx")))
}

/// Ask the endpoint, in order, until one answer names the model's context.
/// `Ok(None)` when the endpoint answers and no answer names it. An error
/// when nothing is known about the context, so that the default budget is
/// not kept for the run: the first probe cannot reach the endpoint, or a
/// probe gets a server error ([`is_server_problem`]). After the endpoint
/// answered once, a later probe that cannot connect is a miss.
pub async fn probe(client: &Client) -> Result<Option<Budget>, SummarizeError> {
    let model = client.endpoint().model.clone();
    let found = |limit: Option<u32>, source| limit.map(|limit| Budget { limit, source });

    let first = answer(client.probe_json(&client.api_url("model/info")).await, true)?;
    if let Some(body) = first
        && let Some(b) = found(parse_litellm(&body, &model), BudgetSource::Litellm)
    {
        return Ok(Some(b));
    }
    let lm_studio = client.probe_json(&client.root_url("api/v0/models")).await;
    if let Some(body) = answer(lm_studio, false)?
        && let Some(b) = found(parse_lm_studio(&body, &model), BudgetSource::LmStudio)
    {
        return Ok(Some(b));
    }
    if let Some(body) = answer(client.probe_json(&client.api_url("models")).await, false)? {
        if let Some(b) = found(parse_vllm(&body, &model), BudgetSource::Vllm) {
            return Ok(Some(b));
        }
        if let Some(b) = found(parse_openrouter(&body, &model), BudgetSource::Openrouter) {
            return Ok(Some(b));
        }
    }
    if let Some(body) = answer(client.probe_json(&client.root_url("props")).await, false)?
        && let Some(b) = found(parse_llama_cpp(&body), BudgetSource::LlamaCpp)
    {
        return Ok(Some(b));
    }
    Ok(None)
}

/// The body of a probe's answer, `None` for a miss, or the error that stops
/// the probe. `first` is true for the first probe of the chain.
fn answer(
    result: Result<Value, SummarizeError>,
    first: bool,
) -> Result<Option<Value>, SummarizeError> {
    match result {
        Ok(body) => Ok(Some(body)),
        Err(e @ SummarizeError::Transport(_)) if first => Err(e),
        Err(e) if is_server_problem(&e) => Err(e),
        Err(_) => Ok(None),
    }
}

/// A server error (5xx), a timeout (408) or a rate limit (429). The server
/// has a problem now, so its answer tells nothing about the context. A
/// route that the server does not have (404) is a miss.
fn is_server_problem(e: &SummarizeError) -> bool {
    matches!(e, SummarizeError::Http { status, .. } if *status >= 500 || matches!(status, 408 | 429))
}
