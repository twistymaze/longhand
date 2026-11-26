//! The agent shapes of KTD13 on the same loop.
//!
//! - `readers`: a lead agent has the note tools and [`ReadPart`]. Each
//!   `read_part` call starts a reader: a new agent run with its own context,
//!   which reads one line range of the transcript with get note and returns
//!   notes. A reader has no `read_part`, so there is one level only.
//! - `steps`: [`run_steps`] runs one agent step for each transcript part,
//!   with the part in its instruction, and a merge step that writes the
//!   summary. Each step can call the note tools.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use note_tools::NoteTools;
use rig_core::completion::ToolDefinition;
use rig_core::message::ToolName;
use serde_json::{Value, json};
use summarize::chunk::Chunk;
use summarize::client::Label;
use summarize::prompts::{self, Details, UserText};
use summarize::tokens::BYTES_PER_TOKEN;

use crate::{
    Agent, BoxFuture, CallResult, HistoryPolicy, Limits, Model, Outcome, RunResult, RunTotals,
    ToolSet, ToolSource,
};

/// The name of the reader tool.
pub const READ_PART: &str = "read_part";

/// The largest part in lines that a reader can read in its own context: the
/// reader's budget less its reply and its fixed text, at the transcript's
/// mean line size. JSON quoting makes a line about a fifth longer in a tool
/// result.
pub fn largest_part(transcript_chars: usize, transcript_lines: usize, limits: &Limits) -> usize {
    let free = free_tokens(limits) as f64;
    let line_bytes = (transcript_chars.max(1) as f64 / transcript_lines.max(1) as f64) * 1.2;
    ((free * BYTES_PER_TOKEN / line_bytes) as usize).max(1)
}

/// True when the whole transcript fits in the room for the tool results of
/// one turn ([`Limits::results_room`]). Then one agent reads the
/// transcript itself and needs no readers. JSON quoting makes a line about
/// a fifth longer in a tool result.
pub fn transcript_fits(transcript_chars: usize, limits: &Limits) -> bool {
    let tokens = transcript_chars as f64 * 1.2 / BYTES_PER_TOKEN;
    tokens <= limits.results_room() as f64
}

/// The transcript lines `first` to `last` of the note `note_id`, with
/// their time stamps.
pub(crate) fn transcript_lines(
    root: &std::path::Path,
    note_id: &str,
    first: usize,
    last: usize,
) -> Result<String, String> {
    let output = NoteTools::new(usize::MAX)
        .call(
            root,
            <note_tools::GetNote as note_tools::Tool>::NAME,
            json!({
                "note_id": note_id,
                "transcript": {"from_line": first, "to_line": last}
            }),
        )
        .map_err(|e| e.to_string())?;
    let hunks = output
        .pointer("/transcript/hunks")
        .and_then(Value::as_array)
        .ok_or("the note has no transcript lines in that range")?;
    Ok(hunks
        .iter()
        .filter_map(|h| h.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// The tokens of a context that hold no fixed text and no reply.
pub(crate) fn free_tokens(limits: &Limits) -> u64 {
    const FIXED: u64 = 3_000;
    limits
        .context_tokens
        .saturating_sub(u64::from(limits.reply_reserve()) + FIXED)
}

/// The notes size of each reader, in tokens: a third of the lead's free
/// context, shared by the parts that cover the whole transcript. A reader
/// is asked for this size, and its notes are cut at twice this size, so
/// that the notes of all parts take at most two thirds of the lead's free
/// context. The rest is for the lead's own messages.
pub fn reader_notes_tokens(lead: &Limits, transcript_lines: usize, largest_part: usize) -> u32 {
    let parts = transcript_lines.div_ceil(largest_part.max(1)).max(1) as u64;
    u32::try_from((free_tokens(lead) / 3 / parts).max(1)).unwrap_or(u32::MAX)
}

/// The `read_part` tool of a lead agent (KTD13).
pub struct ReadPart<'a> {
    pub model: &'a dyn Model,
    pub root: PathBuf,
    pub note_id: String,
    pub background: String,
    pub details: Details,
    /// The limits of each reader.
    pub limits: Limits,
    /// The size cap of one get note result of a reader.
    pub max_chars: usize,
    pub largest_part: usize,
    pub label: Label,
    /// The notes size of each reader ([`reader_notes_tokens`]). `None`: no
    /// size.
    pub notes_tokens: Option<u32>,
    readers: AtomicUsize,
}

impl<'a> ReadPart<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: &'a dyn Model,
        root: PathBuf,
        note_id: String,
        background: String,
        details: Details,
        limits: Limits,
        max_chars: usize,
        largest_part: usize,
        label: Label,
    ) -> Self {
        Self {
            model,
            root,
            note_id,
            background,
            details,
            limits,
            max_chars,
            largest_part,
            label,
            notes_tokens: None,
            readers: AtomicUsize::new(0),
        }
    }

    /// Ask each reader for notes of about `tokens` tokens.
    pub fn with_notes_tokens(mut self, tokens: u32) -> Self {
        self.notes_tokens = Some(tokens);
        self
    }

    /// `notes` cut at twice the notes size, with a marker.
    fn cut(&self, notes: &str) -> String {
        let Some(tokens) = self.notes_tokens else {
            return notes.to_string();
        };
        let most = (f64::from(tokens) * 2.0 * BYTES_PER_TOKEN) as usize;
        if notes.chars().count() <= most {
            return notes.to_string();
        }
        let kept: String = notes.chars().take(most).collect();
        format!(
            "{kept}\n[The reader's notes were cut here: they were longer than the size the lead can hold.]"
        )
    }

    fn lines(&self, first: usize, last: usize) -> Result<String, String> {
        transcript_lines(&self.root, &self.note_id, first, last)
    }

    async fn read(&self, arguments: Value) -> CallResult {
        let line = |key: &str| {
            arguments
                .get(key)
                .and_then(Value::as_u64)
                .map(|n| n as usize)
        };
        let (Some(first), Some(last)) = (line("first_line"), line("last_line")) else {
            return CallResult::error("read_part needs first_line and last_line");
        };
        if first == 0 || last < first {
            return CallResult::error(
                "last_line must not be before first_line, and lines start at 1",
            );
        }
        if last - first + 1 > self.largest_part {
            return CallResult::error(format!(
                "a part has at most {} lines: ask for a smaller range",
                self.largest_part
            ));
        }
        let instruction = arguments
            .get("instruction")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let lines = match self.lines(first, last) {
            Ok(lines) => lines,
            Err(error) => return CallResult::error(error),
        };
        let number = self.readers.fetch_add(1, Ordering::SeqCst) + 1;
        let user = UserText {
            background: &self.background,
            details: &self.details,
        };
        // About 0.75 words for each token.
        let words = self.notes_tokens.map(|t| (t as usize * 3 / 4).max(1));
        let messages = prompts::reader(first, last, &lines, instruction, words, user);
        let tools = ToolSet::empty();
        let result = Box::pin(
            Agent {
                model: self.model,
                tools: &tools,
                preamble: messages[0].content.clone(),
                prompt: messages[1].content.clone(),
                limits: self.limits,
                history: HistoryPolicy::default(),
                label: self.label.clone(),
                name: format!("reader {number}"),
            }
            .run(),
        )
        .await;
        let mut call = match &result.summary {
            Some(notes) if result.outcome.has_summary() => CallResult::ok(self.cut(notes)),
            _ => CallResult::error(format!(
                "the reader of lines {first} to {last} stopped: {:?}{}",
                result.outcome,
                result
                    .detail
                    .as_deref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default()
            )),
        };
        call.nested = Some(result.totals);
        call.nested_turns = result.turns;
        call
    }
}

impl ToolSource for ReadPart<'_> {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition::new(
            ToolName::new(READ_PART).expect("a tool name"),
            format!(
                "Give a range of transcript lines, at most {} lines, to a reader. The reader \
                 reads only those lines in a new context and returns notes on them, with \
                 [hh:mm:ss] times. The instruction tells the reader what to look for.",
                self.largest_part
            ),
            json!({
                "type": "object",
                "properties": {
                    "first_line": {"type": "integer", "minimum": 1},
                    "last_line": {"type": "integer", "minimum": 1},
                    "instruction": {"type": "string"}
                },
                "required": ["first_line", "last_line"],
                "additionalProperties": false
            }),
        )]
    }

    fn call<'b>(&'b self, name: &'b str, arguments: Value) -> BoxFuture<'b, CallResult> {
        Box::pin(async move {
            if name != READ_PART {
                return CallResult::error(format!("there is no tool {name}"));
            }
            self.read(arguments).await
        })
    }
}

/// The input of a `steps` run.
pub struct Steps<'a> {
    pub model: &'a dyn Model,
    pub tools: &'a ToolSet<'a>,
    pub chunks: &'a [Chunk],
    pub note_id: &'a str,
    /// The summary prompt's text.
    pub text: &'a str,
    pub user: UserText<'a>,
    pub limits: Limits,
    pub label: Label,
}

/// Run the `steps` shape (KTD13): one agent step for each part, then the
/// merge step. A failed step ends the run with its outcome. The steps get
/// the transcript in their instructions, so a run with no tool call is a
/// summary too.
pub async fn run_steps(steps: Steps<'_>) -> RunResult {
    let mut totals = RunTotals::default();
    let mut turns = Vec::new();
    let mut notes = Vec::new();
    let total = steps.chunks.len();
    let agent = |preamble: String, prompt: String, name: String| Agent {
        model: steps.model,
        tools: steps.tools,
        preamble,
        prompt,
        limits: steps.limits,
        history: HistoryPolicy::default(),
        label: steps.label.clone(),
        name,
    };
    for (index, chunk) in steps.chunks.iter().enumerate() {
        let messages = prompts::step_part(chunk, index, total, steps.note_id, steps.user);
        let name = format!("step {}", index + 1);
        let result = agent(
            messages[0].content.clone(),
            messages[1].content.clone(),
            name,
        )
        .run()
        .await;
        totals.add_after(&result.totals);
        turns.extend(result.turns);
        match result.summary {
            Some(part) if result.outcome.has_summary() => notes.push(part),
            _ => {
                return RunResult {
                    outcome: result.outcome,
                    summary: None,
                    detail: result.detail,
                    totals,
                    turns,
                };
            }
        }
    }
    let parts: Vec<&str> = notes.iter().map(String::as_str).collect();
    let messages = prompts::step_merge(&parts, steps.note_id, steps.text, steps.user);
    let result = agent(
        messages[0].content.clone(),
        messages[1].content.clone(),
        "merge".into(),
    )
    .run()
    .await;
    totals.add_after(&result.totals);
    turns.extend(result.turns);
    let outcome = match result.outcome {
        Outcome::NoReads => Outcome::Summary,
        other => other,
    };
    RunResult {
        outcome,
        summary: result.summary,
        detail: result.detail,
        totals,
        turns,
    }
}
