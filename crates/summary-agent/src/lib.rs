//! The summary agent loop (KTD1). The loop drives `rig-agent`'s
//! [`AgentRun`] step by step: a model step goes through the summary
//! client's agent turn, a tool step goes to the tool sources, and the
//! budget check runs before each model step. It returns an outcome and a
//! record of each turn (R14, R15).
//!
//! The loop never logs a prompt, a tool result or a reply. A turn record
//! holds the result text for the harness's run folder.

pub mod compaction;
pub mod history;
pub mod passes;
pub mod shapes;
pub mod sources;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use rig_agent::run::{
    AgentRun, AgentRunStep, InvalidToolCallAction, ModelTurn, ModelTurnOutcome, PendingToolCall,
    PromptError, TurnPolicy,
};
use rig_core::completion::{Message, ToolDefinition};
use rig_core::tool::{ToolOutput, ToolResult};
use rig_memory::HeuristicTokenCounter;
use serde::{Deserialize, Serialize};
use summarize::client::Label;
use summarize::tokens::{self, ReplyKind};
use summarize::{AgentReply, AgentTurn, Client, SummarizeError};

use crate::compaction::Compacted;

pub use history::HistoryPolicy;
pub use sources::{BoxFuture, CallResult, NoteSource, ToolSource};

/// A model that answers agent turns. The summary client is one; a test
/// gives scripted turns.
pub trait Model: Send + Sync {
    fn turn<'a>(
        &'a self,
        label: &'a Label,
        turn: &'a AgentTurn,
    ) -> BoxFuture<'a, Result<AgentReply, SummarizeError>>;
}

impl Model for Client {
    fn turn<'a>(
        &'a self,
        label: &'a Label,
        turn: &'a AgentTurn,
    ) -> BoxFuture<'a, Result<AgentReply, SummarizeError>> {
        Box::pin(self.agent_turn(label, turn))
    }
}

/// How an agent run ended. Only `Summary` and `NoReads` give a summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// A summary after at least one tool call.
    Summary,
    /// A summary with no tool call.
    NoReads,
    /// The endpoint refused the request because it carries tools.
    NoToolSupport,
    ToolCallLimit,
    ContextExceeded,
    /// Too many tool steps in a row had only calls with arguments that are
    /// not JSON objects.
    MalformedCalls,
    /// The final reply had no text.
    NoSummary,
    /// The final reply was cut off at its length limit.
    CutOff,
    /// Another failure of the request.
    Transport,
}

impl Outcome {
    pub fn has_summary(self) -> bool {
        matches!(self, Self::Summary | Self::NoReads)
    }
}

/// The limits of one agent run.
///
/// `model_calls`, `tool_calls` and `malformed_steps` are loop guards: they
/// stop a model that repeats itself. They do not limit cost; the provider
/// account does. A three-hour transcript takes about ten `get_note` reads
/// at the default result size, so the guards leave room for several full
/// reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub model_calls: usize,
    pub tool_calls: usize,
    /// Tool steps in a row with only malformed calls.
    pub malformed_steps: usize,
    /// The context budget in tokens. Tool results count in it.
    pub context_tokens: u64,
    /// A fixed reply allowance. `None`: the model's output maximum, cut to
    /// the room that the request leaves, by the rule of the app's summary
    /// path ([`summarize::tokens::reply_allowance`]).
    pub reply_tokens: Option<u32>,
    /// The model's output maximum, when Longhand knows it.
    pub output_max: Option<u32>,
    /// The context size for an Ollama request.
    pub num_ctx: Option<u32>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            model_calls: 100,
            tool_calls: 200,
            malformed_steps: 3,
            context_tokens: 128_000,
            reply_tokens: None,
            output_max: None,
            num_ctx: None,
        }
    }
}

impl Limits {
    fn context_u32(&self) -> u32 {
        u32::try_from(self.context_tokens).unwrap_or(u32::MAX)
    }

    /// The reply allowance of a request of about `request` tokens.
    pub fn reply_allowance(&self, request: u64) -> u32 {
        let used = usize::try_from(request).unwrap_or(usize::MAX);
        let room =
            tokens::reply_allowance(self.context_u32(), used, self.output_max, ReplyKind::Hosted)
                .ceiling;
        self.reply_tokens.map_or(room, |fixed| fixed.min(room))
    }

    /// What a request can hold, in tokens: the context less the reply
    /// reserve.
    pub fn request_room(&self) -> u64 {
        self.context_tokens
            .saturating_sub(u64::from(self.reply_reserve()))
    }

    /// The room for the tool results of one turn, in tokens: half of the
    /// room of a request.
    pub fn results_room(&self) -> u64 {
        self.request_room() / 2
    }

    /// The room that a request keeps for its reply: the fixed allowance, or
    /// else the app's split reserve ([`summarize::tokens::split_reserve`]).
    /// Below it, the run compacts its history or ends.
    pub fn reply_reserve(&self) -> u32 {
        self.reply_tokens
            .unwrap_or_else(|| tokens::split_reserve(self.context_u32()))
    }
}

/// The kind of a turn record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnKind {
    Model,
    Tool,
    /// A model call that compacted the history.
    Compaction,
}

/// One model call or one tool call of a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnRecord {
    /// The agent run that made the turn: "agent", "lead", "reader 2",
    /// "step 1", "merge".
    pub agent: String,
    pub kind: TurnKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub argument_chars: usize,
    pub result_chars: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    /// The loop's estimate of the request, in tokens, before the call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub elapsed_ms: u64,
    pub error: bool,
    /// The tool arguments and result, for the run folder. Never logged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// The totals of a run, with the runs that it started.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunTotals {
    pub model_calls: usize,
    pub tool_calls: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// True when a reply reported no usage, so the token totals miss it.
    pub usage_incomplete: bool,
    pub cost: f64,
    pub elapsed_ms: u64,
}

impl RunTotals {
    /// Add the counters of a run that a tool call started. Its time is
    /// part of the tool call's time, so `elapsed_ms` does not change.
    pub fn add(&mut self, other: &RunTotals) {
        self.model_calls += other.model_calls;
        self.tool_calls += other.tool_calls;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.usage_incomplete |= other.usage_incomplete;
        self.cost += other.cost;
    }

    /// Add the counters and the time of a run that came after this one.
    pub fn add_after(&mut self, other: &RunTotals) {
        self.add(other);
        self.elapsed_ms += other.elapsed_ms;
    }
}

/// The result of an agent run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// More about a failure, from Longhand's own error text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub totals: RunTotals,
    pub turns: Vec<TurnRecord>,
}

/// The tools of a run, from its sources, by name.
pub struct ToolSet<'a> {
    sources: Vec<&'a dyn ToolSource>,
    by_name: BTreeMap<String, usize>,
    definitions: Vec<ToolDefinition>,
}

impl<'a> ToolSet<'a> {
    /// The tools of `sources`. A name that two sources give is refused.
    pub fn new(sources: Vec<&'a dyn ToolSource>) -> Result<Self, String> {
        let mut by_name = BTreeMap::new();
        let mut definitions = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            for definition in source.definitions() {
                let name = definition.name.as_str().to_string();
                if by_name.insert(name.clone(), index).is_some() {
                    return Err(format!("two tool sources give the tool {name}"));
                }
                definitions.push(definition);
            }
        }
        Ok(Self {
            sources,
            by_name,
            definitions,
        })
    }

    pub fn names(&self) -> BTreeSet<String> {
        self.by_name.keys().cloned().collect()
    }

    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    async fn call(&self, name: &str, arguments: serde_json::Value) -> CallResult {
        match self.by_name.get(name) {
            Some(&index) => self.sources[index].call(name, arguments).await,
            None => CallResult::error(format!("there is no tool {name}")),
        }
    }
}

/// One agent run: its model, tools, first instruction and limits.
pub struct Agent<'a> {
    pub model: &'a dyn Model,
    pub tools: &'a ToolSet<'a>,
    pub preamble: String,
    pub prompt: String,
    pub limits: Limits,
    pub history: HistoryPolicy,
    pub label: Label,
    /// The name of the run in turn records.
    pub name: String,
}

/// The estimate of `turn` in tokens, by Rig's heuristic counter.
fn estimate(turn: &AgentTurn) -> u64 {
    let counter = HeuristicTokenCounter::default();
    let tools: usize = turn.tools.iter().map(|t| counter.count_tool(t)).sum();
    let messages: usize = turn
        .history
        .iter()
        .chain([&turn.prompt])
        .map(|m| counter.count(m))
        .sum();
    let preamble = counter.count(&Message::system(turn.preamble.clone()));
    (tools + messages + preamble) as u64
}

/// The estimate of `text` in tokens, by Rig's heuristic counter.
fn text_tokens(text: &str) -> u64 {
    HeuristicTokenCounter::default().count(&Message::user(text)) as u64
}

/// The outcome for an error of a model call.
fn outcome_of(error: &SummarizeError) -> Outcome {
    match error {
        SummarizeError::ContextExceeded => Outcome::ContextExceeded,
        SummarizeError::ReplyCutOff => Outcome::CutOff,
        SummarizeError::Http {
            status: 400 | 404,
            excerpt,
        } if refuses_tools(excerpt) => Outcome::NoToolSupport,
        _ => Outcome::Transport,
    }
}

/// True when the text of a refusal says that the endpoint does not take
/// the tools: OpenRouter's "No endpoints found that support tool use", or
/// a server's "does not support tools".
fn refuses_tools(excerpt: &str) -> bool {
    let lower = excerpt.to_lowercase();
    (lower.contains("tool") || lower.contains("provided parameters"))
        && (lower.contains("support") || lower.contains("no endpoints"))
}

struct Driver<'a, 'b> {
    agent: &'b Agent<'a>,
    turns: Vec<TurnRecord>,
    totals: RunTotals,
    /// The reported input tokens over the estimate, from the last call.
    ratio: f64,
    started: Instant,
}

impl Driver<'_, '_> {
    fn end(
        mut self,
        outcome: Outcome,
        summary: Option<String>,
        detail: Option<String>,
    ) -> RunResult {
        self.totals.elapsed_ms = self.started.elapsed().as_millis() as u64;
        RunResult {
            outcome,
            summary,
            detail,
            totals: self.totals,
            turns: self.turns,
        }
    }

    fn record(&mut self, kind: TurnKind) -> TurnRecord {
        TurnRecord {
            agent: self.agent.name.clone(),
            kind,
            tool: None,
            argument_chars: 0,
            result_chars: 0,
            input_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
            estimated_tokens: None,
            cost: None,
            host: None,
            elapsed_ms: 0,
            error: false,
            arguments: None,
            result: None,
        }
    }
}

impl Agent<'_> {
    /// Run the agent to its outcome.
    pub async fn run(&self) -> RunResult {
        let mut driver = Driver {
            agent: self,
            turns: Vec::new(),
            totals: RunTotals::default(),
            ratio: 1.0,
            started: Instant::now(),
        };
        let names = self.tools.names();
        let mut run =
            AgentRun::new(Message::user(self.prompt.clone())).max_turns(self.limits.model_calls);
        let mut malformed_steps = 0;
        let mut compacted: Option<Compacted> = None;
        // This run's own tool calls. The totals also count the calls of the
        // runs that a tool call started.
        let mut own_calls = 0;
        loop {
            let step = match run.next_step() {
                Ok(step) => step,
                Err(error) => return self.prompt_error(driver, error),
            };
            match step {
                AgentRunStep::CallModel {
                    prompt, history, ..
                } => {
                    let reserve = u64::from(self.limits.reply_reserve());
                    let mut turn = AgentTurn {
                        preamble: self.preamble.clone(),
                        history: self.sent_history(&history, compacted.as_ref()),
                        prompt: (*prompt).clone(),
                        tools: self.tools.definitions().to_vec(),
                        max_tokens: 0,
                        num_ctx: self.limits.num_ctx,
                    };
                    let mut raw = estimate(&turn);
                    let mut next = (raw as f64 * driver.ratio) as u64;
                    if self.history == HistoryPolicy::Compact
                        && next + reserve > self.limits.context_tokens
                    {
                        match self
                            .compact(&mut driver, &history, compacted.as_ref())
                            .await
                        {
                            Ok(Some(done)) => {
                                compacted = Some(done);
                                turn.history = self.sent_history(&history, compacted.as_ref());
                                raw = estimate(&turn);
                                next = (raw as f64 * driver.ratio) as u64;
                            }
                            Ok(None) => {}
                            Err((outcome, detail)) => {
                                return driver.end(outcome, None, Some(detail));
                            }
                        }
                    }
                    let mut record = driver.record(TurnKind::Model);
                    record.estimated_tokens = Some(next);
                    if next + reserve > self.limits.context_tokens {
                        let detail = format!(
                            "the next request needs about {next} tokens and room for a reply of {reserve}, more than the budget of {}",
                            self.limits.context_tokens
                        );
                        return driver.end(Outcome::ContextExceeded, None, Some(detail));
                    }
                    turn.max_tokens = self.limits.reply_allowance(next);
                    let started = Instant::now();
                    let reply = self.model.turn(&self.label, &turn).await;
                    record.elapsed_ms = started.elapsed().as_millis() as u64;
                    driver.totals.model_calls += 1;
                    let reply = match reply {
                        Ok(reply) => reply,
                        Err(error) => {
                            record.error = true;
                            driver.turns.push(record);
                            let detail =
                                format!("{error} (reply allowance {} tokens)", turn.max_tokens);
                            return driver.end(outcome_of(&error), None, Some(detail));
                        }
                    };
                    let usage = reply.response.usage;
                    record.input_tokens = usage.input_tokens;
                    record.output_tokens = usage.output_tokens;
                    record.reasoning_tokens = usage.reasoning_tokens;
                    record.cost = reply.cost;
                    record.host = reply.served_by.clone();
                    match usage.input_tokens {
                        Some(input) if raw > 0 => {
                            driver.ratio = (input as f64 / raw as f64).clamp(0.5, 4.0);
                        }
                        _ => {}
                    }
                    driver.totals.input_tokens += usage.input_tokens.unwrap_or(0);
                    driver.totals.output_tokens += usage.output_tokens.unwrap_or(0);
                    driver.totals.usage_incomplete |=
                        usage.input_tokens.is_none() || usage.output_tokens.is_none();
                    driver.totals.cost += reply.cost.unwrap_or(0.0);
                    driver.turns.push(record);
                    let policy = match TurnPolicy::new(names.clone(), None, None) {
                        Ok(policy) => policy,
                        Err(e) => return driver.end(Outcome::Transport, None, Some(e.to_string())),
                    };
                    let mut outcome =
                        run.model_response(ModelTurn::from_policy(&reply.response, policy));
                    loop {
                        match outcome {
                            Ok(ModelTurnOutcome::NeedsResolution(context)) => {
                                let text = format!(
                                    "error: there is no tool {}; the tools are {}",
                                    context.tool_name,
                                    context.available_tools.join(", ")
                                );
                                outcome = run
                                    .resolve_invalid_tool_call(InvalidToolCallAction::skip(text));
                            }
                            Ok(_) => break,
                            Err(error) => return self.prompt_error(driver, error),
                        }
                    }
                }
                AgentRunStep::CallTools { calls } => {
                    let mut answers = Vec::new();
                    let (mut executed, mut malformed) = (false, false);
                    // The results of one turn arrive in one message, which
                    // compaction cannot shorten. They share half of the
                    // room that a request has; compaction can always bring
                    // the older messages into the other half.
                    let mut room = self.limits.results_room();
                    for call in calls {
                        match call {
                            PendingToolCall::Execute(call) => {
                                if own_calls >= self.limits.tool_calls {
                                    let detail = format!(
                                        "the run reached its limit of {} tool calls",
                                        self.limits.tool_calls
                                    );
                                    return driver.end(Outcome::ToolCallLimit, None, Some(detail));
                                }
                                driver.totals.tool_calls += 1;
                                own_calls += 1;
                                executed = true;
                                let name = call.name().as_str().to_string();
                                let arguments = call.arguments();
                                let mut record = driver.record(TurnKind::Tool);
                                let started = Instant::now();
                                let mut result = self.tools.call(&name, arguments.clone()).await;
                                record.elapsed_ms = started.elapsed().as_millis() as u64;
                                let size = (text_tokens(&result.text) as f64 * driver.ratio) as u64;
                                if size > room {
                                    let refused = CallResult::error(format!(
                                        "this result has about {size} tokens, and the results of \
                                         this turn have room for {room} more tokens. Ask for fewer \
                                         lines, or ask for them in a later turn."
                                    ));
                                    result.text = refused.text;
                                    result.is_error = true;
                                } else {
                                    room -= size;
                                }
                                if let Some(nested) = &result.nested {
                                    driver.totals.add(nested);
                                }
                                driver.turns.extend(result.nested_turns.iter().cloned());
                                let arguments = arguments.to_string();
                                record.tool = Some(name);
                                record.argument_chars = arguments.chars().count();
                                record.result_chars = result.text.chars().count();
                                record.error = result.is_error;
                                record.arguments = Some(arguments);
                                record.result = Some(result.text.clone());
                                driver.turns.push(record);
                                answers.push(
                                    call.answer(ToolResult::success(ToolOutput::text(result.text))),
                                );
                            }
                            PendingToolCall::Malformed(call) => {
                                malformed = true;
                                let text = format!(
                                    "error: the arguments of {} are not a JSON object; send them again as one JSON object",
                                    call.name().as_str()
                                );
                                answers.push(call.answer(Some(InvalidToolCallAction::skip(text))));
                            }
                            _ => {
                                return driver.end(
                                    Outcome::Transport,
                                    None,
                                    Some("the run gave a tool call of an unknown kind".into()),
                                );
                            }
                        }
                    }
                    if executed {
                        malformed_steps = 0;
                    } else if malformed {
                        malformed_steps += 1;
                        if malformed_steps >= self.limits.malformed_steps {
                            let detail = format!(
                                "{malformed_steps} tool steps in a row had only calls with arguments that are not JSON objects"
                            );
                            return driver.end(Outcome::MalformedCalls, None, Some(detail));
                        }
                    }
                    if let Err(error) = run.answer_all(answers) {
                        return self.prompt_error(driver, error);
                    }
                }
                AgentRunStep::Done(response) => {
                    let text = response.output().trim().to_string();
                    if text.is_empty() {
                        return driver.end(Outcome::NoSummary, None, None);
                    }
                    let outcome = if own_calls == 0 {
                        Outcome::NoReads
                    } else {
                        Outcome::Summary
                    };
                    return driver.end(outcome, Some(text), None);
                }
            }
        }
    }

    /// The messages that a request sends before its prompt.
    fn sent_history(
        &self,
        history: &[Arc<Message>],
        compacted: Option<&Compacted>,
    ) -> Vec<Message> {
        match compacted {
            Some(compacted) => compaction::sent(history, compacted),
            None => history::apply(self.history, history),
        }
    }

    /// Replace the older messages of `history` with notes from the model.
    /// `Ok(None)` when nothing can be compacted.
    async fn compact(
        &self,
        driver: &mut Driver<'_, '_>,
        history: &[Arc<Message>],
        previous: Option<&Compacted>,
    ) -> Result<Option<Compacted>, (Outcome, String)> {
        let start = previous.map_or(1, |p| p.kept);
        let room = self.limits.request_room();
        let keep = compaction::keep_recent_tokens(room);
        let Some(kept) = compaction::cut(history, start, keep) else {
            return Ok(None);
        };
        // About 0.75 words for each token.
        let words = (compaction::notes_tokens(room) as usize * 3 / 4).max(1);
        let mut turn = AgentTurn {
            preamble: compaction::COMPACTION_RULES.to_string(),
            history: Vec::new(),
            prompt: Message::user(compaction::request_text(
                previous.map(|p| p.notes.as_str()),
                &history[start..kept],
                words,
            )),
            tools: Vec::new(),
            max_tokens: 0,
            num_ctx: self.limits.num_ctx,
        };
        let next = (estimate(&turn) as f64 * driver.ratio) as u64;
        turn.max_tokens = self.limits.reply_allowance(next);
        let mut record = driver.record(TurnKind::Compaction);
        record.estimated_tokens = Some(next);
        let started = Instant::now();
        let reply = self.model.turn(&self.label, &turn).await;
        record.elapsed_ms = started.elapsed().as_millis() as u64;
        driver.totals.model_calls += 1;
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                record.error = true;
                driver.turns.push(record);
                return Err((outcome_of(&error), format!("compaction: {error}")));
            }
        };
        let usage = reply.response.usage;
        record.input_tokens = usage.input_tokens;
        record.output_tokens = usage.output_tokens;
        record.reasoning_tokens = usage.reasoning_tokens;
        record.cost = reply.cost;
        record.host = reply.served_by.clone();
        driver.totals.input_tokens += usage.input_tokens.unwrap_or(0);
        driver.totals.output_tokens += usage.output_tokens.unwrap_or(0);
        driver.totals.usage_incomplete |=
            usage.input_tokens.is_none() || usage.output_tokens.is_none();
        driver.totals.cost += reply.cost.unwrap_or(0.0);
        let notes: String = reply
            .response
            .choice
            .iter()
            .filter_map(|c| match c {
                rig_core::message::AssistantContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        record.result_chars = notes.chars().count();
        record.result = Some(notes.clone());
        driver.turns.push(record);
        if notes.trim().is_empty() {
            return Err((
                Outcome::ContextExceeded,
                "compaction: the notes were empty".into(),
            ));
        }
        // Notes longer than twice their size are cut, so that the request
        // after the compaction fits.
        let most = (compaction::notes_tokens(room) as f64 * 2.0 * tokens::BYTES_PER_TOKEN) as usize;
        let notes = if notes.chars().count() > most {
            let kept: String = notes.chars().take(most).collect();
            format!("{kept}\n[The notes were cut here: they were too long for the context.]")
        } else {
            notes
        };
        Ok(Some(Compacted { kept, notes }))
    }

    fn prompt_error(&self, driver: Driver, error: PromptError) -> RunResult {
        match error {
            PromptError::MaxTurns { .. } => {
                let detail = format!(
                    "the run reached its limit of {} model calls",
                    self.limits.model_calls
                );
                driver.end(Outcome::ToolCallLimit, None, Some(detail))
            }
            other => driver.end(Outcome::Transport, None, Some(other.to_string())),
        }
    }
}
