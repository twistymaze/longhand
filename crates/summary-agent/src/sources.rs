//! Tool sources (KTD3). A tool source gives tool definitions and runs a
//! call. The loop takes its tools from a list of sources, and refuses a
//! name that more than one source gives.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use note_tools::{GetNote, NoteTools, Tool, ToolError};
use rig_core::completion::ToolDefinition;
use rig_core::message::ToolName;
use serde_json::{Value, json};

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
                Ok(Ok(value)) => CallResult::ok(value.to_string()),
                Ok(Err(error)) => CallResult {
                    text: error_text(&error),
                    is_error: true,
                    nested: None,
                    nested_turns: Vec::new(),
                },
                Err(_) => CallResult::error("the tool stopped unexpectedly"),
            }
        })
    }
}
