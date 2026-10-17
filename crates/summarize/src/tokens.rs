//! Client-side token estimates (KTD16). No tokenizer is portable across
//! endpoints, so the estimate is bytes over a conservative ratio plus the
//! per-message framing chat templates add, and every budget check applies a
//! 15% margin. A server that tokenizes denser still rejects the prompt, and
//! the run stops (see `pipeline::summarize`).

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
pub fn prompt_allowance(limit: u32, reply_reserve: u32) -> usize {
    let usable = (limit.saturating_sub(reply_reserve)) as f64 / (1.0 + MARGIN);
    usable.floor() as usize
}

/// Room kept for the model's reply: a quarter of the context, within bounds
/// that keep a summary useful and a small window usable.
pub fn reply_reserve(limit: u32) -> u32 {
    (limit / 4).clamp(512, 8_192)
}
