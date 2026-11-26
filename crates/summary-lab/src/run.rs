//! A harness run (U6): the run folder, the manifest, the baselines and the
//! agent runs, with `--resume` and `--dry-run`.
//!
//! ```text
//! <out>/<name>/manifest.json
//! <out>/<name>/snapshot/                       the storage folder copy
//! <out>/<name>/results/<note>/<model>/baseline.json
//! <out>/<name>/results/<note>/<model>/agent-<n>.json
//! <out>/baselines/<key>.json                   baselines for later runs
//! ```
//!
//! The baseline is the summary from the app's summary pipeline (timeline
//! parts, a writer and an audit) on an in-memory store (KTD7). A later run
//! uses it again while the note's files, the model, the endpoint, the
//! context budget and the summary prompt do not change (KTD8). Baseline
//! files from before the pipeline came from the old single-call method.
//! Their reuse key has no [`BASELINE_METHOD`], so no run uses them again.
//! No file holds a key.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use library::reconcile::{Reconciler, Target};
use library::scan::ScanOptions;
use note_tools::NoteTools;
use note_tools::storage::{NOTES_FILE, TRANSCRIPT_FILE};
use pipeline::summarize::SummaryEndpoint;
use pipeline::{Control, SharedStore, StageError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shared_settings::view::SharedView;
use summarize::chunk::{self, Chunk};
use summarize::client::{Label, RETRY_WAITS, RequestKind, Thinking};
use summarize::prompts::{self, AgentFacts, Details, UserText};
use summarize::provider::ThinkingLevel;
use summarize::{Client, Endpoint, OpenRouterRouting, Prompt, context, tokens};
use summary_agent::passes::{self, Pass};
use summary_agent::shapes::{
    ReadPart, Steps, largest_part, reader_notes_tokens, run_steps, transcript_fits,
};
use summary_agent::{Agent, Limits, MemoryNoteSource, NoteSource, Outcome, RunResult, ToolSet};

use crate::run_config::{RunConfig, Shape, model_folder, result_chars};
use crate::snapshot::{self, Selected, Skipped};
use crate::{judge, report};

/// The default folder of the runs, relative to the current folder.
pub const DEFAULT_OUT: &str = ".context/summary-lab";
pub const MANIFEST: &str = "manifest.json";
pub const SNAPSHOT: &str = "snapshot";
pub const RESULTS: &str = "results";
pub const BASELINES: &str = "baselines";

/// The summary method of the baselines, in each reuse key. Change it when
/// the app's summary method changes, so that no run uses a baseline from
/// another method.
pub const BASELINE_METHOD: &str = "pipeline";

/// How a run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A new run folder. An existing folder with the name is an error.
    New,
    /// Fill the missing result files of an existing run.
    Resume,
    /// Print the planned calls and write nothing.
    DryRun,
}

/// One note of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestNote {
    #[serde(flatten)]
    pub note: Selected,
    /// The fingerprint of the note's files in the storage folder. A
    /// snapshot without the summary file has the same fingerprint.
    pub fingerprint: String,
    pub prompt_id: String,
    pub prompt_name: String,
}

/// One model of a run, with its context budget (KTD14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestModel {
    pub model: String,
    pub folder: String,
    pub provider: String,
    pub context_tokens: u32,
    /// `set`, `catalog`, `probe` or `default`.
    pub budget_source: String,
    /// The model's output maximum: from Rig's catalog for a hosted
    /// provider, else from the server's model list (LiteLLM `/model/info`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_max: Option<u32>,
    /// The reasoning level of the baseline and the agent requests.
    #[serde(default = "model_default", skip_serializing_if = "is_model_default")]
    pub thinking_level: ThinkingLevel,
}

fn model_default() -> ThinkingLevel {
    ThinkingLevel::ModelDefault
}

fn is_model_default(level: &ThinkingLevel) -> bool {
    *level == ThinkingLevel::ModelDefault
}

impl ManifestModel {
    /// The client setting of the reasoning level. A level that the run
    /// sets is a chosen level: a model that refuses it fails the request.
    pub fn thinking(&self) -> Thinking {
        Thinking {
            level: self.thinking_level,
            chosen: self.thinking_level != ThinkingLevel::ModelDefault,
        }
    }
}

/// The manifest of a run folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub config: RunConfig,
    /// The start time, as RFC 3339.
    pub created: String,
    pub notes: Vec<ManifestNote>,
    pub skipped: Vec<Skipped>,
    pub models: Vec<ManifestModel>,
    /// The seed of the summary order for the judges.
    pub seed: u64,
    /// Why the last start of the run stopped before the end, if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
}

/// The baseline of one note and model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineFile {
    /// The reuse key (KTD8).
    pub key: String,
    /// True when an earlier run wrote the summary.
    pub reused: bool,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

/// One agent run of one note and model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentFile {
    pub run: usize,
    pub shape: Shape,
    pub result: RunResult,
}

/// What a run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunSummary {
    pub baselines_written: usize,
    pub baselines_reused: usize,
    pub agent_runs: usize,
    pub agent_tokens: u64,
    pub judge_files: usize,
    pub stopped: Option<String>,
}

/// The surroundings of a run.
pub struct Lab<'a> {
    /// The folder of the run folders.
    pub out: PathBuf,
    /// The endpoint of a model entry.
    pub resolve: &'a dyn Fn(&str) -> Result<Endpoint, String>,
    /// Progress lines, for a person who waits.
    pub log: &'a dyn Fn(&str),
}

/// Start, resume or plan the run of `config`.
pub async fn run(config: &RunConfig, mode: Mode, lab: &Lab<'_>) -> Result<RunSummary, String> {
    let folder = lab.out.join(&config.name);
    let manifest_path = folder.join(MANIFEST);
    let manifest = match mode {
        Mode::New if folder.exists() => {
            return Err(format!(
                "the run {} exists: choose another name, or use --resume",
                config.name
            ));
        }
        Mode::Resume => {
            let manifest = read_manifest(&manifest_path)?;
            if !same_config(&manifest.config, config) {
                return Err(format!(
                    "the configuration is not the configuration of the run {}: only token_cap can change on --resume",
                    config.name
                ));
            }
            Some(manifest)
        }
        _ => None,
    };
    let snapshot_root = folder.join(SNAPSHOT);
    let routing = OpenRouterRouting {
        deny_data_collection: true,
        zero_retention: config.zdr,
    };
    let mut manifest = match manifest {
        Some(mut manifest) => {
            manifest.config = config.clone();
            manifest
        }
        None => match new_manifest(config, mode, lab, &snapshot_root, routing).await {
            Ok(manifest) => manifest,
            Err(error) => {
                if mode == Mode::New {
                    let _ = std::fs::remove_dir_all(&folder);
                }
                return Err(error);
            }
        },
    };

    if mode == Mode::DryRun {
        print_plan(config, &manifest, &config.storage);
        return Ok(RunSummary::default());
    }
    manifest.stopped = None;
    write_json(&manifest_path, &manifest)?;

    let judges = config
        .judges
        .iter()
        .map(|judge| {
            let client = Client::new((lab.resolve)(judge)?)
                .map_err(|e| e.to_string())?
                .with_openrouter(routing);
            Ok((judge.clone(), client))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let view = shared_settings::view::load(&snapshot_root, None).view;
    let details_store = adopt(&snapshot_root)?;
    let mut notes = Vec::new();
    for note in &manifest.notes {
        let prompt = note_prompt(
            &view,
            config.prompt.as_deref(),
            note.note.note_type.as_deref(),
        )?;
        let details = pipeline::summarize::note_details(&details_store, &note.note.note_id)
            .map_err(|e| format!("the details of {}: {e}", note.note.note_id))?;
        notes.push((note, prompt, details));
    }
    let shared = Shared {
        config,
        lab,
        folder: &folder,
        snapshot_root: &snapshot_root,
        routing,
        seed: manifest.seed,
        judges: &judges,
        summary: Mutex::new(RunSummary {
            agent_tokens: spent_tokens(&folder.join(RESULTS))?,
            ..RunSummary::default()
        }),
    };
    // The models run at the same time; the notes of one model run in turn.
    let results = futures_util::future::join_all(
        manifest
            .models
            .iter()
            .map(|model| model_pairs(&shared, model, &notes)),
    )
    .await;
    let summary = shared.summary.into_inner().map_err(|e| e.to_string())?;
    if let Some(error) = results.into_iter().find_map(Result::err) {
        return Err(error);
    }
    if let Some(reason) = &summary.stopped {
        manifest.stopped = Some(reason.clone());
        write_json(&manifest_path, &manifest)?;
        return Ok(summary);
    }
    report::write(&folder)?;
    Ok(summary)
}

/// What the models of a run share.
struct Shared<'a> {
    config: &'a RunConfig,
    lab: &'a Lab<'a>,
    folder: &'a Path,
    snapshot_root: &'a Path,
    routing: OpenRouterRouting,
    seed: u64,
    judges: &'a [(String, Client)],
    summary: Mutex<RunSummary>,
}

impl Shared<'_> {
    fn count(&self, change: impl FnOnce(&mut RunSummary)) {
        if let Ok(mut summary) = self.summary.lock() {
            change(&mut summary);
        }
    }

    /// The reason to stop, when the token cap is reached. It records the
    /// reason for the other models.
    fn stop(&self) -> Option<String> {
        let mut summary = self.summary.lock().ok()?;
        if summary.stopped.is_some() {
            return summary.stopped.clone();
        }
        let cap = self.config.token_cap?;
        (summary.agent_tokens >= cap).then(|| {
            let reason = format!(
                "the agent turns used {} tokens, which is the token cap of {cap} or more: raise token_cap and use --resume",
                summary.agent_tokens
            );
            summary.stopped = Some(reason.clone());
            reason
        })
    }
}

/// The baselines, agent runs and judge results of `model` for each note.
async fn model_pairs(
    shared: &Shared<'_>,
    model: &ManifestModel,
    notes: &[(&ManifestNote, Prompt, Details)],
) -> Result<(), String> {
    let (config, lab, snapshot_root, routing) = (
        shared.config,
        shared.lab,
        shared.snapshot_root,
        shared.routing,
    );
    let endpoint = (lab.resolve)(&model.model)?;
    for (note, prompt, details) in notes {
        let dir = shared
            .folder
            .join(RESULTS)
            .join(&note.note.note_id)
            .join(&model.folder);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        // A result that failed for a reason outside the run is made
        // again, and then the judges of the pair score again.
        let mut changed = false;
        for sample in 1..=config.baselines {
            let baseline_path = dir.join(baseline_file(sample));
            if usable(&baseline_path, |b: &BaselineFile| b.summary.is_some()) {
                continue;
            }
            changed = true;
            (lab.log)(&format!(
                "baseline {sample}: {} with {}",
                note.note.note_id, model.model
            ));
            let file = baseline(
                &lab.out,
                snapshot_root,
                note,
                model,
                &endpoint,
                prompt,
                routing,
                sample,
            )
            .await?;
            shared.count(|s| {
                if file.reused {
                    s.baselines_reused += 1;
                } else {
                    s.baselines_written += 1;
                }
            });
            write_json(&baseline_path, &file)?;
        }
        for run in 1..=config.runs {
            let path = dir.join(format!("agent-{run}.json"));
            if usable(&path, |a: &AgentFile| {
                a.result.outcome != Outcome::Transport
            }) {
                continue;
            }
            changed = true;
            if shared.stop().is_some() {
                return Ok(());
            }
            (lab.log)(&format!(
                "agent run {run}: {} with {}",
                note.note.note_id, model.model
            ));
            let client = Client::new(endpoint.clone())
                .map_err(|e| e.to_string())?
                .with_openrouter(routing)
                .with_thinking(model.thinking());
            let (shape, result) =
                agent_run(config, snapshot_root, note, model, &client, prompt, details).await?;
            shared.count(|s| {
                s.agent_tokens += result.totals.input_tokens + result.totals.output_tokens;
                s.agent_runs += 1;
            });
            write_json(&path, &AgentFile { run, shape, result })?;
        }
        for (judge, client) in shared.judges {
            let path = dir.join(format!("judge-{}.json", model_folder(judge)));
            if !changed && usable(&path, |j: &judge::JudgeFile| j.scores.is_some()) {
                continue;
            }
            let Some((order, texts)) =
                candidates(&dir, config.runs, shared.seed, note, model, judge)?
            else {
                continue;
            };
            (lab.log)(&format!(
                "judge {judge}: {} with {}",
                note.note.note_id, model.model
            ));
            let summaries: Vec<&str> = texts.iter().map(String::as_str).collect();
            let messages = judge::messages(
                &part(snapshot_root, note, TRANSCRIPT_FILE),
                &part(snapshot_root, note, NOTES_FILE),
                &order,
                &summaries,
            );
            let seed = judge::pair_seed(shared.seed, &note.note.note_id, &model.model, judge);
            let file = judge::judge(client, judge, seed, order, &messages).await;
            shared.count(|s| s.judge_files += 1);
            write_json(&path, &file)?;
        }
    }
    Ok(())
}

/// The candidates of a judge request and their texts, in the same order.
type Judged = (Vec<judge::Candidate>, Vec<String>);

/// The summaries to judge for one note and model, in the order of the seed:
/// the baseline and each agent summary. `None` when the baseline has no
/// summary, because the loss needs it.
fn candidates(
    dir: &Path,
    runs: usize,
    seed: u64,
    note: &ManifestNote,
    model: &ManifestModel,
    judge: &str,
) -> Result<Option<Judged>, String> {
    let baseline: BaselineFile = read_json(&dir.join("baseline.json"))?;
    let Some(baseline) = baseline.summary else {
        return Ok(None);
    };
    let mut texts = vec![("baseline".to_string(), baseline)];
    for sample in 2.. {
        let path = dir.join(baseline_file(sample));
        if !path.exists() {
            break;
        }
        let file: BaselineFile = read_json(&path)?;
        if let Some(text) = file.summary {
            texts.push((baseline_source(sample), text));
        }
    }
    for run in 1..=runs {
        let path = dir.join(format!("agent-{run}.json"));
        if !path.exists() {
            continue;
        }
        let agent: AgentFile = read_json(&path)?;
        if agent.result.outcome.has_summary()
            && let Some(text) = agent.result.summary
        {
            texts.push((format!("agent-{run}"), text));
        }
    }
    let sources: Vec<String> = texts.iter().map(|(s, _)| s.clone()).collect();
    let order = judge::order(
        &sources,
        judge::pair_seed(seed, &note.note.note_id, &model.model, judge),
    );
    let ordered = order
        .iter()
        .map(|c| {
            texts
                .iter()
                .find(|(s, _)| *s == c.source)
                .map(|(_, t)| t.clone())
                .unwrap_or_default()
        })
        .collect();
    Ok(Some((order, ordered)))
}

/// Whether the result file at `path` exists, parses, and `keep` accepts it.
fn usable<T: serde::de::DeserializeOwned>(path: &Path, keep: impl Fn(&T) -> bool) -> bool {
    read_json::<T>(path).is_ok_and(|value| keep(&value))
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} does not parse: {e}", path.display()))
}

/// The manifest of a new run: the selected notes, the snapshot (not for a
/// dry run), the prompts and the context budgets.
async fn new_manifest(
    config: &RunConfig,
    mode: Mode,
    lab: &Lab<'_>,
    snapshot_root: &Path,
    routing: OpenRouterRouting,
) -> Result<Manifest, String> {
    let endpoints = config
        .models
        .iter()
        .map(|entry| (lab.resolve)(entry.model()))
        .collect::<Result<Vec<_>, _>>()?;
    let (selected, skipped) = snapshot::select(&config.storage, &config.notes)?;
    if selected.is_empty() {
        let reasons: Vec<String> = skipped
            .iter()
            .map(|s| format!("{}: {}", s.note, s.reason))
            .collect();
        return Err(format!(
            "the filter selects no note that the run can use. Skipped: {}",
            reasons.join("; ")
        ));
    }
    let source = if mode == Mode::DryRun {
        config.storage.clone()
    } else {
        snapshot::copy(
            &config.storage,
            snapshot_root,
            &selected,
            config.existing_summary,
        )?;
        snapshot_root.to_path_buf()
    };
    let view = shared_settings::view::load(&source, None).view;
    let mut notes = Vec::new();
    for note in selected {
        let prompt = note_prompt(&view, config.prompt.as_deref(), note.note_type.as_deref())?;
        notes.push(ManifestNote {
            fingerprint: snapshot::fingerprint(&config.storage.join(&note.folder))?,
            prompt_id: prompt.id,
            prompt_name: prompt.name,
            note,
        });
    }
    let mut models = Vec::new();
    for (entry, endpoint) in config.models.iter().zip(&endpoints) {
        let (context_tokens, source, output_max) = if mode == Mode::DryRun {
            let (limit, source) = automatic_budget_offline(entry.context_tokens(), endpoint);
            let output = context::catalog_output_max(endpoint.provider, &endpoint.model);
            (limit, source, output)
        } else {
            budget(entry.context_tokens(), endpoint, routing).await?
        };
        models.push(ManifestModel {
            model: entry.model().to_string(),
            folder: model_folder(entry.model()),
            provider: format!("{:?}", endpoint.provider).to_lowercase(),
            context_tokens,
            budget_source: source.into(),
            output_max: entry.output_tokens().or(output_max),
            thinking_level: entry.thinking().unwrap_or(ThinkingLevel::ModelDefault),
        });
    }
    Ok(Manifest {
        config: config.clone(),
        created: chrono::Local::now().to_rfc3339(),
        notes,
        skipped,
        models,
        seed: config.seed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or_default()
        }),
        stopped: None,
    })
}

/// Two configurations are the same when only the token cap is different.
fn same_config(saved: &RunConfig, given: &RunConfig) -> bool {
    let mut given = given.clone();
    given.token_cap = saved.token_cap;
    *saved == given
}

pub fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read the manifest {}: {e}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("the manifest {} does not parse: {e}", path.display()))
}

/// Write `value` as JSON to `path` through a temporary file, so that a
/// stopped run leaves no part of a file.
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("json.part");
    std::fs::write(&temporary, text)
        .map_err(|e| format!("cannot write {}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The prompt of a note: the prompt `chosen`, else the prompt that the app
/// uses for the type `note_type` (`None` is General), with the Background
/// of `view`.
pub fn note_prompt(
    view: &SharedView,
    chosen: Option<&str>,
    note_type: Option<&str>,
) -> Result<Prompt, String> {
    let mut prompt = match chosen {
        Some(id) => prompts::built_in(id)
            .or_else(|| view.prompts.iter().find(|p| p.id == id).cloned())
            .ok_or_else(|| format!("no summary prompt has the id {id}"))?,
        None => {
            let valid = note_type.is_none_or(|name| {
                session_save::naming::check_type_name(name).as_deref() == Ok(name)
            });
            view.fields
                .note_type_prompts
                .known()
                .and_then(|map| shared_settings::note_types::prompt_choice(map, note_type, valid))
                .and_then(|id| view.type_prompts.get(id))
                .and_then(|p| p.prompt())
                .or_else(|| view.active.prompt())
                .cloned()
                .unwrap_or_else(prompts::topic_summary)
        }
    };
    prompt.background = view.background.clone();
    Ok(prompt)
}

/// The context budget of a model (KTD14): the set value, else the input
/// limit in Rig's catalog, else the probe, else the default.
/// The context budget, its source and the model's output maximum. A
/// model that the catalog does not know is probed, also when the run sets
/// its budget, because the probe gives the output maximum.
async fn budget(
    set: Option<u32>,
    endpoint: &Endpoint,
    routing: OpenRouterRouting,
) -> Result<(u32, &'static str, Option<u32>), String> {
    let catalog_limit = context::catalog_input_limit(endpoint.provider, &endpoint.model);
    let catalog_output = context::catalog_output_max(endpoint.provider, &endpoint.model);
    if let Some(limit) = catalog_limit {
        return Ok((
            set.unwrap_or(limit),
            if set.is_some() { "set" } else { "catalog" },
            catalog_output,
        ));
    }
    let client = Client::new(endpoint.clone())
        .map_err(|e| e.to_string())?
        .with_openrouter(routing);
    let probed = context::probe(&client, &Label::without_note(RequestKind::Probe)).await;
    let probed = match (set, probed) {
        (_, Ok(probed)) => probed,
        // The probe only adds the output maximum to a set budget.
        (Some(_), Err(_)) => None,
        (None, Err(e)) => return Err(format!("the context probe of {}: {e}", endpoint.model)),
    };
    let output = probed.and_then(|b| b.output_max).or(catalog_output);
    let budget = context::resolve(set, probed);
    let source = match (set, probed) {
        (Some(_), _) => "set",
        (None, Some(_)) => "probe",
        (None, None) => "default",
    };
    Ok((budget.limit, source, output))
}

/// The context budget for a dry run, with no network: the set value, else
/// the catalog, else the default.
fn automatic_budget_offline(set: Option<u32>, endpoint: &Endpoint) -> (u32, &'static str) {
    if let Some(limit) = set {
        return (limit, "set");
    }
    if let Some(limit) = context::catalog_input_limit(endpoint.provider, &endpoint.model) {
        return (limit, "catalog");
    }
    (
        context::resolve(None, None).limit,
        "default (a real run probes the model)",
    )
}

fn adopt(root: &Path) -> Result<SharedStore, String> {
    let store = session_core::Store::open_in_memory().map_err(|e| e.to_string())?;
    let store = Arc::new(Mutex::new(store));
    Reconciler::new(&ScanOptions::default())
        .run(&store, &Target::Full(vec![root.to_path_buf()]), &|_| false)
        .map_err(|e| format!("cannot read the snapshot: {e}"))?;
    Ok(store)
}

/// The control of a baseline: it never stops.
struct Always;

impl Control for Always {
    fn proceed(&self) -> Result<(), StageError> {
        Ok(())
    }
}

/// The file name of baseline `sample`: `baseline.json` for the first, so
/// that older run folders keep their file names.
pub fn baseline_file(sample: usize) -> String {
    if sample <= 1 {
        "baseline.json".into()
    } else {
        format!("baseline-{sample}.json")
    }
}

/// The judge source name of baseline `sample`.
pub fn baseline_source(sample: usize) -> String {
    if sample <= 1 {
        "baseline".into()
    } else {
        format!("baseline-{sample}")
    }
}

/// Baseline `sample` of `note` with `model`: a reused one, or a new one
/// from the app's summary pipeline. Each sample has its own reuse key, so that
/// the samples are separate summaries.
#[allow(clippy::too_many_arguments)]
async fn baseline(
    out: &Path,
    snapshot_root: &Path,
    note: &ManifestNote,
    model: &ManifestModel,
    endpoint: &Endpoint,
    prompt: &Prompt,
    routing: OpenRouterRouting,
    sample: usize,
) -> Result<BaselineFile, String> {
    let mut key = baseline_key(note, model, endpoint, prompt, routing);
    if sample > 1 {
        key = format!("{key}-{sample}");
    }
    let cache = out.join(BASELINES).join(format!("{key}.json"));
    if let Ok(text) = std::fs::read_to_string(&cache)
        && let Ok(mut file) = serde_json::from_str::<BaselineFile>(&text)
        && file.summary.is_some()
    {
        file.reused = true;
        return Ok(file);
    }
    let summary_endpoint = SummaryEndpoint {
        provider: endpoint.provider,
        base_url: endpoint.base_url.clone(),
        model: endpoint.model.clone(),
        credential: endpoint.credential.clone(),
        context_limit: Some(model.context_tokens),
        thinking: model.thinking(),
        requests_at_once: 4,
        retry_waits: RETRY_WAITS.to_vec(),
        openrouter: Some(routing),
    };
    let root = snapshot_root.to_path_buf();
    let note_id = note.note.note_id.clone();
    let folder = root.join(&note.note.folder);
    let prompt = prompt.clone();
    let start = Instant::now();
    // `produce` blocks on the current runtime, as in the app: it runs on
    // the blocking pool, not on an async worker thread (KTD7).
    let result = tokio::task::spawn_blocking(move || {
        adopt(&root).and_then(|store| {
            pipeline::summarize::produce(
                &store,
                &note_id,
                &folder,
                summary_endpoint,
                prompt,
                &Always,
                Duration::ZERO,
                &mut |_, _, _| {},
            )
            .map(|summary| summary.body)
            .map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|_| "the baseline task stopped".to_string())?;
    let file = BaselineFile {
        key,
        reused: false,
        elapsed_ms: start.elapsed().as_millis() as u64,
        summary: result.as_ref().ok().cloned(),
        error: result.err(),
    };
    if file.summary.is_some() {
        std::fs::create_dir_all(out.join(BASELINES))
            .map_err(|e| format!("cannot create the baseline folder: {e}"))?;
        write_json(&cache, &file)?;
    }
    Ok(file)
}

/// The reuse key of a baseline (KTD8): the summary method, the note's
/// files, the model, the endpoint, the context budget, the routing and the
/// summary prompt.
fn baseline_key(
    note: &ManifestNote,
    model: &ManifestModel,
    endpoint: &Endpoint,
    prompt: &Prompt,
    routing: OpenRouterRouting,
) -> String {
    let mut hash = Sha256::new();
    for part in [
        BASELINE_METHOD,
        note.fingerprint.as_str(),
        &note.note.note_id,
        &model.model,
        &format!("{:?}", endpoint.provider),
        &endpoint.base_url,
        &model.context_tokens.to_string(),
        &format!("{}{}", routing.deny_data_collection, routing.zero_retention),
        &prompt.text,
        &prompt.title_text,
        &prompt.background,
    ] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    // The model default adds nothing, so that the key of a baseline at the
    // model default does not change.
    if model.thinking_level != ThinkingLevel::ModelDefault {
        let part = format!("{:?}", model.thinking_level);
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

/// The tokens of the agent files under `results`.
fn spent_tokens(results: &Path) -> Result<u64, String> {
    let mut total = 0;
    let Ok(notes) = std::fs::read_dir(results) else {
        return Ok(0);
    };
    for note in notes.flatten() {
        for model in std::fs::read_dir(note.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            for file in std::fs::read_dir(model.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let name = file.file_name().to_string_lossy().to_string();
                if name.starts_with("agent-") && name.ends_with(".json") {
                    let text = std::fs::read_to_string(file.path()).map_err(|e| e.to_string())?;
                    let agent: AgentFile = serde_json::from_str(&text)
                        .map_err(|e| format!("{} does not parse: {e}", file.path().display()))?;
                    total += agent.result.totals.input_tokens + agent.result.totals.output_tokens;
                }
            }
        }
    }
    Ok(total)
}

fn part(root: &Path, note: &ManifestNote, name: &str) -> String {
    std::fs::read_to_string(root.join(&note.note.folder).join(name)).unwrap_or_default()
}

/// The start time of the note `note_id` in the storage folder `root`, as
/// get note gives it.
fn started(root: &Path, note_id: &str) -> Result<String, String> {
    let output = NoteTools::new(0)
        .call(root, "get_note", serde_json::json!({"note_id": note_id}))
        .map_err(|e| format!("the details of {note_id}: {e}"))?;
    output["started"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{note_id} has no start time"))
}

/// The transcript parts of the `steps` shape for a context budget.
fn step_chunks(transcript: &str, prompt: &Prompt, limits: &Limits, user: UserText) -> Vec<Chunk> {
    let lines = pipeline::summarize::lines_of(transcript);
    let reserve = limits.reply_reserve().saturating_add(2_000);
    let budget = u32::try_from(limits.context_tokens).unwrap_or(u32::MAX);
    let allowance = tokens::prompt_allowance(budget, reserve)
        .saturating_sub(prompts::overhead("", &prompt.text, user))
        .max(1);
    chunk::chunk(&lines, allowance)
}

/// One agent run of `note` with `model` in the shape of the run, and the
/// shape that it used (`auto` chooses one for each note).
async fn agent_run(
    config: &RunConfig,
    root: &Path,
    note: &ManifestNote,
    model: &ManifestModel,
    client: &Client,
    prompt: &Prompt,
    details: &Details,
) -> Result<(Shape, RunResult), String> {
    let note_id = note.note.note_id.as_str();
    let transcript = part(root, note, TRANSCRIPT_FILE);
    let facts = AgentFacts {
        note_id: note_id.to_string(),
        transcript_lines: transcript.lines().count(),
        transcript_chars: transcript.chars().count(),
        has_manual_notes: !part(root, note, NOTES_FILE).trim().is_empty(),
    };
    let strategy = config
        .strategy
        .as_deref()
        .unwrap_or(prompts::DEFAULT_STRATEGY);
    let user = UserText {
        background: &prompt.background,
        details,
    };
    let limits = config.limits.limits(
        model.context_tokens,
        model.output_max.or(client.output_max()),
    );
    let max_chars = result_chars(config.max_chars, model.context_tokens);
    let label = Label::note(note_id, RequestKind::AgentTurn);
    let source = NoteSource {
        root: root.to_path_buf(),
        tools: NoteTools::new(max_chars),
        only: None,
    };
    let shape = match config.shape {
        Shape::Auto if transcript_fits(facts.transcript_chars, &limits) => Shape::Single,
        Shape::Auto => Shape::Readers,
        shape => shape,
    };
    let pass = Pass {
        model: client,
        note: MemoryNoteSource::new(
            note_id.to_string(),
            started(root, note_id)?,
            transcript.clone(),
            part(root, note, NOTES_FILE),
            details.clone(),
            max_chars,
        ),
        background: prompt.background.clone(),
        text: prompt.text.clone(),
        limits,
        label: label.clone(),
        // The lab runs one auditor at a time and reports no progress.
        at_once: 1,
        progress: None,
    };
    let result = match shape {
        Shape::Auto => unreachable!("auto is resolved above"),
        Shape::Single => {
            let tools = ToolSet::new(vec![&source])?;
            let messages = prompts::agent(&facts, strategy, &prompt.text, None, user);
            Agent {
                model: client,
                tools: &tools,
                preamble: messages[0].content.clone(),
                prompt: messages[1].content.clone(),
                limits,
                history: config.history,
                label: label.clone(),
                name: "agent".into(),
            }
            .run()
            .await
        }
        Shape::Readers => {
            let reader_limits = config.reader_limits.limits(
                model.context_tokens,
                model.output_max.or(client.output_max()),
            );
            let largest = largest_part(
                facts.transcript_chars,
                facts.transcript_lines,
                &reader_limits,
            );
            let read_part = ReadPart::new(
                client,
                root.to_path_buf(),
                note_id.to_string(),
                prompt.background.clone(),
                details.clone(),
                reader_limits,
                max_chars,
                largest,
                label.clone(),
            )
            .with_notes_tokens(reader_notes_tokens(
                &limits,
                facts.transcript_lines,
                largest,
            ));
            let tools = ToolSet::new(vec![&source, &read_part])?;
            let messages = prompts::agent(&facts, strategy, &prompt.text, Some(largest), user);
            Agent {
                model: client,
                tools: &tools,
                preamble: messages[0].content.clone(),
                prompt: messages[1].content.clone(),
                limits,
                history: config.history,
                label: label.clone(),
                name: "lead".into(),
            }
            .run()
            .await
        }
        Shape::Steps => {
            let tools = ToolSet::new(vec![&source])?;
            let chunks = step_chunks(&transcript, prompt, &limits, user);
            run_steps(Steps {
                model: client,
                tools: &tools,
                chunks: &chunks,
                note_id,
                text: &prompt.text,
                user,
                limits,
                label,
            })
            .await
        }
        Shape::Timeline => {
            let tools = ToolSet::new(vec![&source])?;
            pass.timeline(&tools, facts.transcript_lines, facts.transcript_chars)
                .await
                .result
        }
    };
    let mut result = result;
    if config.review {
        let Some(draft) = result.written() else {
            return Ok((shape, result));
        };
        let reviewed = pass
            .review(&draft, facts.transcript_lines, facts.transcript_chars)
            .await;
        result = passes::join(result, reviewed);
    }
    Ok((shape, result))
}

/// The largest number of model calls of one agent run of a note with the
/// transcript `transcript`, at the limits of the run.
pub fn planned_agent_calls(
    config: &RunConfig,
    model: &ManifestModel,
    transcript: &str,
    prompt: &Prompt,
) -> usize {
    let limits = config.limits.limits(model.context_tokens, None);
    let shape = match config.shape {
        Shape::Auto if transcript_fits(transcript.chars().count(), &limits) => Shape::Single,
        Shape::Auto => Shape::Readers,
        shape => shape,
    };
    match shape {
        Shape::Auto | Shape::Single => limits.model_calls,
        Shape::Readers => {
            let reader = config.reader_limits.limits(model.context_tokens, None);
            limits.model_calls + limits.tool_calls * reader.model_calls
        }
        Shape::Steps => {
            let details = Details::default();
            let user = UserText {
                background: &prompt.background,
                details: &details,
            };
            (step_chunks(transcript, prompt, &limits, user).len() + 1) * limits.model_calls
        }
        Shape::Timeline => {
            let lines = transcript.lines().count();
            let size = largest_part(transcript.chars().count(), lines, &limits);
            // Each part, a writer for each part at most, and the merge.
            (2 * lines.div_ceil(size.max(1)) + 1) * limits.model_calls
        }
    }
}

/// Print the planned calls of a dry run.
fn print_plan(config: &RunConfig, manifest: &Manifest, root: &Path) {
    println!(
        "run {}: {} notes, {} skipped, {} models, shape {:?}, {} agent runs each",
        config.name,
        manifest.notes.len(),
        manifest.skipped.len(),
        manifest.models.len(),
        config.shape,
        config.runs
    );
    for skipped in &manifest.skipped {
        println!("skip\t{}\t{}", skipped.note, skipped.reason);
    }
    let view = shared_settings::view::load(root, None).view;
    println!("model\tcontext budget\tbaselines\tmost agent model calls");
    let mut total = 0;
    for model in &manifest.models {
        let mut calls = 0;
        for note in &manifest.notes {
            let prompt = note_prompt(
                &view,
                config.prompt.as_deref(),
                note.note.note_type.as_deref(),
            )
            .unwrap_or_else(|_| prompts::topic_summary());
            let transcript = part(root, note, TRANSCRIPT_FILE);
            calls += config.runs * planned_agent_calls(config, model, &transcript, &prompt);
        }
        total += calls;
        println!(
            "{}\t{} ({})\t{}\t{calls}",
            model.model,
            model.context_tokens,
            model.budget_source,
            manifest.notes.len()
        );
    }
    println!("at most {total} agent model calls in all, plus the baselines that are not reused");
}
