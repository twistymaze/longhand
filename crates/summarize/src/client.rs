//! Chat completions against the one configured endpoint (R30, R30a).
//!
//! The client follows no redirects: a 3xx is a typed failure naming where
//! it pointed, so nothing the app does by itself sends the transcript or
//! the credential to a host the user did not enter. Every request runs
//! under connect, response, idle and total deadlines with a size cap,
//! because the queue runs one stage at a time and an endpoint that accepts
//! a connection and never answers would otherwise hold every later session.

use std::net::IpAddr;
use std::time::Duration;

use reqwest::Url;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::time::timeout;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// OpenAI-style base, usually ending in `/v1`.
    pub base_url: String,
    pub model: String,
    /// The bearer credential stored for this endpoint's host, if any. A
    /// local server may need none.
    pub credential: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub connect: Duration,
    /// Until the response headers. A non-streamed completion sends them
    /// only once the reply is written, so this is the generation deadline.
    pub response: Duration,
    /// Between body chunks.
    pub idle: Duration,
    pub total: Duration,
    pub max_bytes: usize,
}

impl Limits {
    pub fn completion() -> Self {
        Self {
            connect: Duration::from_secs(10),
            response: Duration::from_secs(600),
            idle: Duration::from_secs(60),
            total: Duration::from_secs(900),
            max_bytes: 8 << 20,
        }
    }

    pub fn probe() -> Self {
        Self {
            connect: Duration::from_secs(5),
            response: Duration::from_secs(10),
            idle: Duration::from_secs(10),
            total: Duration::from_secs(15),
            max_bytes: 4 << 20,
        }
    }
}

/// The `location` of a redirect that has no `Location` header.
pub(crate) const NO_LOCATION: &str = "an unnamed location";

#[derive(Debug, thiserror::Error)]
pub enum SummarizeError {
    #[error("the summarization endpoint is not a valid http(s) URL: {0}")]
    BadUrl(String),
    #[error(
        "the endpoint answered HTTP {status} redirecting to {location}; Longhand follows no redirects"
    )]
    Redirect { status: u16, location: String },
    #[error(
        "the endpoint refused the request as unauthorized (HTTP {0}); check the credential stored for this host"
    )]
    Unauthorized(u16),
    #[error("the prompt is longer than the model accepts")]
    ContextExceeded,
    /// The reply ran out of room (`finish_reason: length`): the prompt fit,
    /// the answer did not. A reasoning model can spend the reply budget
    /// thinking before it writes anything.
    #[error("the reply was cut off at its length limit")]
    ReplyCutOff,
    #[error("the endpoint answered HTTP {status}: {excerpt}")]
    Http { status: u16, excerpt: String },
    #[error("no {0} from the endpoint within its deadline")]
    Deadline(&'static str),
    #[error("the endpoint's reply exceeded {0} bytes")]
    TooLarge(usize),
    #[error("could not reach the endpoint: {0}")]
    Transport(String),
    #[error("the endpoint's reply is not a chat completion: {0}")]
    Malformed(String),
}

impl SummarizeError {
    /// The model's context is smaller than the budget assumed: shrink the
    /// budget and chunk again rather than fail.
    pub fn is_context_pressure(&self) -> bool {
        matches!(self, Self::ContextExceeded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub finish_reason: Option<String>,
}

/// The waits before each new try of a request that could not reach the
/// endpoint: about 7 minutes in total. A Mac can wake from sleep for some
/// seconds with no network. The time of a wait does not advance while the
/// Mac sleeps, so a request that started in such a wake tries again after
/// the next full wake.
pub const RETRY_WAITS: [Duration; 10] = [
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(20),
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(60),
    Duration::from_secs(60),
    Duration::from_secs(60),
    Duration::from_secs(60),
    Duration::from_secs(60),
];

pub struct Client {
    endpoint: Endpoint,
    base: Url,
    http: reqwest::Client,
    /// The same, with the probe's shorter connect timeout.
    probe_http: reqwest::Client,
    completion: Limits,
    probe: Limits,
    /// The waits before each new try of a request that could not reach the
    /// endpoint. Empty: a request is sent once.
    retry_waits: Vec<Duration>,
}

impl Client {
    pub fn new(endpoint: Endpoint) -> Result<Self, SummarizeError> {
        Self::with_limits(endpoint, Limits::completion(), Limits::probe())
    }

    pub fn with_limits(
        endpoint: Endpoint,
        completion: Limits,
        probe: Limits,
    ) -> Result<Self, SummarizeError> {
        let base = Url::parse(endpoint.base_url.trim())
            .map_err(|e| SummarizeError::BadUrl(format!("{}: {e}", endpoint.base_url)))?;
        if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
            return Err(SummarizeError::BadUrl(endpoint.base_url.clone()));
        }
        let client = |connect| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(connect)
                .build()
                .map_err(|e| SummarizeError::Transport(e.to_string()))
        };
        let http = client(completion.connect)?;
        let probe_http = client(probe.connect)?;
        Ok(Self {
            endpoint,
            base,
            http,
            probe_http,
            completion,
            probe,
            retry_waits: Vec::new(),
        })
    }

    /// Try a request again after each of `waits` when it cannot reach the
    /// endpoint. A request that reaches the endpoint is never sent again.
    pub fn with_retry_waits(mut self, waits: Vec<Duration>) -> Self {
        self.retry_waits = waits;
        self
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn host(&self) -> String {
        self.base.host_str().unwrap_or_default().to_string()
    }

    /// Plain HTTP to another machine: allowed, and the surface says so (R30a).
    pub fn crosses_network_unencrypted(&self) -> bool {
        self.base.scheme() == "http" && !is_loopback(&self.base)
    }

    /// `path` under the base, e.g. `chat/completions` under `…/v1`.
    pub fn api_url(&self, path: &str) -> String {
        format!("{}/{path}", self.base.as_str().trim_end_matches('/'))
    }

    /// `path` at the server root, for probes that live beside `/v1`.
    pub fn root_url(&self, path: &str) -> String {
        let base = self.base.as_str().trim_end_matches('/');
        let root = base.strip_suffix("/v1").unwrap_or(base);
        format!("{root}/{path}")
    }

    fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.endpoint.credential {
            Some(token) if !token.is_empty() => request.bearer_auth(token),
            _ => request,
        }
    }

    pub async fn probe_json(&self, url: &str) -> Result<Value, SummarizeError> {
        let request = self.authorize(self.probe_http.get(url));
        let (status, body) = self.send(request, &self.probe).await?;
        if !status.is_success() {
            return Err(http_error(status.as_u16(), &body));
        }
        serde_json::from_slice(&body).map_err(|e| SummarizeError::Malformed(e.to_string()))
    }

    pub async fn complete(
        &self,
        messages: &[Message],
        max_tokens: u32,
    ) -> Result<Completion, SummarizeError> {
        match self.complete_with(messages, "max_tokens", max_tokens).await {
            // Some hosted reasoning models refuse `max_tokens` by name.
            Err(SummarizeError::Http {
                status: 400,
                excerpt,
            }) if excerpt.contains("max_completion_tokens") => {
                self.complete_with(messages, "max_completion_tokens", max_tokens)
                    .await
            }
            other => other,
        }
    }

    async fn complete_with(
        &self,
        messages: &[Message],
        cap_field: &str,
        max_tokens: u32,
    ) -> Result<Completion, SummarizeError> {
        let mut body = json!({
            "model": self.endpoint.model,
            "messages": messages,
            "stream": false,
        });
        body[cap_field] = json!(max_tokens);
        let request = self
            .authorize(self.http.post(self.api_url("chat/completions")))
            .json(&body);
        let (status, bytes) = self.send(request, &self.completion).await?;
        if !status.is_success() {
            return Err(http_error(status.as_u16(), &bytes));
        }
        let reply: Value =
            serde_json::from_slice(&bytes).map_err(|e| SummarizeError::Malformed(e.to_string()))?;
        parse_completion(&reply)
    }

    /// Send `request`, and send it again after each retry wait while it
    /// cannot reach the endpoint.
    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        limits: &Limits,
    ) -> Result<(reqwest::StatusCode, Vec<u8>), SummarizeError> {
        let mut waits = self.retry_waits.iter();
        loop {
            let attempt = request.try_clone().ok_or_else(|| {
                SummarizeError::Transport("the request cannot be sent again".into())
            })?;
            match self.send_once(attempt, limits).await {
                Ok(answer) => return Ok(answer),
                Err(Failure::Answered(e)) => return Err(e),
                Err(Failure::NotReached(e)) => {
                    let Some(wait) = waits.next() else {
                        return Err(SummarizeError::Transport(e));
                    };
                    log::warn!(
                        "could not reach {}: {e}; trying again in {} ms",
                        self.host(),
                        wait.as_millis()
                    );
                    tokio::time::sleep(*wait).await;
                }
            }
        }
    }

    async fn send_once(
        &self,
        request: reqwest::RequestBuilder,
        limits: &Limits,
    ) -> Result<(reqwest::StatusCode, Vec<u8>), Failure> {
        let work = async {
            let mut response = timeout(limits.response, request.send())
                .await
                .map_err(|_| SummarizeError::Deadline("response"))?
                .map_err(|e| {
                    // A request that could not be built fails the same way
                    // on each try.
                    if e.is_builder() {
                        Failure::Answered(SummarizeError::Transport(without_url(e)))
                    } else {
                        Failure::NotReached(without_url(e))
                    }
                })?;
            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(NO_LOCATION)
                    .to_string();
                return Err(SummarizeError::Redirect {
                    status: status.as_u16(),
                    location,
                }
                .into());
            }
            let mut body = Vec::new();
            loop {
                match timeout(limits.idle, response.chunk()).await {
                    Err(_) => return Err(SummarizeError::Deadline("data").into()),
                    Ok(Err(e)) => return Err(SummarizeError::Transport(without_url(e)).into()),
                    Ok(Ok(None)) => break,
                    Ok(Ok(Some(bytes))) => {
                        if body.len() + bytes.len() > limits.max_bytes {
                            return Err(SummarizeError::TooLarge(limits.max_bytes).into());
                        }
                        body.extend_from_slice(&bytes);
                    }
                }
            }
            Ok((status, body))
        };
        timeout(limits.total, work)
            .await
            .map_err(|_| SummarizeError::Deadline("complete reply"))?
    }
}

/// How one try of a request failed.
enum Failure {
    /// The request did not reach the endpoint: no answer started. A new try
    /// can succeed.
    NotReached(String),
    /// The endpoint answered, or the request cannot be sent. A new try would
    /// send the prompt again for the same result.
    Answered(SummarizeError),
}

impl From<SummarizeError> for Failure {
    fn from(e: SummarizeError) -> Self {
        Self::Answered(e)
    }
}

fn without_url(e: reqwest::Error) -> String {
    e.without_url().to_string()
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    }
}

const CONTEXT_SIGNS: &[&str] = &[
    "context length",
    "context_length",
    "maximum context",
    "context window",
    "too many tokens",
    "prompt is too long",
    "exceeds the context",
    "input is too long",
    "n_ctx",
    "reduce the length",
];

fn http_error(status: u16, body: &[u8]) -> SummarizeError {
    let text = String::from_utf8_lossy(body);
    let lower = text.to_lowercase();
    if matches!(status, 400 | 413 | 422) && CONTEXT_SIGNS.iter().any(|s| lower.contains(s)) {
        return SummarizeError::ContextExceeded;
    }
    if matches!(status, 401 | 403) {
        return SummarizeError::Unauthorized(status);
    }
    let excerpt: String = text.chars().take(200).collect();
    SummarizeError::Http {
        status,
        excerpt: excerpt.trim().to_string(),
    }
}

pub fn parse_completion(reply: &Value) -> Result<Completion, SummarizeError> {
    let choice = reply
        .pointer("/choices/0")
        .ok_or_else(|| SummarizeError::Malformed("no choices".into()))?;
    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    // llama.cpp says so when it cut the prompt to fit.
    if reply.get("truncated").and_then(Value::as_bool) == Some(true) {
        return Err(SummarizeError::ContextExceeded);
    }
    if finish_reason.as_deref() == Some("length") {
        return Err(SummarizeError::ReplyCutOff);
    }
    let text = choice
        .pointer("/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| SummarizeError::Malformed("no message content".into()))?
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(SummarizeError::Malformed("an empty reply".into()));
    }
    Ok(Completion {
        text,
        finish_reason,
    })
}
