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
/// when a probe cannot reach the endpoint: then nothing is known about
/// the context, and the default budget must not be kept for the run.
pub async fn probe(client: &Client) -> Result<Option<Budget>, SummarizeError> {
    let model = client.endpoint().model.clone();
    let found = |limit: Option<u32>, source| limit.map(|limit| Budget { limit, source });

    if let Some(body) = answer(client, &client.api_url("model/info")).await?
        && let Some(b) = found(parse_litellm(&body, &model), BudgetSource::Litellm)
    {
        return Ok(Some(b));
    }
    if let Some(body) = answer(client, &client.root_url("api/v0/models")).await?
        && let Some(b) = found(parse_lm_studio(&body, &model), BudgetSource::LmStudio)
    {
        return Ok(Some(b));
    }
    if let Some(body) = answer(client, &client.api_url("models")).await? {
        if let Some(b) = found(parse_vllm(&body, &model), BudgetSource::Vllm) {
            return Ok(Some(b));
        }
        if let Some(b) = found(parse_openrouter(&body, &model), BudgetSource::Openrouter) {
            return Ok(Some(b));
        }
    }
    if let Some(body) = answer(client, &client.root_url("props")).await?
        && let Some(b) = found(parse_llama_cpp(&body), BudgetSource::LlamaCpp)
    {
        return Ok(Some(b));
    }
    Ok(None)
}

/// The JSON body at `url`. `None` when the endpoint answers with no usable
/// body, and an error when it cannot be reached.
async fn answer(client: &Client, url: &str) -> Result<Option<Value>, SummarizeError> {
    match client.probe_json(url).await {
        Ok(body) => Ok(Some(body)),
        Err(e @ SummarizeError::Transport(_)) => Err(e),
        Err(_) => Ok(None),
    }
}
