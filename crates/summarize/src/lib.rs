//! Summarization through one OpenAI-compatible endpoint (KTD16, R17, R18).
//! Written for Longhand; nothing upstream summarizes a long transcript.
//!
//! The pieces: [`context`] discovers how much the endpoint's model takes,
//! [`models`] lists the model ids that the endpoint offers,
//! [`tokens`] estimates what a prompt costs, [`chunk`] packs whole turns
//! under the budget when a transcript does not fit in one call, [`prompts`]
//! frames the transcript as quoted data and holds the summary prompts, and
//! [`client`] speaks chat completions under deadlines, following no
//! redirects. [`transport`] is the HTTP client that Rig sends through,
//! under the same rules.

pub mod chunk;
pub mod client;
pub mod context;
pub mod models;
pub mod prompts;
pub mod tokens;
pub mod transport;

pub use chunk::{Chunk, Line, Turn};
pub use client::{Client, Completion, Endpoint, Limits, SummarizeError};
pub use context::{Budget, BudgetSource};
pub use models::{ModelList, list_models};
pub use prompts::Prompt;
pub use transport::{Transport, TransportError};
