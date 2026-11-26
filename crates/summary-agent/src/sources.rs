//! Tool sources (KTD3). A tool source gives tool definitions and runs a
//! call. The loop takes its tools from a list of sources, and refuses a
//! name that more than one source gives.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use note_tools::get_note::{PartText, TranscriptRequest, capped, transcript_part};
use note_tools::storage::{GENERAL, PartState};
use note_tools::{ErrorKind, GetNote, GetNoteInput, GetNoteOutput, NoteTools, Tool, ToolError};
use rig_core::completion::ToolDefinition;
use rig_core::message::ToolName;
use serde_json::{Value, json};
use summarize::prompts::Details;

/// A boxed future, so that a source can be a trait object.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The result of one tool call: the text that the model gets. An error is
/// text too, because the OpenAI route drops the error flag of a result.
#[derive(Debug, Clone, PartialEq)]
pub struct CallResult {
    pub text: String,
    pub is_error: bool,
    /// The totals of an agent run that the call started, for a reader.
    pub nested: Option<crate::RunTotals>,
    /// The turn records of that agent run.
    pub nested_turns: Vec<crate::TurnRecord>,
}

impl CallResult {
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
            nested: None,
            nested_turns: Vec::new(),
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: format!("error: {}", text.into()),
            is_error: true,
            nested: None,
            nested_turns: Vec::new(),
        }
    }
}

/// A set of tools that the loop can call.
pub trait ToolSource: Send + Sync {
    /// The definitions of the tools of the source.
    fn definitions(&self) -> Vec<ToolDefinition>;

    /// Run the tool `name` with the JSON arguments `arguments`.
    fn call<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, CallResult>;
}

/// The note tools on one storage folder, as a tool source. With `only`,
/// get note reads only that note, and list notes is not given (a reader).
pub struct NoteSource {
    pub root: PathBuf,
    pub tools: NoteTools,
    pub only: Option<String>,
}

fn definition(name: &str, description: String, parameters: Value) -> ToolDefinition {
    ToolDefinition::new(
        ToolName::new(name).expect("a tool name is not empty"),
        description,
        parameters,
    )
}

fn error_text(error: &ToolError) -> String {
    serde_json::to_string(&json!({"error": error.kind, "message": error.message}))
        .unwrap_or_else(|_| error.message.clone())
}

fn result_of(result: Result<Value, ToolError>) -> CallResult {
    match result {
        Ok(value) => CallResult::ok(value.to_string()),
        Err(error) => CallResult {
            text: error_text(&error),
            is_error: true,
            nested: None,
            nested_turns: Vec::new(),
        },
    }
}

/// One note in memory, as a tool source (KTD2). It holds the transcript
/// text, the manual notes and the details of the note, and gives get note
/// for this note only, with the schema, the line numbers and the size cap
/// of get note on a storage folder. It gives no summary, no other note and
/// no list notes. It reads no file, so it serves a note that no storage
/// folder has yet.
#[derive(Debug, Clone)]
pub struct MemoryNoteSource {
    pub note_id: String,
    /// The start time, as RFC 3339.
    pub started: String,
    /// The text of the transcript file. Line numbers count its raw lines
    /// from 1.
    transcript: String,
    /// The number of lines of the transcript.
    transcript_lines: usize,
    /// The number of characters of the transcript.
    transcript_chars: usize,
    pub manual_notes: String,
    pub details: Details,
    /// The size cap of the text in one get note result.
    pub max_chars: usize,
}

impl MemoryNoteSource {
    /// The note `note_id` with the start time `started` (RFC 3339), the
    /// text of its transcript file, its manual notes and its details.
    /// `max_chars` is the size cap of the text in one get note result.
    pub fn new(
        note_id: String,
        started: String,
        transcript: String,
        manual_notes: String,
        details: Details,
        max_chars: usize,
    ) -> Self {
        Self {
            note_id,
            started,
            transcript_lines: transcript.lines().count(),
            transcript_chars: transcript.chars().count(),
            transcript,
            manual_notes,
            details,
            max_chars,
        }
    }

    /// The number of lines of the transcript.
    pub fn transcript_lines(&self) -> usize {
        self.transcript_lines
    }

    /// The number of characters of the transcript.
    pub fn transcript_chars(&self) -> usize {
        self.transcript_chars
    }

    /// The transcript lines `first` to `last`, with their time stamps. The
    /// size cap of a tool result does not apply.
    pub fn lines(&self, first: usize, last: usize) -> Result<String, String> {
        let request = TranscriptRequest {
            from_line: Some(first),
            to_line: Some(last),
            ..TranscriptRequest::default()
        };
        let part =
            transcript_part(&self.transcript, &request, usize::MAX).map_err(|e| e.to_string())?;
        Ok(part
            .hunks
            .iter()
            .map(|h| h.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// Get note for `input`, as on a storage folder.
    fn read(&self, input: GetNoteInput) -> Result<GetNoteOutput, ToolError> {
        let id = input.note_id.trim();
        if id != self.note_id {
            return Err(ToolError::new(
                ErrorKind::NotFound,
                format!("no note has the id {id}"),
            ));
        }
        let mut budget = self.max_chars;
        let manual_notes = input.manual_notes.then(|| {
            let (text, truncated) = capped(&self.manual_notes, budget);
            budget = budget.saturating_sub(text.chars().count());
            PartText {
                state: PartState::Present,
                text: Some(text),
                truncated,
            }
        });
        let transcript = input
            .transcript
            .as_ref()
            .map(|request| transcript_part(&self.transcript, request, budget))
            .transpose()?;
        Ok(GetNoteOutput {
            note_id: self.note_id.clone(),
            title: self.details.title.clone(),
            note_type: self
                .details
                .note_type
                .clone()
                .unwrap_or_else(|| GENERAL.into()),
            tags: self.details.tags.clone(),
            started: self.started.clone(),
            busy: false,
            busy_reasons: Vec::new(),
            transcript_state: PartState::Present,
            transcript_lines: Some(self.transcript_lines),
            transcript_chars: Some(self.transcript_chars),
            manual_notes,
            summary: None,
            transcript,
        })
    }

    fn call_get_note(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        if name != GetNote::NAME {
            return Err(ToolError::invalid(format!("there is no tool {name}")));
        }
        let input: GetNoteInput = serde_json::from_value(arguments).map_err(|e| {
            ToolError::invalid(format!("the input of {} is not valid: {e}", GetNote::NAME))
        })?;
        let output = self.read(input)?;
        serde_json::to_value(output)
            .map_err(|e| ToolError::new(ErrorKind::Unreadable, format!("the result: {e}")))
    }
}

impl ToolSource for MemoryNoteSource {
    fn definitions(&self) -> Vec<ToolDefinition> {
        NoteTools::new(self.max_chars)
            .definitions()
            .into_iter()
            .filter(|d| d.name == GetNote::NAME)
            .map(|d| definition(d.name, d.description, d.input_schema))
            .collect()
    }

    fn call<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, CallResult> {
        Box::pin(async move { result_of(self.call_get_note(name, arguments)) })
    }
}

impl ToolSource for NoteSource {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .definitions()
            .into_iter()
            .filter(|d| self.only.is_none() || d.name == GetNote::NAME)
            .map(|d| definition(d.name, d.description, d.input_schema))
            .collect()
    }

    fn call<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, CallResult> {
        Box::pin(async move {
            if let Some(only) = &self.only {
                if name != GetNote::NAME {
                    return CallResult::error(format!("there is no tool {name}"));
                }
                if arguments.get("note_id").and_then(Value::as_str) != Some(only.as_str()) {
                    return CallResult::error(format!("you can read only the note {only}"));
                }
            }
            let (root, tools, name) = (self.root.clone(), self.tools.clone(), name.to_string());
            // The tools read files and can wait for a slow sync client.
            let result =
                tokio::task::spawn_blocking(move || tools.call(&root, &name, arguments)).await;
            match result {
                Ok(result) => result_of(result),
                Err(_) => CallResult::error("the tool stopped unexpectedly"),
            }
        })
    }
}
