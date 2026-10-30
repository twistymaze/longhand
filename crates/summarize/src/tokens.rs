//! Client-side token estimates (KTD16). No tokenizer is portable across
//! endpoints, so the estimate is bytes over a conservative ratio plus the
//! per-message framing chat templates add, and every budget check applies a
//! 15% margin. A server that tokenizes denser still rejects the prompt, and
//! the run stops (see `pipeline::summarize`).

use crate::context::BudgetSource;
use crate::provider::Provider;

/// English prose runs near four bytes a token; three and a half leaves room
/// for names, numbers and punctuation-heavy speech.
pub const BYTES_PER_TOKEN: f64 = 3.5;
/// What a chat template spends on each message's role and delimiters.
pub const MESSAGE_FRAMING: usize = 8;
pub const MARGIN: f64 = 0.15;

pub fn estimate(text: &str) -> usize {
    estimate_bytes(text.len())
}

/// The estimate for a text of `bytes` bytes.
pub fn estimate_bytes(bytes: usize) -> usize {
    (bytes as f64 / BYTES_PER_TOKEN).ceil() as usize
}

pub fn estimate_messages<'a>(texts: impl IntoIterator<Item = &'a str>) -> usize {
    texts
        .into_iter()
        .map(|t| estimate(t) + MESSAGE_FRAMING)
        .sum()
}

/// The estimate with the margin applied: what a budget must hold.
pub fn with_margin(tokens: usize) -> usize {
    (tokens as f64 * (1.0 + MARGIN)).ceil() as usize
}

/// How much of `limit` the prompt may use once the reply is reserved.
pub fn prompt_allowance(limit: u32, split_reserve: u32) -> usize {
    let usable = (limit.saturating_sub(split_reserve)) as f64 / (1.0 + MARGIN);
    usable.floor() as usize
}

/// The split reserve: the room for the reply that the split decision and
/// the part sizes keep (KTD1). It is a quarter of the context, within
/// bounds that keep a summary useful and a small window usable. The reply
/// allowance of each request is a different number ([`reply_allowance`]).
pub fn split_reserve(limit: u32) -> u32 {
    (limit / 4).clamp(512, 8_192)
}

/// The largest reply allowance when Longhand does not know the output
/// maximum of a hosted model, and the first allowance of a hosted model
/// (R2, KTD2).
pub const HOSTED_REPLY: u32 = 16_384;
/// The first reply allowance of an Ollama request. Ollama reserves memory
/// for the prompt and the allowance (`num_ctx`), so a larger allowance
/// costs memory and can load the model again (KTD2).
pub const OLLAMA_FIRST_REPLY: u32 = 32_768;
/// The largest reply allowance of an Anthropic request. Anthropic refuses
/// a request without streaming above this number (KTD2).
pub const ANTHROPIC_REPLY: u32 = 21_333;

/// How the provider kind limits the reply allowance (KTD2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    /// LM Studio, and a Custom server whose context limit came from a
    /// llama.cpp or vLLM probe: the first request gets all the room.
    Full,
    /// Ollama: the first request gets at most [`OLLAMA_FIRST_REPLY`].
    Ollama,
    /// The other providers: the first request gets at most
    /// [`HOSTED_REPLY`], and an unknown output maximum is [`HOSTED_REPLY`].
    Hosted,
    /// Anthropic: as [`ReplyKind::Hosted`], and never more than
    /// [`ANTHROPIC_REPLY`].
    Anthropic,
}

impl ReplyKind {
    /// The kind of the requests to `provider` under a budget from `source`.
    pub fn of(provider: Provider, source: BudgetSource) -> Self {
        match (provider, source) {
            (Provider::LmStudio, _) => Self::Full,
            (Provider::Custom, BudgetSource::LlamaCpp | BudgetSource::Vllm) => Self::Full,
            (Provider::Ollama, _) => Self::Ollama,
            (Provider::Anthropic, _) => Self::Anthropic,
            _ => Self::Hosted,
        }
    }
}

/// The reply allowance of one request (R1, KTD2): the first allowance, and
/// the largest allowance that the request can get.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allowance {
    pub first: u32,
    pub ceiling: u32,
}

/// The reply allowance of a request in a context of `limit` tokens, whose
/// prompt estimate with the margin is `used` tokens. `output_max` is the
/// model's output maximum when Longhand knows it.
pub fn reply_allowance(
    limit: u32,
    used: usize,
    output_max: Option<u32>,
    kind: ReplyKind,
) -> Allowance {
    let left = u32::try_from((limit as usize).saturating_sub(used)).unwrap_or(u32::MAX);
    let ninety = (u64::from(limit) * 9 / 10) as u32;
    let mut ceiling = left.min(ninety);
    if let Some(max) = output_max {
        ceiling = ceiling.min(max);
    }
    let first = match kind {
        ReplyKind::Full => ceiling,
        ReplyKind::Ollama => ceiling.min(OLLAMA_FIRST_REPLY),
        ReplyKind::Hosted | ReplyKind::Anthropic => {
            if output_max.is_none() {
                ceiling = ceiling.min(HOSTED_REPLY);
            }
            if kind == ReplyKind::Anthropic {
                ceiling = ceiling.min(ANTHROPIC_REPLY);
            }
            ceiling.min(HOSTED_REPLY)
        }
    };
    Allowance { first, ceiling }
}
