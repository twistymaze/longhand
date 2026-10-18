//! The context budget (KTD16). No field of `/v1/models` is portable, so the
//! limit is probed across the servers people actually run, in order, and
//! each answer is read defensively: a gateway may list the model with no
//! context metadata at all (an alias row, a locally served model), and a
//! partial answer is a miss, not an error. The user's setting wins; with
//! neither, [`DEFAULT_LIMIT`], which most current models accept. A model
//! that accepts less rejects the prompt, and the user sets the limit.
//!
//! The probe for each provider (KTD5): the chain above for an
//! OpenAI-compatible server (OpenRouter, LM Studio, Custom), the input
//! token limit in Gemini's model list, and the context length in Ollama's
//! `/api/show`. OpenAI and Anthropic report no limit, so no request is
//! sent for them.

use rig_core::model::ModelInfo;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::{Client, Label, SummarizeError};
use crate::provider::Provider;

pub const DEFAULT_LIMIT: u32 = 128_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetSource {
    User,
    /// A limit that an older version learned from a rejection. Only a
    /// saved plan can have it.
    Learned,
    Litellm,
    LmStudio,
    Vllm,
    LlamaCpp,
    Openrouter,
    Gemini,
    Ollama,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub limit: u32,
    pub source: BudgetSource,
}

/// The setting, else the probe, else the default.
pub fn resolve(user: Option<u32>, probed: Option<Budget>) -> Budget {
    match (user, probed) {
        (Some(limit), _) => Budget {
            limit,
            source: BudgetSource::User,
        },
        (None, Some(probed)) => probed,
        (None, None) => Budget {
            limit: DEFAULT_LIMIT,
            source: BudgetSource::Default,
        },
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
/// when the first probe cannot reach the endpoint, gets no answer within
/// its deadline, or gets a server error ([`is_server_problem`]): then
/// nothing is known about the context, and the default budget must not be
/// kept for the run. A later probe asks a route that the server can lack,
/// so any failure there is a miss.
pub async fn probe(client: &Client, label: &Label) -> Result<Option<Budget>, SummarizeError> {
    match client.endpoint().provider {
        Provider::OpenAi | Provider::Anthropic => Ok(None),
        Provider::Gemini => probe_gemini(client, label).await,
        Provider::Ollama => probe_ollama(client, label).await,
        Provider::OpenRouter | Provider::LmStudio | Provider::Custom => {
            probe_compatible(client, label).await
        }
    }
}

/// Ollama `/api/show`: `model_info.{architecture}.context_length`, the
/// longest context that the model accepts.
pub fn parse_ollama_show(body: &Value) -> Option<u32> {
    let info = body.get("model_info")?.as_object()?;
    if let Some(arch) = info.get("general.architecture").and_then(Value::as_str)
        && let Some(limit) = positive(info.get(&format!("{arch}.context_length")))
    {
        return Some(limit);
    }
    info.iter()
        .filter(|(key, _)| key.ends_with(".context_length"))
        .find_map(|(_, value)| positive(Some(value)))
}

/// The input token limit of `model` in Gemini's model list. The id in the
/// list has no `models/` prefix.
fn parse_gemini(models: &[ModelInfo], model: &str) -> Option<u32> {
    let model = model.strip_prefix("models/").unwrap_or(model);
    models
        .iter()
        .filter(|info| info.id == model)
        .find_map(|info| info.context_length.filter(|&n| n > 0))
}

/// The Gemini limit, from the model list that Rig reads (KTD5).
async fn probe_gemini(client: &Client, label: &Label) -> Result<Option<Budget>, SummarizeError> {
    let models = answer(client.rig_models(label).await.models, true)?;
    Ok(models
        .and_then(|list| parse_gemini(&list.data, &client.endpoint().model))
        .map(|limit| Budget {
            limit,
            source: BudgetSource::Gemini,
        }))
}

/// The Ollama limit, from `/api/show` at the server root.
async fn probe_ollama(client: &Client, label: &Label) -> Result<Option<Budget>, SummarizeError> {
    let request = json!({ "model": client.endpoint().model });
    let body = answer(
        client
            .probe_post_json(label, &client.root_url("api/show"), &request)
            .await,
        true,
    )?;
    Ok(body
        .as_ref()
        .and_then(parse_ollama_show)
        .map(|limit| Budget {
            limit,
            source: BudgetSource::Ollama,
        }))
}

/// The probe chain for an OpenAI-compatible server.
async fn probe_compatible(
    client: &Client,
    label: &Label,
) -> Result<Option<Budget>, SummarizeError> {
    let model = client.endpoint().model.clone();
    let found = |limit: Option<u32>, source| limit.map(|limit| Budget { limit, source });

    let first = answer(
        client
            .probe_json(label, &client.api_url("model/info"))
            .await,
        true,
    )?;
    if let Some(body) = first
        && let Some(b) = found(parse_litellm(&body, &model), BudgetSource::Litellm)
    {
        return Ok(Some(b));
    }
    let lm_studio = client
        .probe_json(label, &client.root_url("api/v0/models"))
        .await;
    if let Some(body) = answer(lm_studio, false)?
        && let Some(b) = found(parse_lm_studio(&body, &model), BudgetSource::LmStudio)
    {
        return Ok(Some(b));
    }
    if let Some(body) = answer(
        client.probe_json(label, &client.api_url("models")).await,
        false,
    )? {
        if let Some(b) = found(parse_vllm(&body, &model), BudgetSource::Vllm) {
            return Ok(Some(b));
        }
        if let Some(b) = found(parse_openrouter(&body, &model), BudgetSource::Openrouter) {
            return Ok(Some(b));
        }
    }
    if let Some(body) = answer(
        client.probe_json(label, &client.root_url("props")).await,
        false,
    )? && let Some(b) = found(parse_llama_cpp(&body), BudgetSource::LlamaCpp)
    {
        return Ok(Some(b));
    }
    Ok(None)
}

/// The body of a probe's answer, `None` for a miss, or the error that stops
/// the probe. `first` is true for the first probe of the chain.
fn answer<T>(result: Result<T, SummarizeError>, first: bool) -> Result<Option<T>, SummarizeError> {
    match result {
        Ok(body) => Ok(Some(body)),
        Err(e @ (SummarizeError::Transport(_) | SummarizeError::Deadline(_))) if first => Err(e),
        Err(e) if first && is_server_problem(&e) => Err(e),
        // 401 or 403 stays a miss: a key can be limited to the completion
        // routes. A bad key fails at the first completion with a clear error.
        Err(_) => Ok(None),
    }
}

/// A server error (5xx), a timeout (408) or a rate limit (429). The server
/// has a problem now, so its answer tells nothing about the context. A
/// route that the server does not have (404, 501) is a miss.
fn is_server_problem(e: &SummarizeError) -> bool {
    matches!(e, SummarizeError::Http { status, .. }
        if (*status >= 500 && *status != 501) || matches!(status, 408 | 429))
}
