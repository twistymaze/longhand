//! The layer around each model turn of the summary pipeline (KTD3, KTD4).
//!
//! Before each turn, the layer calls `control.proceed()`, so a pause holds
//! the run and a cancel stops it (R7, R8). It returns the saved reply of a
//! turn when the turn key has one. Otherwise it sends the turn and saves
//! the reply, so that a run that stops continues from the last completed
//! turn (R6). A saved reply that does not decode is a cache miss: the layer
//! deletes the row and sends the turn again.
//!
//! The layer keeps the typed error of the last turn when that turn failed
//! (KTD10), and it sets a flag when a cancel stopped a turn. `produce`
//! reads the flag, because the agent gets a cancel as an ordinary error.
//!
//! The turn key is [`TURN_PREFIX`] and a SHA-256 hash of a canonical form
//! of the turn: the role and the text of each message, the tool calls with
//! their names and arguments, the tool results, the preamble, the tool
//! definitions, the reply allowance, the Ollama context size, the Thinking
//! level and the source (provider, address and model). The hash does not
//! use Rig's JSON of the turn, because that JSON can change with a Rig
//! update or a `serde_json` feature (R14). Object keys go into the hash in
//! sorted order.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rig_core::completion::{AssistantContent, CompletionResponse, Message as RigMessage};
use rig_core::message::{ToolResultContent, UserContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use session_core::Stage;
use sha2::{Digest, Sha256};
use summarize::client::Label;
use summarize::provider::ThinkingLevel;
use summarize::{AgentReply, AgentTurn, Client, SummarizeError};
use summary_agent::{BoxFuture, Model};

use super::Source;
use crate::{Control, SharedStore, StageError, checkpoint, lock};

/// The start of each turn key. An older build's layer keys start with `l`
/// and two digits, so the two forms never match (R14).
pub const TURN_PREFIX: &str = "turn-";

/// The version of the canonical form. A change to the form changes it, so
/// that no key of the old form matches a turn of the new form.
const FORM: &str = "longhand turn key 1";

/// The saved reply of a turn.
#[derive(Serialize, Deserialize)]
struct SavedTurn {
    response: CompletionResponse,
    served_by: Option<String>,
    cost: Option<f64>,
}

/// The model of the summary pipeline: the app's client with a saved reply
/// for each turn, the control before each turn, the cancel flag and the
/// typed error of the last failed turn.
pub struct Turns<'a> {
    store: &'a SharedStore,
    note_id: &'a str,
    client: &'a Client,
    control: &'a dyn Control,
    source: Source,
    cancelled: AtomicBool,
    error: Mutex<Option<SummarizeError>>,
}

impl<'a> Turns<'a> {
    /// The layer for the turns of the note `note_id`, sent by `client`. The
    /// replies are saved in `store` under the Summarize stage.
    pub fn new(
        store: &'a SharedStore,
        note_id: &'a str,
        client: &'a Client,
        control: &'a dyn Control,
    ) -> Self {
        Self {
            store,
            note_id,
            client,
            control,
            source: Source::of(client),
            cancelled: AtomicBool::new(false),
            error: Mutex::new(None),
        }
    }

    /// True when a cancel stopped a turn. The agent gets the cancel as an
    /// error, so the caller must read this flag before it uses an outcome.
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// The typed error of the last turn, when that turn failed (KTD10).
    pub fn error(&self) -> Option<SummarizeError> {
        self.kept().clone()
    }

    /// Forget the kept error. The caller does this at the start of each
    /// agent run, so that an error of an earlier run is not used.
    pub fn clear_error(&self) {
        *self.kept() = None;
    }

    fn kept(&self) -> std::sync::MutexGuard<'_, Option<SummarizeError>> {
        self.error.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The checkpoint key of `turn` (KTD4).
    pub fn key(&self, turn: &AgentTurn) -> String {
        let mut form = Form::default();
        form.text(FORM);
        form.text(&self.source.provider);
        form.text(&self.source.address);
        form.text(&self.source.model);
        form.text(thinking_name(self.client.thinking().level));
        form.number(u64::from(turn.max_tokens));
        match turn.num_ctx {
            Some(size) => {
                form.text("num_ctx");
                form.number(u64::from(size));
            }
            None => form.text("no num_ctx"),
        }
        form.text(&turn.preamble);
        form.number(turn.tools.len() as u64);
        for tool in &turn.tools {
            form.text(tool.name.as_str());
            form.text(&tool.description);
            form.json(&tool.parameters);
        }
        form.number(turn.history.len() as u64 + 1);
        for message in turn.history.iter().chain([&turn.prompt]) {
            form.message(message);
        }
        let digest = form.hash.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        format!("{TURN_PREFIX}{hex}")
    }

    /// The saved reply of `key`, or `None`. A row that does not decode is
    /// deleted, and the turn is sent again (KTD3).
    fn saved(&self, key: &str) -> Option<AgentReply> {
        // The guard goes before the match, because a block below locks the
        // store again.
        let read = lock(self.store).checkpoint_value(self.note_id, Stage::Summarize, key);
        let payload = match read {
            Ok(payload) => payload?,
            Err(e) => {
                log::warn!("could not read the saved summary turn {key}: {e}");
                return None;
            }
        };
        match serde_json::from_str::<SavedTurn>(&payload) {
            Ok(saved) => Some(AgentReply {
                response: saved.response,
                served_by: saved.served_by,
                cost: saved.cost,
            }),
            Err(e) => {
                log::warn!("the saved summary turn {key} does not decode ({e}); sending it again");
                let store = lock(self.store);
                if let Err(e) = store.delete_checkpoint(self.note_id, Stage::Summarize, key) {
                    log::warn!("could not delete the saved summary turn {key}: {e}");
                }
                None
            }
        }
    }

    /// Save the reply of `key`. A failure to save loses only the saved
    /// reply, so the turn continues.
    fn save(&self, key: &str, reply: &AgentReply) {
        let saved = SavedTurn {
            response: reply.response.clone(),
            served_by: reply.served_by.clone(),
            cost: reply.cost,
        };
        if let Err(e) = checkpoint::put(self.store, self.note_id, Stage::Summarize, key, &saved) {
            log::warn!("could not save the summary turn {key}: {e}");
        }
    }

    /// One turn. A turn that ends with a reply forgets the kept error, and
    /// a turn that fails keeps its error, so the kept error is always the
    /// error of the last turn.
    async fn run(&self, label: &Label, turn: &AgentTurn) -> Result<AgentReply, SummarizeError> {
        if self.cancelled() {
            return Err(stopped());
        }
        match self.control.proceed() {
            Ok(()) => {}
            Err(StageError::Cancelled) => {
                self.cancelled.store(true, Ordering::SeqCst);
                return Err(stopped());
            }
            Err(e) => {
                let error = SummarizeError::Transport(e.to_string());
                *self.kept() = Some(error.clone());
                return Err(error);
            }
        }
        let key = self.key(turn);
        if let Some(reply) = self.saved(&key) {
            self.clear_error();
            return Ok(reply);
        }
        match self.client.agent_turn(label, turn).await {
            Ok(reply) => {
                self.save(&key, &reply);
                self.clear_error();
                Ok(reply)
            }
            Err(e) => {
                *self.kept() = Some(e.clone());
                Err(e)
            }
        }
    }
}

impl Model for Turns<'_> {
    fn turn<'a>(
        &'a self,
        label: &'a Label,
        turn: &'a AgentTurn,
    ) -> BoxFuture<'a, Result<AgentReply, SummarizeError>> {
        Box::pin(self.run(label, turn))
    }
}

/// The error that ends the agent after a cancel. The caller reads
/// [`Turns::cancelled`] and does not show it.
fn stopped() -> SummarizeError {
    SummarizeError::Transport("the summary was cancelled".into())
}

fn thinking_name(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::ModelDefault => "thinking model default",
        ThinkingLevel::Off => "thinking off",
        ThinkingLevel::Low => "thinking low",
        ThinkingLevel::Medium => "thinking medium",
        ThinkingLevel::High => "thinking high",
    }
}

/// The canonical form of a turn, written into a hash. Each text has its
/// length before it, so two fields never join into the same bytes.
#[derive(Default)]
struct Form {
    hash: Sha256,
}

impl Form {
    fn number(&mut self, value: u64) {
        self.hash.update(value.to_le_bytes());
    }

    fn text(&mut self, text: &str) {
        self.number(text.len() as u64);
        self.hash.update(text.as_bytes());
    }

    /// A JSON value, with the keys of each object in sorted order.
    fn json(&mut self, value: &Value) {
        match value {
            Value::Null => self.text("null"),
            Value::Bool(value) => self.text(if *value { "true" } else { "false" }),
            Value::Number(value) => {
                self.text("number");
                self.text(&value.to_string());
            }
            Value::String(value) => {
                self.text("string");
                self.text(value);
            }
            Value::Array(items) => {
                self.text("array");
                self.number(items.len() as u64);
                for item in items {
                    self.json(item);
                }
            }
            Value::Object(map) => self.object(map),
        }
    }

    fn object(&mut self, map: &serde_json::Map<String, Value>) {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        self.text("object");
        self.number(keys.len() as u64);
        for key in keys {
            self.text(key);
            self.json(&map[key]);
        }
    }

    /// A message. Call ids are not in the form: they do not change what
    /// the model reads. Content kinds that the pipeline does not send
    /// (images, audio, video, documents and opaque blocks) give only their
    /// kind.
    fn message(&mut self, message: &RigMessage) {
        match message {
            RigMessage::System { content } => {
                self.text("system");
                self.text(content);
            }
            RigMessage::User { content } => {
                self.text("user");
                self.number(content.len() as u64);
                for part in content {
                    self.user(part);
                }
            }
            RigMessage::Assistant(message) => {
                self.text("assistant");
                self.number(message.content.len() as u64);
                for part in &message.content {
                    self.assistant(part);
                }
            }
        }
    }

    fn user(&mut self, part: &UserContent) {
        match part {
            UserContent::Text(text) => {
                self.text("text");
                self.text(&text.text);
            }
            UserContent::ToolResult(result) => {
                self.text("tool result");
                self.text(result.name.as_str());
                self.text(if result.is_error { "error" } else { "ok" });
                self.number(result.content.len() as u64);
                for item in &result.content {
                    match item {
                        ToolResultContent::Text(text) => {
                            self.text("text");
                            self.text(&text.text);
                        }
                        ToolResultContent::Json { value } => {
                            self.text("json");
                            self.json(value);
                        }
                        ToolResultContent::Image(_) => self.text("image"),
                    }
                }
            }
            UserContent::Image(_) => self.text("image"),
            UserContent::Audio(_) => self.text("audio"),
            UserContent::Video(_) => self.text("video"),
            UserContent::Document(_) => self.text("document"),
        }
    }

    fn assistant(&mut self, part: &AssistantContent) {
        match part {
            AssistantContent::Text(text) => {
                self.text("text");
                self.text(&text.text);
            }
            AssistantContent::ToolCall(call) => {
                self.text("tool call");
                self.text(call.function.name.as_str());
                self.object(&call.function.arguments);
                match &call.function.invalid_arguments {
                    Some(text) => {
                        self.text("invalid arguments");
                        self.text(text);
                    }
                    None => self.text("valid arguments"),
                }
            }
            AssistantContent::Reasoning(reasoning) => {
                self.text("reasoning");
                self.text(&reasoning.text);
                self.text(if reasoning.redacted {
                    "redacted"
                } else {
                    "plain"
                });
            }
            AssistantContent::Image(_) => self.text("image"),
            _ => self.text("other"),
        }
    }
}
