//! Judging variants against each other. A run judges its agent summaries
//! only with its own baseline, so the scores of two runs come from
//! different judge requests and cannot be compared directly. `versus`
//! takes runs with the same notes and models (for example a control and a
//! variant) and puts, for each note, model and sample k, the baselines of
//! the first run and agent summary k of each run in one judge request,
//! with no labels and in a seeded order.
//!
//! The report gives, for each model and run, the absolute scores, the
//! summaries with no summary, the length and the error counts, and the
//! paired difference to the first run (the reference) for each note, with
//! a win count and a bootstrap interval over the notes.
//!
//! `versus` reads the summaries of existing runs and makes no summary.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use note_tools::storage::{NOTES_FILE, TRANSCRIPT_FILE};
use summarize::{Client, OpenRouterRouting};

use crate::cross::summary_of;
use crate::judge::{self, JudgeFile};
use crate::run::{
    Lab, MANIFEST, Manifest, ManifestNote, RESULTS, SNAPSHOT, baseline_file, baseline_source,
    read_json, write_json,
};
use crate::run_config::model_folder;

/// Judge requests that run at the same time.
const PARALLEL: usize = 4;

/// The resamples of the bootstrap interval.
const RESAMPLES: usize = 2_000;

/// What `versus` judges.
pub struct Spec {
    pub name: String,
    /// Run names. The first is the reference of the differences.
    pub runs: Vec<String>,
    pub judges: Vec<String>,
    pub seed: u64,
    /// Model folders to judge. Empty: every model of the first run.
    pub models: Vec<String>,
}

/// The sources and texts of one judge request.
struct Request {
    note: ManifestNote,
    snapshot: PathBuf,
    model: String,
    sample: usize,
    sources: Vec<String>,
    texts: Vec<String>,
}

impl Request {
    fn path(&self, folder: &Path, judge: &str) -> PathBuf {
        folder
            .join(&self.note.note.note_id)
            .join(&self.model)
            .join(format!(
                "judge-{}-{}.json",
                model_folder(judge),
                self.sample
            ))
    }
}

/// The source name of agent summaries of `run`.
fn run_source(run: &str) -> String {
    format!("run:{run}")
}

fn manifests(spec: &Spec, out: &Path) -> Result<Vec<Manifest>, String> {
    spec.runs
        .iter()
        .map(|run| read_json(&out.join(run).join(MANIFEST)))
        .collect()
}

fn requests(spec: &Spec, out: &Path) -> Result<Vec<Request>, String> {
    let manifests = manifests(spec, out)?;
    let first = &manifests[0];
    let samples = manifests.iter().map(|m| m.config.runs).max().unwrap_or(0);
    let mut requests = Vec::new();
    for note in &first.notes {
        let id = &note.note.note_id;
        for model in first
            .models
            .iter()
            .filter(|m| spec.models.is_empty() || spec.models.contains(&m.folder))
        {
            let dir = |run: &str| out.join(run).join(RESULTS).join(id).join(&model.folder);
            for sample in 1..=samples {
                let mut sources = Vec::new();
                let mut texts = Vec::new();
                for baseline in 1.. {
                    let path = dir(&spec.runs[0]).join(baseline_file(baseline));
                    if !path.exists() {
                        break;
                    }
                    if let Some(text) = summary_of(&path) {
                        sources.push(baseline_source(baseline));
                        texts.push(text);
                    }
                }
                for run in &spec.runs {
                    let path = dir(run).join(format!("agent-{sample}.json"));
                    if let Some(text) = summary_of(&path) {
                        sources.push(run_source(run));
                        texts.push(text);
                    }
                }
                if sources.iter().filter(|s| s.starts_with("run:")).count() > 0 {
                    requests.push(Request {
                        note: note.clone(),
                        snapshot: out.join(&spec.runs[0]).join(SNAPSHOT),
                        model: model.folder.clone(),
                        sample,
                        sources,
                        texts,
                    });
                }
            }
        }
    }
    Ok(requests)
}

/// Judge the runs against each other and write the judge files and
/// `report.md` to `<out>/<name>/`. A judge file that has scores is not
/// made again.
pub async fn versus(spec: &Spec, lab: &Lab<'_>) -> Result<String, String> {
    if spec.runs.len() < 2 {
        return Err("versus needs at least two runs".into());
    }
    let folder = lab.out.join(&spec.name);
    let routing = OpenRouterRouting {
        deny_data_collection: true,
        zero_retention: false,
    };
    let judges = spec
        .judges
        .iter()
        .map(|judge| {
            let client = Client::new((lab.resolve)(judge)?)
                .map_err(|e| e.to_string())?
                .with_openrouter(routing);
            Ok((judge.clone(), client))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let requests = requests(spec, &lab.out)?;
    let mut jobs = Vec::new();
    for request in &requests {
        for (judge, client) in &judges {
            let path = request.path(&folder, judge);
            if !read_json::<JudgeFile>(&path).is_ok_and(|f| f.scores.is_some()) {
                jobs.push((request, judge, client, path));
            }
        }
    }
    let results = futures_util::stream::iter(jobs)
        .map(|(request, judge, client, path)| async move {
            let id = &request.note.note.note_id;
            (lab.log)(&format!(
                "judge {judge}: {id} with {} sample {}",
                request.model, request.sample
            ));
            let read = |name: &str| {
                std::fs::read_to_string(request.snapshot.join(&request.note.note.folder).join(name))
                    .unwrap_or_default()
            };
            let key = format!("{}#{}", request.model, request.sample);
            let seed = judge::pair_seed(spec.seed, id, &key, judge);
            let order = judge::order(&request.sources, seed);
            let texts: Vec<&str> = order
                .iter()
                .map(|c| {
                    let i = request.sources.iter().position(|s| s == &c.source);
                    i.map_or("", |i| request.texts[i].as_str())
                })
                .collect();
            let messages =
                judge::messages(&read(TRANSCRIPT_FILE), &read(NOTES_FILE), &order, &texts);
            let file = judge::judge(client, judge, seed, order, &messages).await;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            write_json(&path, &file)
        })
        .buffer_unordered(PARALLEL)
        .collect::<Vec<_>>()
        .await;
    if let Some(error) = results.into_iter().find_map(Result::err) {
        return Err(error);
    }
    let report = report(spec, &lab.out, &requests, &folder)?;
    std::fs::write(folder.join("report.md"), &report)
        .map_err(|e| format!("cannot write the report: {e}"))?;
    Ok(report)
}

/// The values of one run and model.
#[derive(Default)]
struct Cell {
    overall: Vec<f64>,
    coverage: Vec<f64>,
    faithfulness: Vec<f64>,
    errors: Vec<f64>,
    words: Vec<f64>,
    /// The difference to the reference in the same request, by note.
    diffs: BTreeMap<String, Vec<f64>>,
    /// The difference to the reference, by judge.
    judge_diffs: BTreeMap<String, Vec<f64>>,
    agent_files: usize,
    no_summary: usize,
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn cell(value: Option<f64>, digits: usize) -> String {
    value.map_or("-".into(), |v| format!("{v:.digits$}"))
}

/// A next value of the xorshift sequence.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// The 5th and the 95th percentile of the mean of `values`, from
/// resamples with replacement. The seed makes the interval reproducible.
pub fn bootstrap(values: &[f64], seed: u64) -> Option<(f64, f64)> {
    if values.len() < 2 {
        return None;
    }
    let mut state = seed | 1;
    let mut means: Vec<f64> = (0..RESAMPLES)
        .map(|_| {
            let sum: f64 = (0..values.len())
                .map(|_| values[(next(&mut state) % values.len() as u64) as usize])
                .sum();
            sum / values.len() as f64
        })
        .collect();
    means.sort_by(|a, b| a.total_cmp(b));
    let at = |p: f64| means[((RESAMPLES as f64 - 1.0) * p).round() as usize];
    Some((at(0.05), at(0.95)))
}

/// The win, tie and loss counts of the per-note differences.
fn wins(diffs: &[f64]) -> (usize, usize, usize) {
    let win = diffs.iter().filter(|d| **d > 0.0).count();
    let loss = diffs.iter().filter(|d| **d < 0.0).count();
    (win, diffs.len() - win - loss, loss)
}

fn report(spec: &Spec, out: &Path, requests: &[Request], folder: &Path) -> Result<String, String> {
    let reference = run_source(&spec.runs[0]);
    let mut cells: BTreeMap<(String, String), Cell> = BTreeMap::new();
    for request in requests {
        for (source, text) in request.sources.iter().zip(&request.texts) {
            let entry = cells
                .entry((request.model.clone(), source.clone()))
                .or_default();
            entry.words.push(text.split_whitespace().count() as f64);
        }
        for judge in &spec.judges {
            let Ok(file) = read_json::<JudgeFile>(&request.path(folder, judge)) else {
                continue;
            };
            let base = file.score_of(&reference).map(|s| f64::from(s.overall));
            for source in &request.sources {
                let Some(score) = file.score_of(source) else {
                    continue;
                };
                let entry = cells
                    .entry((request.model.clone(), source.clone()))
                    .or_default();
                entry.overall.push(f64::from(score.overall));
                entry.coverage.push(f64::from(score.coverage));
                entry.faithfulness.push(f64::from(score.faithfulness));
                if let Some(e) = score.errors {
                    let total = e.missed
                        + e.unsupported
                        + e.misattributed
                        + e.out_of_order
                        + e.noise_as_fact;
                    entry.errors.push(f64::from(total));
                }
                if let Some(base) = base {
                    let diff = f64::from(score.overall) - base;
                    entry
                        .diffs
                        .entry(request.note.note.note_id.clone())
                        .or_default()
                        .push(diff);
                    entry
                        .judge_diffs
                        .entry(judge.clone())
                        .or_default()
                        .push(diff);
                }
            }
        }
    }
    // The agent runs with no summary, from the run folders.
    let manifests = manifests(spec, out)?;
    for (run, manifest) in spec.runs.iter().zip(&manifests) {
        for note in &manifest.notes {
            for model in manifest
                .models
                .iter()
                .filter(|m| spec.models.is_empty() || spec.models.contains(&m.folder))
            {
                let dir = out
                    .join(run)
                    .join(RESULTS)
                    .join(&note.note.note_id)
                    .join(&model.folder);
                let entry = cells
                    .entry((model.folder.clone(), run_source(run)))
                    .or_default();
                for sample in 1..=manifest.config.runs {
                    let path = dir.join(format!("agent-{sample}.json"));
                    if !path.exists() {
                        continue;
                    }
                    entry.agent_files += 1;
                    if summary_of(&path).is_none() {
                        entry.no_summary += 1;
                    }
                }
            }
        }
    }
    let mut out_text = format!(
        "# Versus {}\n\n- Runs: {} (the reference is {})\n- Judges: {}\n- Seed: {}\n\n\
         Each judge request holds the baselines of the reference run and agent summary k of \
         each run for one note and model, with no labels. Scores are means over notes, \
         samples and judges. The difference is the run's overall score less the reference's \
         overall score in the same request, as a mean for each note; wins, ties and losses \
         count notes, and the interval is a 90% bootstrap interval over the notes.\n",
        spec.name,
        spec.runs.join(", "),
        spec.runs[0],
        spec.judges.join(", "),
        spec.seed
    );
    let models: Vec<String> = cells
        .keys()
        .map(|(m, _)| m.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    for model in models {
        out_text.push_str(&format!(
            "\n## {model}\n\n| Source | Overall | Coverage | Faithfulness | Errors | Words | No summary | Difference | Interval | Wins/ties/losses | {} |\n|---|---|---|---|---|---|---|---|---|---|{}\n",
            spec.judges
                .iter()
                .map(|j| format!("Difference, {j}"))
                .collect::<Vec<_>>()
                .join(" | "),
            "---|".repeat(spec.judges.len())
        ));
        for ((m, source), c) in &cells {
            if *m != model {
                continue;
            }
            let per_note: Vec<f64> = c.diffs.values().filter_map(|d| mean(d)).collect();
            let is_run = source.starts_with("run:");
            let (w, t, l) = wins(&per_note);
            let seed = spec.seed ^ ((source.len() as u64) << 7);
            let interval = bootstrap(&per_note, seed)
                .map_or("-".into(), |(lo, hi)| format!("{lo:+.2} to {hi:+.2}"));
            let no_summary = if is_run && c.agent_files > 0 {
                format!("{}/{}", c.no_summary, c.agent_files)
            } else {
                "-".into()
            };
            out_text.push_str(&format!(
                "| {source} | {} | {} | {} | {} | {} | {no_summary} | {} | {interval} | {w}/{t}/{l} | {} |\n",
                cell(mean(&c.overall), 2),
                cell(mean(&c.coverage), 2),
                cell(mean(&c.faithfulness), 2),
                cell(mean(&c.errors), 1),
                cell(mean(&c.words), 0),
                cell(mean(&per_note), 2),
                spec.judges
                    .iter()
                    .map(|j| cell(c.judge_diffs.get(j).and_then(|d| mean(d)), 2))
                    .collect::<Vec<_>>()
                    .join(" | "),
            ));
        }
    }
    Ok(out_text)
}
