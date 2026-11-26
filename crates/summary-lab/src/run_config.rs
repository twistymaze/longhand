//! The configuration of a harness run (R18, KTD13, KTD14). A run reads it
//! from a JSON file. The manifest keeps a copy, and `--resume` refuses a
//! configuration that is different.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use summarize::provider::ThinkingLevel;
use summarize::tokens::ReplyKind;
use summary_agent::{HistoryPolicy, Limits};

/// The agent shape of a run (KTD13).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// `single` when the whole transcript fits in the room for the tool
    /// results of one turn, else `readers`
    /// ([`summary_agent::shapes::transcript_fits`]).
    #[default]
    Auto,
    /// One agent with the note tools.
    Single,
    /// A lead agent with the note tools and `read_part`, which starts a
    /// reader agent in a new context.
    Readers,
    /// One code-driven step for each transcript part, then a merge step.
    Steps,
    /// Code-driven timeline parts, then one writer or a writer for each
    /// group of parts and a merge step
    /// ([`summary_agent::passes::Pass::timeline`]).
    Timeline,
}

/// The notes of a run: note ids, a `list_notes` input, or both. With both,
/// a note must be in both.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotesFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    /// A `list_notes` input object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<Value>,
}

/// One model of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelEntry {
    /// A model id with an automatic context budget.
    Id(String),
    Full {
        model: String,
        /// The context budget in tokens. Absent: automatic (KTD14).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_tokens: Option<u32>,
        /// The most output tokens that the server sends in one reply.
        /// Absent: Rig's catalog, else the server's model list. Set it when
        /// the server enforces less than it reports.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u32>,
        /// The reasoning level of the baseline and the agent: `off`, `low`,
        /// `medium` or `high`. Absent: the model's default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking: Option<ThinkingLevel>,
    },
}

impl ModelEntry {
    pub fn model(&self) -> &str {
        match self {
            ModelEntry::Id(model) | ModelEntry::Full { model, .. } => model,
        }
    }

    pub fn context_tokens(&self) -> Option<u32> {
        match self {
            ModelEntry::Id(_) => None,
            ModelEntry::Full { context_tokens, .. } => *context_tokens,
        }
    }

    pub fn output_tokens(&self) -> Option<u32> {
        match self {
            ModelEntry::Id(_) => None,
            ModelEntry::Full { output_tokens, .. } => *output_tokens,
        }
    }

    pub fn thinking(&self) -> Option<ThinkingLevel> {
        match self {
            ModelEntry::Id(_) => None,
            ModelEntry::Full { thinking, .. } => *thinking,
        }
    }
}

/// Limit values that replace the defaults of [`Limits`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_calls: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub malformed_steps: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_tokens: Option<u32>,
}

impl LimitsConfig {
    /// The limits of an agent run with the context budget `context_tokens`
    /// and the model's output maximum `output_max`.
    pub fn limits(&self, context_tokens: u32, output_max: Option<u32>) -> Limits {
        let default = Limits::default();
        Limits {
            model_calls: self.model_calls.unwrap_or(default.model_calls),
            tool_calls: self.tool_calls.unwrap_or(default.tool_calls),
            malformed_steps: self.malformed_steps.unwrap_or(default.malformed_steps),
            context_tokens: u64::from(context_tokens),
            reply_tokens: self.reply_tokens,
            output_max,
            num_ctx: None,
            // The lab measured the hosted rule.
            reply_kind: ReplyKind::Hosted,
        }
    }
}

/// The configuration of a harness run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    /// The name of the run folder.
    pub name: String,
    /// The storage folder to read.
    pub storage: PathBuf,
    pub notes: NotesFilter,
    pub models: Vec<ModelEntry>,
    #[serde(default)]
    pub shape: Shape,
    /// The reading strategy of the agent. Absent: the default strategy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(default)]
    pub history: HistoryPolicy,
    /// The id of the summary prompt. Absent: the prompt that the app uses
    /// for the note's type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// The number of agent runs for each note and model.
    #[serde(default = "default_runs")]
    pub runs: usize,
    /// The number of baselines for each note and model. More than one
    /// measures how much the baseline itself changes between samples.
    #[serde(default = "default_baselines")]
    pub baselines: usize,
    #[serde(default)]
    pub limits: LimitsConfig,
    /// The limits of each reader of the `readers` shape.
    #[serde(default)]
    pub reader_limits: LimitsConfig,
    /// The size cap of one get note result. Absent: [`result_chars`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
    /// The largest number of agent tokens (input and output) of the run.
    /// The run stops when its agent turns pass it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_cap: Option<u64>,
    /// Send only to OpenRouter hosts with zero data retention.
    #[serde(default)]
    pub zdr: bool,
    /// Keep the summary that each note has in the snapshot, so that the
    /// agent can read it. False: the agent writes the first summary, as the
    /// baseline does.
    #[serde(default)]
    pub existing_summary: bool,
    /// Auditors check the summary against the transcript and a reviser
    /// corrects it.
    #[serde(default)]
    pub review: bool,
    /// The judge models on OpenRouter (KTD9). An empty list judges nothing.
    #[serde(default = "default_judges")]
    pub judges: Vec<String>,
    /// The seed of the summary order for the judges. Absent: the run
    /// chooses one and records it in the manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

fn default_runs() -> usize {
    3
}

fn default_baselines() -> usize {
    1
}

/// The default judges: the strongest Claude model and a strong OpenAI
/// model. A judge call is a small part of the cost of a run.
pub fn default_judges() -> Vec<String> {
    vec![
        "anthropic/claude-opus-5.5".into(),
        "openai/gpt-6.2-sol".into(),
    ]
}

/// The largest `get_note` result in tokens when the run sets no
/// `max_chars`: about the 25,000-token cap of Claude Code's file read tool.
pub const RESULT_TOKENS: u64 = 25_000;

/// The size cap of one `get_note` result for a model with a context of
/// `context_tokens`: `max_chars` when set, else [`RESULT_TOKENS`] and at
/// most a quarter of the context, in characters.
pub fn result_chars(max_chars: Option<usize>, context_tokens: u32) -> usize {
    max_chars.unwrap_or_else(|| {
        let tokens = RESULT_TOKENS.min(u64::from(context_tokens) / 4);
        ((tokens as f64 * summarize::tokens::BYTES_PER_TOKEN) as usize).max(1)
    })
}

impl RunConfig {
    /// Parse and check the text of a configuration file.
    pub fn parse(text: &str) -> Result<Self, String> {
        let config: RunConfig =
            serde_json::from_str(text).map_err(|e| format!("the configuration: {e}"))?;
        config.check()?;
        Ok(config)
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::parse(&text)
    }

    fn check(&self) -> Result<(), String> {
        check_name(&self.name)?;
        if self.models.is_empty() {
            return Err("the configuration needs at least one model".into());
        }
        if self.notes.ids.is_empty() && self.notes.list.is_none() {
            return Err("the configuration needs notes.ids or a notes.list filter".into());
        }
        if let Some(list) = &self.notes.list
            && !list.is_object()
        {
            return Err("notes.list must be a list_notes input object".into());
        }
        if self.baselines == 0 {
            return Err("baselines must be 1 or more".into());
        }
        if self.runs == 0 {
            return Err("runs must be 1 or more".into());
        }
        if self.max_chars == Some(0) {
            return Err("max_chars must be 1 or more".into());
        }
        let mut judges = std::collections::BTreeSet::new();
        for judge in &self.judges {
            if !judges.insert(model_folder(judge)) {
                return Err(format!("the judge {judge} is in the list twice"));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in &self.models {
            if !seen.insert(model_folder(entry.model())) {
                return Err(format!(
                    "the model {} is in the list twice, or has the folder name of another model",
                    entry.model()
                ));
            }
            if entry.context_tokens().is_some_and(|t| t < 4_000) {
                return Err(format!(
                    "the context budget of {} must be 4000 tokens or more",
                    entry.model()
                ));
            }
        }
        Ok(())
    }
}

/// A run name is a folder name: letters, digits, `.`, `_` and `-`, with no
/// `.` at the start.
pub fn check_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "the run name {name:?} must have only letters, digits, '.', '_' and '-', and must not start with '.'"
        ))
    }
}

/// The folder name of a model id: the id with each character that is not
/// a letter, a digit, `.`, `_` or `-` as `_`.
pub fn model_folder(model: &str) -> String {
    model
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}
