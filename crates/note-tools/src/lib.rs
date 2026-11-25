//! The read-only note tools (KTD2): list notes and get note. Each tool has
//! one definition: a name, a description for a model, an input JSON Schema,
//! an output JSON Schema and a call from typed input to typed output. One
//! JSON-in, JSON-out entry point ([`call`]) serves every front end: the
//! summary agent, the harness and a later CLI or MCP server.
//!
//! The tools read the plain files of one storage folder. They write
//! nothing, and they read no file outside the storage folder (R1, R6, R7).

pub mod get_note;
pub mod list_notes;
pub mod storage;

use std::path::Path;

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub use get_note::{GetNote, GetNoteInput, GetNoteOutput};
pub use list_notes::{ListNotes, ListNotesInput, ListNotesOutput};

/// The kind of a tool error. A front end shows the kind and the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The input does not match the input schema, or a value in it is not
    /// valid.
    InvalidInput,
    /// No note has the note id.
    NotFound,
    /// More than one folder has the note id.
    Ambiguous,
    /// The folder is not a storage folder: it has no `.longhand/library`.
    NotAStorageFolder,
    /// The storage folder or the note folder cannot be read.
    Unreadable,
}

/// A tool error, with a message that a model or a person can act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct ToolError {
    pub kind: ErrorKind,
    pub message: String,
}

impl ToolError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }
}

/// One tool, with typed input and output.
pub trait Tool {
    type Input: DeserializeOwned + JsonSchema;
    type Output: Serialize + JsonSchema;

    const NAME: &'static str;

    /// The description for a model.
    fn description(&self) -> String;

    /// Run the tool on the storage folder `root`.
    fn call(&self, root: &Path, input: Self::Input) -> Result<Self::Output, ToolError>;
}

/// The definition of a tool for a front end: the name, the description and
/// the two schemas.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Definition {
    pub name: &'static str,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
}

fn definition<T: Tool>(tool: &T) -> Definition {
    Definition {
        name: T::NAME,
        description: tool.description(),
        input_schema: schemars::schema_for!(T::Input).to_value(),
        output_schema: schemars::schema_for!(T::Output).to_value(),
    }
}

fn call_typed<T: Tool>(tool: &T, root: &Path, input: Value) -> Result<Value, ToolError> {
    let input: T::Input = serde_json::from_value(input)
        .map_err(|e| ToolError::invalid(format!("the input of {} is not valid: {e}", T::NAME)))?;
    let output = tool.call(root, input)?;
    serde_json::to_value(output)
        .map_err(|e| ToolError::new(ErrorKind::Unreadable, format!("the result: {e}")))
}

/// The note tools of one front end. The front end sets the size cap of a
/// get note result from its own budget (KTD11).
#[derive(Debug, Clone)]
pub struct NoteTools {
    pub list_notes: ListNotes,
    pub get_note: GetNote,
}

impl NoteTools {
    /// The tools, with `max_chars` as the size cap of a get note result.
    pub fn new(max_chars: usize) -> Self {
        Self {
            list_notes: ListNotes::default(),
            get_note: GetNote { max_chars },
        }
    }

    /// The definition of each tool.
    pub fn definitions(&self) -> Vec<Definition> {
        vec![definition(&self.list_notes), definition(&self.get_note)]
    }

    /// Run the tool `name` on the storage folder `root` with the JSON
    /// input `input`, and return the JSON result.
    pub fn call(&self, root: &Path, name: &str, input: Value) -> Result<Value, ToolError> {
        match name {
            ListNotes::NAME => call_typed(&self.list_notes, root, input),
            GetNote::NAME => call_typed(&self.get_note, root, input),
            other => Err(ToolError::invalid(format!("there is no tool {other}"))),
        }
    }
}
