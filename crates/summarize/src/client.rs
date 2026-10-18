//! Summary requests to the one configured endpoint (R30, R30a). Rig sends
//! each summary request through [`Transport`] (KTD1, KTD2). This module
//! builds the request, sends it, and maps the reply and each error to a
//! [`Completion`] or a [`SummarizeError`] (KTD4).
//!
//! The client follows no redirects: a 3xx is a typed failure naming where
//! it pointed, so nothing the app does by itself sends the transcript or
//! the credential to a host the user did not enter. Every request runs
//! under connect, response, idle and total deadlines with a size cap. The
//! summary queue sends one request at a time, so an endpoint that accepts a
//! connection and never answers would otherwise stop the summary queue.
//! The audio stages of other notes do not wait for the summary queue.
//!
//! Each request writes one log line when it starts and one when it ends
//! (R13). The lines give the [`Label`] of the caller, the host, the model,
//! the duration and the HTTP status or the kind of error. They never give
//! the URL path, the credential, the prompt or the reply. A
//! [`SummarizeError`] made from a Rig result holds only text that Longhand
//! writes, never the text of a Rig or reqwest error.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use reqwest::Url;
use rig_core::ProviderError;
use rig_core::completion::{
    CompletionRequest, CompletionResponse, FinishReason, Message as RigMessage,
};
use rig_core::driver::Model;
use rig_core::http_client::HttpClientExt;
use rig_core::model::ModelList as RigModelList;
use rig_core::providers::anthropic::Anthropic;
use rig_core::providers::anthropic::wire::{AnthropicConfig, Messages};
use rig_core::providers::gemini::completion::GenerateContent;
use rig_core::providers::gemini::{Gemini, GeminiConfig};
use rig_core::providers::ollama::{Chat as OllamaChat, Ollama, OllamaConfig};
use rig_core::providers::openai::wire::{Auth, Chat, Dialect, OPENAI, OPENROUTER};
use rig_core::providers::openai::{OpenAI, OpenAIConfig};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::time::timeout;

use crate::provider::{Provider, Route};
use crate::transport::{Recording, Transport, TransportError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// The provider, which selects the request route (KTD2).
    pub provider: Provider,
    /// The address of the provider. For an OpenAI-compatible provider, it
    /// is the base of the API, usually ending in `/v1`. An empty address
    /// selects [`Provider::default_address`].
    pub base_url: String,
    pub model: String,
    /// The credential stored for this endpoint's provider and host, if
    /// any. A local server may need none.
    pub credential: Option<String>,
}

impl Endpoint {
    /// The address that requests go to: `base_url`, or the default
    /// address of the provider when `base_url` is empty. `None` when both
    /// are empty.
    pub fn address(&self) -> Option<&str> {
        let typed = self.base_url.trim();
        if typed.is_empty() {
            self.provider.default_address()
        } else {
            Some(typed)
        }
    }
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
            // A slow model can take more than 10 minutes to write a summary
            // (R7). `total` adds time to read the reply.
            response: Duration::from_secs(30 * 60),
            idle: Duration::from_secs(60),
            total: Duration::from_secs(32 * 60),
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

/// The kind of a request to the endpoint. The request log names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// A request for the model's context limit.
    Probe,
    /// The summary of one part of a transcript that does not fit in one call.
    Chunk,
    /// A request that combines the summaries of the parts.
    Merge,
    /// The summary of a transcript that fits in one call.
    Single,
    /// The title turn after a summary.
    Title,
    /// The request for the list of models in Settings.
    Models,
    /// The one-token chat request that checks a server with no model list
    /// in Settings (KTD9).
    Check,
}

impl RequestKind {
    fn name(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Chunk => "chunk",
            Self::Merge => "merge",
            Self::Single => "single",
            Self::Title => "title",
            Self::Models => "models",
            Self::Check => "check",
        }
    }
}

/// What the request log says about a request: the note, the kind of
/// request and the time that the request waited in the queue before it
/// was sent. The caller supplies the label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// The note id. `None` for a request that is not for a note.
    pub note: Option<String>,
    pub kind: RequestKind,
    /// The time from when the caller put the job in the queue until the
    /// job started.
    pub queued: Duration,
}

impl Label {
    /// A request for the note `id` that did not wait in a queue.
    pub fn note(id: &str, kind: RequestKind) -> Self {
        Self {
            note: Some(id.to_string()),
            kind,
            queued: Duration::ZERO,
        }
    }

    /// A request that is not for a note.
    pub fn without_note(kind: RequestKind) -> Self {
        Self {
            note: None,
            kind,
            queued: Duration::ZERO,
        }
    }

    /// The same label, with `queued` as the time in the queue.
    pub fn queued(mut self, queued: Duration) -> Self {
        self.queued = queued;
        self
    }
}

/// The log text at the start of a request. It has no URL path, no
/// credential, no prompt and no reply text.
fn start_line(label: &Label, host: &str, model: &str) -> String {
    format!("request start: {}", describe(label, host, model))
}

/// The log text at the end of a request. `result` is the HTTP status or
/// the kind of error, from [`result_text`].
fn end_line(label: &Label, host: &str, model: &str, elapsed: Duration, result: &str) -> String {
    format!(
        "request end: {}, {} ms, {result}",
        describe(label, host, model),
        elapsed.as_millis()
    )
}

/// Logs a `cancelled` end line when it is dropped while it is armed. The
/// caller can drop a request before it ends (R13: each start line has an
/// end line). [`Client::send`] disarms it after it logs the usual end line.
struct EndGuard<'a> {
    label: &'a Label,
    host: &'a str,
    model: &'a str,
    started: Instant,
    armed: bool,
}

impl<'a> EndGuard<'a> {
    fn new(label: &'a Label, host: &'a str, model: &'a str) -> Self {
        Self {
            label,
            host,
            model,
            started: Instant::now(),
            armed: true,
        }
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    /// The end line for a dropped request, or `None` after [`Self::disarm`].
    fn cancelled_text(&self) -> Option<String> {
        self.armed.then(|| {
            end_line(
                self.label,
                self.host,
                self.model,
                self.elapsed(),
                "cancelled",
            )
        })
    }
}

impl Drop for EndGuard<'_> {
    fn drop(&mut self) {
        if let Some(text) = self.cancelled_text() {
            log::info!("{text}");
        }
    }
}

fn describe(label: &Label, host: &str, model: &str) -> String {
    format!(
        "note {}, {}, host {host}, model {model}, {} ms in the queue",
        label.note.as_deref().unwrap_or("none"),
        label.kind.name(),
        label.queued.as_millis()
    )
}

/// The HTTP status of an answer, or the kind of error. The text of an
/// error is not used, because it can contain the endpoint's reply.
fn result_text(result: &Result<(reqwest::StatusCode, Vec<u8>), SummarizeError>) -> String {
    match result {
        Ok((status, _)) => format!("HTTP {}", status.as_u16()),
        Err(e) => error_text(e),
    }
}

/// The result of a summary request for the end line. Rig gives a success
/// reply only for a 2xx status.
fn completion_text(result: &Result<Completion, SummarizeError>) -> String {
    match result {
        Ok(_) => "HTTP 2xx".into(),
        Err(e) => error_text(e),
    }
}

/// The kind of an error, for the end line.
fn error_text(error: &SummarizeError) -> String {
    match error {
        SummarizeError::Redirect { status, .. } => format!("redirect, HTTP {status}"),
        SummarizeError::Deadline(what) => format!("no {what} within the deadline"),
        SummarizeError::TooLarge(_) => "reply too large".into(),
        SummarizeError::Transport(_) => "connection error".into(),
        _ => "error".into(),
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
    /// The prompt is longer than the model accepts: the model's context is
    /// smaller than the budget. The run stops, and the user sets the limit.
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

/// The Chat Completions dialect of Custom and LM Studio (KTD2): OpenAI's
/// request format, the legacy `max_tokens` field for every model id, and
/// no key header when no key is saved. Longhand never serializes it.
static GATEWAY: Dialect = Dialect::gateway("longhand-gateway", "", "");

/// The Rig model that sends the summary requests of one endpoint, one kind
/// for each [`Route`].
enum Engine {
    Chat(Box<Model<Chat>>),
    Anthropic(Box<Model<Messages>>),
    Gemini(Box<Model<GenerateContent>>),
    Ollama(Box<Model<OllamaChat>>),
}

/// The Rig client of one provider at one address (KTD2). It sends each
/// request through the transport that it is made with.
enum RigClient {
    OpenAi(Box<OpenAI>),
    Anthropic(Box<Anthropic>),
    Gemini(Box<Gemini>),
    Ollama(Box<Ollama>),
}

impl RigClient {
    fn new(
        provider: Provider,
        address: &str,
        key: &str,
        transport: impl HttpClientExt + 'static,
    ) -> Self {
        let address = address.trim_end_matches('/');
        match provider {
            Provider::OpenAi | Provider::OpenRouter | Provider::LmStudio | Provider::Custom => {
                let dialect = match provider {
                    Provider::OpenAi => &OPENAI,
                    Provider::OpenRouter => &OPENROUTER,
                    _ => &GATEWAY,
                };
                // Today's rule for each OpenAI-compatible provider: the key
                // goes as a bearer only when there is one.
                let config = OpenAIConfig::with_key(dialect, key)
                    .with_base_url(address)
                    .with_auth(Auth::OptionalBearer);
                Self::OpenAi(Box::new(config.connect(transport)))
            }
            Provider::Anthropic => Self::Anthropic(Box::new(
                AnthropicConfig::new(key)
                    .with_base_url(address)
                    .connect(transport),
            )),
            Provider::Gemini => Self::Gemini(Box::new(
                GeminiConfig::new(key)
                    .with_base_url(address)
                    .connect(transport),
            )),
            Provider::Ollama => {
                // Ollama's native API is at the root, beside `/v1`.
                let root = address.strip_suffix("/v1").unwrap_or(address);
                let mut config = OllamaConfig::new().with_base_url(root);
                if !key.is_empty() {
                    config = config.with_api_key(key);
                }
                Self::Ollama(Box::new(config.connect(transport)))
            }
        }
    }

    /// The models that the provider lists, every page read (KTD5).
    async fn list_models(&self) -> Result<RigModelList, ProviderError> {
        match self {
            Self::OpenAi(client) => client.list_models().await,
            Self::Anthropic(client) => client.list_models().await,
            Self::Gemini(client) => client.list_models().await,
            Self::Ollama(client) => client.list_models().await,
        }
    }
}

impl Engine {
    /// The model for `provider` at `address` (KTD2), sending through
    /// `transport`.
    fn new(
        provider: Provider,
        address: &str,
        model: &str,
        key: &str,
        transport: Transport,
    ) -> Self {
        match RigClient::new(provider, address, key, transport) {
            RigClient::OpenAi(client) => Self::Chat(Box::new(client.chat(model))),
            RigClient::Anthropic(client) => Self::Anthropic(Box::new(client.completion(model))),
            RigClient::Gemini(client) => Self::Gemini(Box::new(client.completion(model))),
            RigClient::Ollama(client) => Self::Ollama(Box::new(client.native_completion(model))),
        }
    }

    async fn call(&self, request: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        match self {
            Self::Chat(model) => model.call(request).await,
            Self::Anthropic(model) => model.call(request).await,
            Self::Gemini(model) => model.call(request).await,
            Self::Ollama(model) => model.call(request).await,
        }
    }
}

/// The result of a model list request through Rig.
pub(crate) struct Listing {
    pub(crate) models: Result<RigModelList, SummarizeError>,
    /// The body of the last success reply, when one came.
    pub(crate) body: Option<bytes::Bytes>,
}

pub struct Client {
    endpoint: Endpoint,
    base: Url,
    route: Route,
    /// The Rig model for summary requests, over a [`Transport`] with the
    /// completion limits.
    engine: Engine,
    /// The probe client, with the probe's shorter connect timeout.
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
        let address = endpoint
            .address()
            .ok_or_else(|| SummarizeError::BadUrl(endpoint.base_url.clone()))?
            .to_string();
        let base =
            Url::parse(&address).map_err(|e| SummarizeError::BadUrl(format!("{address}: {e}")))?;
        if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
            return Err(SummarizeError::BadUrl(address));
        }
        let probe_http =
            http_client(probe.connect).map_err(|e| SummarizeError::Transport(without_url(e)))?;
        let transport = Transport::with_limits(completion).map_err(|e| match e {
            TransportError::Request(text) => SummarizeError::Transport(text),
            _ => SummarizeError::Transport("the HTTP client could not be made".into()),
        })?;
        let key = endpoint
            .credential
            .as_deref()
            .filter(|key| !key.is_empty())
            .unwrap_or_default();
        let engine = Engine::new(endpoint.provider, &address, &endpoint.model, key, transport);
        Ok(Self {
            route: endpoint.provider.route(),
            endpoint,
            base,
            engine,
            probe_http,
            completion,
            probe,
            retry_waits: Vec::new(),
        })
    }

    /// Try a request again after each of `waits` when the connection to the
    /// endpoint fails. A request that can have reached the endpoint is never
    /// sent again.
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

    pub async fn probe_json(&self, label: &Label, url: &str) -> Result<Value, SummarizeError> {
        self.probe_request(label, self.probe_http.get(url)).await
    }

    /// [`Self::probe_json`] with a POST of the JSON `body`, for a probe
    /// such as Ollama's `/api/show`.
    pub async fn probe_post_json(
        &self,
        label: &Label,
        url: &str,
        body: &Value,
    ) -> Result<Value, SummarizeError> {
        self.probe_request(label, self.probe_http.post(url).json(body))
            .await
    }

    async fn probe_request(
        &self,
        label: &Label,
        request: reqwest::RequestBuilder,
    ) -> Result<Value, SummarizeError> {
        let (status, body) = self
            .send(label, self.authorize(request), &self.probe)
            .await?;
        if !status.is_success() {
            return Err(http_error(Route::OpenAiChat, status.as_u16(), &body));
        }
        serde_json::from_slice(&body).map_err(|e| SummarizeError::Malformed(e.to_string()))
    }

    /// The probe limits of this client.
    pub(crate) fn probe_limits(&self) -> Limits {
        self.probe
    }

    /// The address that requests go to. [`Client::with_limits`] refuses an
    /// endpoint with no address.
    pub(crate) fn address(&self) -> &str {
        self.endpoint.address().unwrap_or_default()
    }

    /// True when the client sends a key.
    pub(crate) fn sends_key(&self) -> bool {
        self.endpoint
            .credential
            .as_deref()
            .is_some_and(|key| !key.is_empty())
    }

    /// The models that the provider lists, through Rig over a
    /// [`Transport`] with the probe limits (KTD5). Rig sends no request
    /// again. The listing also gives the body of the last success reply.
    pub(crate) async fn rig_models(&self, label: &Label) -> Listing {
        let transport = match Transport::with_limits(self.probe) {
            Ok(transport) => Recording::new(transport),
            Err(_) => {
                return Listing {
                    models: Err(SummarizeError::Transport(
                        "the HTTP client could not be made".into(),
                    )),
                    body: None,
                };
            }
        };
        let key = self.endpoint.credential.as_deref().unwrap_or_default();
        let rig = RigClient::new(
            self.endpoint.provider,
            self.address(),
            key,
            transport.clone(),
        );
        let (host, model) = (self.host(), &self.endpoint.model);
        log::info!("{}", start_line(label, &host, model));
        let mut guard = EndGuard::new(label, &host, model);
        let models = match timeout(self.probe.total, rig.list_models()).await {
            Err(_) => Err(SummarizeError::Deadline("complete reply")),
            Ok(Ok(list)) => Ok(list),
            Ok(Err(error)) => Err(match failure(self.route, &error) {
                Failure::Answered(e) => e,
                Failure::NotReached(text) => SummarizeError::Transport(text),
            }),
        };
        let result = match &models {
            Ok(_) => "HTTP 2xx".to_string(),
            Err(e) => error_text(e),
        };
        log::info!(
            "{}",
            end_line(label, &host, model, guard.elapsed(), &result)
        );
        guard.disarm();
        Listing {
            models,
            body: transport.body(),
        }
    }

    /// Send one chat request with a reply cap of one token, to check a
    /// server that has no model list (KTD9). The request is sent once
    /// unless the client has retry waits.
    pub(crate) async fn check_chat(&self, label: &Label) -> Result<Completion, SummarizeError> {
        let messages = [Message {
            role: "user",
            content: "Reply with OK.".into(),
        }];
        self.complete_with(label, &messages, "max_tokens", 1, None)
            .await
    }

    /// Send `messages` and return the reply, with at most `max_tokens` in
    /// the reply. `num_ctx` is the context size for an Ollama request
    /// (KTD7). The other providers do not use it.
    pub async fn complete(
        &self,
        label: &Label,
        messages: &[Message],
        max_tokens: u32,
        num_ctx: Option<u32>,
    ) -> Result<Completion, SummarizeError> {
        match self
            .complete_with(label, messages, "max_tokens", max_tokens, num_ctx)
            .await
        {
            // Some hosted reasoning models refuse `max_tokens` by name.
            Err(SummarizeError::Http {
                status: 400,
                excerpt,
            }) if self.route == Route::OpenAiChat && excerpt.contains("max_completion_tokens") => {
                self.complete_with(
                    label,
                    messages,
                    "max_completion_tokens",
                    max_tokens,
                    num_ctx,
                )
                .await
            }
            other => other,
        }
    }

    async fn complete_with(
        &self,
        label: &Label,
        messages: &[Message],
        cap_field: &str,
        max_tokens: u32,
        num_ctx: Option<u32>,
    ) -> Result<Completion, SummarizeError> {
        let request = rig_request(self.route, messages, cap_field, max_tokens, num_ctx);
        let (host, model) = (self.host(), &self.endpoint.model);
        log::info!("{}", start_line(label, &host, model));
        let mut guard = EndGuard::new(label, &host, model);
        let result = self.call_with_retries(request).await;
        log::info!(
            "{}",
            end_line(
                label,
                &host,
                model,
                guard.elapsed(),
                &completion_text(&result)
            )
        );
        guard.disarm();
        result
    }

    /// Send `request` through Rig, and send it again after each retry wait
    /// while it cannot reach the endpoint. Each try has the total deadline
    /// (KTD1).
    async fn call_with_retries(
        &self,
        request: CompletionRequest,
    ) -> Result<Completion, SummarizeError> {
        let mut waits = self.retry_waits.iter();
        loop {
            let reply = timeout(self.completion.total, self.engine.call(request.clone()))
                .await
                .map_err(|_| SummarizeError::Deadline("complete reply"))?;
            let error = match reply {
                Ok(response) => return completion(self.route, &response),
                Err(error) => error,
            };
            match failure(self.route, &error) {
                Failure::Answered(e) => return Err(e),
                Failure::NotReached(text) => {
                    let Some(wait) = waits.next() else {
                        return Err(SummarizeError::Transport(text));
                    };
                    log::warn!(
                        "could not reach {}: {text}; trying again in {} ms",
                        self.host(),
                        wait.as_millis()
                    );
                    tokio::time::sleep(*wait).await;
                }
            }
        }
    }

    /// Send `request`, and log one line at its start and one at its end
    /// (R13).
    async fn send(
        &self,
        label: &Label,
        request: reqwest::RequestBuilder,
        limits: &Limits,
    ) -> Result<(reqwest::StatusCode, Vec<u8>), SummarizeError> {
        let (host, model) = (self.host(), &self.endpoint.model);
        log::info!("{}", start_line(label, &host, model));
        let mut guard = EndGuard::new(label, &host, model);
        let result = self.send_with_retries(request, limits).await;
        log::info!(
            "{}",
            end_line(label, &host, model, guard.elapsed(), &result_text(&result))
        );
        guard.disarm();
        result
    }

    /// Send `request`, and send it again after each retry wait while it
    /// cannot reach the endpoint.
    async fn send_with_retries(
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
                    // Only a failed connection proves that the endpoint did
                    // not receive the request. A connection that closes
                    // later can follow a request that the endpoint runs.
                    // A TLS failure is final: a retry gets the same failure.
                    if e.is_connect() && !is_tls_failure(&e) {
                        Failure::NotReached(without_url(e))
                    } else {
                        Failure::Answered(SummarizeError::Transport(without_url(e)))
                    }
                })?;
            let status = response.status();
            if status.is_redirection() {
                return Err(SummarizeError::Redirect {
                    status: status.as_u16(),
                    location: redirect_location(response.headers()),
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

/// An HTTP client that follows no redirects and stops a connection attempt
/// after `connect`.
pub(crate) fn http_client(connect: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(connect)
        .build()
}

/// The target that a redirect names, or [`NO_LOCATION`].
pub(crate) fn redirect_location(headers: &reqwest::header::HeaderMap) -> String {
    headers
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or(NO_LOCATION)
        .to_string()
}

/// How one try of a request failed.
enum Failure {
    /// The connection to the endpoint failed, so the endpoint did not receive
    /// the request. A new try can succeed.
    NotReached(String),
    /// The endpoint can have received the request. A new try could make the
    /// model do the same work again.
    Answered(SummarizeError),
}

impl From<SummarizeError> for Failure {
    fn from(e: SummarizeError) -> Self {
        Self::Answered(e)
    }
}

/// True when a TLS error (for example a certificate problem) is in the
/// cause chain of `e`.
pub(crate) fn is_tls_failure(e: &reqwest::Error) -> bool {
    let mut cause = std::error::Error::source(e);
    while let Some(c) = cause {
        if c.is::<native_tls::Error>() {
            return true;
        }
        cause = c.source();
    }
    false
}

/// The error with each of its causes, with no URL. reqwest's own text is
/// often only "error sending request", which does not tell why.
pub(crate) fn without_url(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut text = e.to_string();
    let mut cause = std::error::Error::source(&e);
    while let Some(c) = cause {
        let part = c.to_string();
        if !text.ends_with(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        cause = c.source();
    }
    text
}

/// Whether a request to `address` crosses the network unencrypted: plain
/// HTTP to another computer (R21). Plain HTTP to this computer (a loopback
/// address or `localhost`) does not. An address that does not parse does
/// not cross the network.
pub fn crosses_network_unencrypted(address: &str) -> bool {
    Url::parse(address.trim()).is_ok_and(|url| url.scheme() == "http" && !is_loopback(&url))
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

/// Gemini's text for a prompt that is longer than the model accepts.
const GEMINI_CONTEXT_SIGN: &str = "exceeds the maximum number of tokens";

/// Gemini's texts for a key that it does not accept. Gemini answers such a
/// key with 400 or 403, not 401.
const GEMINI_KEY_SIGNS: &[&str] = &["api key not valid", "api_key_invalid"];

/// True when `lower`, a lowercase error body, says that the prompt is too
/// long for the model on `route`.
fn names_context(route: Route, lower: &str) -> bool {
    CONTEXT_SIGNS.iter().any(|s| lower.contains(s))
        || (route == Route::Gemini && lower.contains(GEMINI_CONTEXT_SIGN))
}

/// The error for a reply with the failure status `status` and the body
/// `body` on `route` (KTD4).
fn http_error(route: Route, status: u16, body: &[u8]) -> SummarizeError {
    let text = String::from_utf8_lossy(body);
    let lower = text.to_lowercase();
    if matches!(status, 400 | 413 | 422) && names_context(route, &lower) {
        return SummarizeError::ContextExceeded;
    }
    let refused_key = match route {
        Route::Gemini => {
            matches!(status, 400 | 401 | 403) && GEMINI_KEY_SIGNS.iter().any(|s| lower.contains(s))
        }
        _ => matches!(status, 401 | 403),
    };
    if refused_key {
        return SummarizeError::Unauthorized(status);
    }
    SummarizeError::Http {
        status,
        excerpt: excerpt(&text),
    }
}

/// The start of a provider's error body, as the user sees it.
fn excerpt(text: &str) -> String {
    let excerpt: String = text.chars().take(200).collect();
    excerpt.trim().to_string()
}

/// The text of a reply that Rig could not read as a completion. The Rig
/// error is not used, because its text can contain the reply.
const UNREADABLE: &str = "the reply could not be read";

/// The Rig request for `messages` on `route` (KTD3). On the
/// OpenAI-compatible route, the additional parameters hold today's
/// `messages` (string content), `"stream": false` and the reply cap
/// under `cap_field`, so the body is the same as before Rig. Ollama gets
/// `num_ctx` when there is one, and always `truncate: false` (KTD7).
fn rig_request(
    route: Route,
    messages: &[Message],
    cap_field: &str,
    max_tokens: u32,
    num_ctx: Option<u32>,
) -> CompletionRequest {
    let mut history: Vec<RigMessage> = messages
        .iter()
        .map(|m| match m.role {
            "system" => RigMessage::system(m.content.clone()),
            "assistant" => RigMessage::assistant(m.content.clone()),
            _ => RigMessage::user(m.content.clone()),
        })
        .collect();
    let prompt = history.pop().unwrap_or_else(|| RigMessage::user(""));
    let request = CompletionRequest::new(prompt).messages(history);
    match route {
        Route::OpenAiChat => {
            let mut params = Map::new();
            params.insert("messages".into(), json!(messages));
            params.insert("stream".into(), Value::Bool(false));
            params.insert(cap_field.into(), json!(max_tokens));
            request.additional_params(Value::Object(params))
        }
        Route::Anthropic | Route::Gemini => request.max_tokens(u64::from(max_tokens)),
        Route::Ollama => {
            let mut params = Map::new();
            params.insert("truncate".into(), Value::Bool(false));
            if let Some(num_ctx) = num_ctx {
                params.insert("num_ctx".into(), json!(num_ctx));
            }
            request
                .max_tokens(u64::from(max_tokens))
                .additional_params(Value::Object(params))
        }
    }
}

/// The completion in a Rig reply on `route`, or the error that the reply
/// signals (KTD4).
fn completion(route: Route, response: &CompletionResponse) -> Result<Completion, SummarizeError> {
    let raw = &response.raw;
    match route {
        // llama.cpp says so when it cut the prompt to fit.
        Route::OpenAiChat if raw.get("truncated").and_then(Value::as_bool) == Some(true) => {
            return Err(SummarizeError::ContextExceeded);
        }
        // Rig reads this stop reason as `Length`, but the prompt filled the
        // context window: a bigger reply cap does not help.
        Route::Anthropic
            if raw.get("stop_reason").and_then(Value::as_str)
                == Some("model_context_window_exceeded") =>
        {
            return Err(SummarizeError::ContextExceeded);
        }
        _ => {}
    }
    let finish_reason = response.finish_reason();
    if finish_reason == Some(FinishReason::Length) {
        return Err(SummarizeError::ReplyCutOff);
    }
    let text = response.text().trim().to_string();
    if text.is_empty() {
        return Err(SummarizeError::Malformed("an empty reply".into()));
    }
    Ok(Completion {
        text,
        finish_reason: finish_reason.map(finish_name),
    })
}

/// The wire name of a finish reason, as OpenAI spells it.
fn finish_name(reason: FinishReason) -> String {
    match reason {
        FinishReason::Stop => "stop".into(),
        FinishReason::Length => "length".into(),
        FinishReason::ToolCalls => "tool_calls".into(),
        FinishReason::ContentFilter => "content_filter".into(),
        FinishReason::Other(name) => name,
        _ => "other".into(),
    }
}

/// How a Rig request on `route` failed, as a Longhand error (KTD4). Only a
/// transport error that proves the endpoint did not receive the request
/// can be sent again.
fn failure(route: Route, error: &ProviderError) -> Failure {
    let answered = |e| Failure::Answered(e);
    match error {
        ProviderError::Http(inner) => match TransportError::find(inner) {
            Some(e) => transport_failure(e),
            None => answered(SummarizeError::Transport(
                "the connection to the endpoint failed".into(),
            )),
        },
        ProviderError::ProviderResponse(reply)
        | ProviderError::InvalidAuthentication(reply)
        | ProviderError::CacheExpired {
            response: reply, ..
        } => answered(match reply.status {
            Some(status) => http_error(route, status.as_u16(), reply.body.as_bytes()),
            // An error reply with no status, for example an error in a 2xx
            // body.
            None if names_context(route, &reply.body.to_lowercase()) => {
                SummarizeError::ContextExceeded
            }
            None => SummarizeError::Malformed(format!("an error reply: {}", excerpt(&reply.body))),
        }),
        ProviderError::Request(_) | ProviderError::Url(_) | ProviderError::UnsupportedOption(_) => {
            answered(SummarizeError::Transport(
                "the request could not be made".into(),
            ))
        }
        _ => answered(SummarizeError::Malformed(UNREADABLE.into())),
    }
}

/// The Longhand error for a transport error. Each text is written by
/// Longhand, or is a cause that [`Transport`] got with no URL.
fn transport_failure(error: &TransportError) -> Failure {
    let e = match error {
        TransportError::Unreachable(text) => return Failure::NotReached(text.clone()),
        TransportError::Redirect { status, location } => SummarizeError::Redirect {
            status: *status,
            location: location.clone(),
        },
        TransportError::Tls(text)
        | TransportError::Connection(text)
        | TransportError::Request(text) => SummarizeError::Transport(text.clone()),
        TransportError::ResponseDeadline => SummarizeError::Deadline("response"),
        TransportError::IdleDeadline => SummarizeError::Deadline("data"),
        TransportError::TooLarge(limit) => SummarizeError::TooLarge(*limit),
        TransportError::Unsupported(_) => {
            SummarizeError::Transport("the transport cannot send this request".into())
        }
    };
    Failure::Answered(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_http_to_another_computer_crosses_the_network_unencrypted() {
        assert!(crosses_network_unencrypted("http://192.168.1.20:11434"));
        assert!(crosses_network_unencrypted("http://llm.example.net/v1"));
        assert!(!crosses_network_unencrypted("http://localhost:11434"));
        assert!(!crosses_network_unencrypted("http://127.0.0.1:1234/v1"));
        assert!(!crosses_network_unencrypted("http://[::1]:1234/v1"));
        assert!(!crosses_network_unencrypted("https://api.openai.com/v1"));
        assert!(!crosses_network_unencrypted("not a url"));
    }

    #[test]
    fn the_request_log_names_the_request_and_not_its_content() {
        let label =
            Label::note("2026-10-14-0900", RequestKind::Chunk).queued(Duration::from_millis(4_200));
        let start = start_line(&label, "llm.example.net", "qwen3-30b");
        assert_eq!(
            start,
            "request start: note 2026-10-14-0900, chunk, host llm.example.net, \
             model qwen3-30b, 4200 ms in the queue"
        );
        let reply = Ok((reqwest::StatusCode::OK, b"secret summary text".to_vec()));
        let end = end_line(
            &label,
            "llm.example.net",
            "qwen3-30b",
            Duration::from_millis(91_000),
            &result_text(&reply),
        );
        assert_eq!(
            end,
            "request end: note 2026-10-14-0900, chunk, host llm.example.net, \
             model qwen3-30b, 4200 ms in the queue, 91000 ms, HTTP 200"
        );
        assert!(!end.contains("secret"));
    }

    #[test]
    fn the_end_line_gives_the_error_kind_and_not_the_error_text() {
        // An error text can hold the endpoint's reply or a URL.
        let cases = [
            (
                SummarizeError::Deadline("response"),
                "no response within the deadline",
            ),
            (
                SummarizeError::Transport("http://h/v1?key=sk-secret refused".into()),
                "connection error",
            ),
            (
                SummarizeError::Redirect {
                    status: 307,
                    location: "https://other/?key=sk-secret".into(),
                },
                "redirect, HTTP 307",
            ),
            (SummarizeError::TooLarge(8), "reply too large"),
        ];
        for (error, expected) in cases {
            assert_eq!(result_text(&Err(error)), expected);
        }
        let label = Label::without_note(RequestKind::Models);
        let line = end_line(&label, "h", "m", Duration::ZERO, "connection error");
        assert!(line.starts_with("request end: note none, models, host h,"));
    }

    #[test]
    fn a_dropped_request_gives_a_cancelled_end_line_unless_it_ended() {
        // `block_on` drops the send future when a note is resumed or the
        // app quits. The log must then show that the request stopped.
        let label = Label::note("2026-10-14-0900", RequestKind::Chunk);
        let armed = EndGuard::new(&label, "h", "m");
        let text = armed.cancelled_text().expect("an armed guard gives a line");
        assert!(text.starts_with("request end: note 2026-10-14-0900, chunk, host h, model m,"));
        assert!(text.ends_with(" ms, cancelled"));

        let mut ended = EndGuard::new(&label, "h", "m");
        ended.disarm();
        assert_eq!(ended.cancelled_text(), None);
    }
}
