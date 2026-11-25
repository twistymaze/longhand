//! Judging across models (R16). A run's judges compare each model only
//! with its own baseline, so they cannot say how far a model is from a
//! stronger one. `cross` puts the baselines of reference models (for
//! example Haiku and gpt-6-luna) in the same judge request as the
//! summaries of another model, with no labels and in a seeded order. The
//! gap to the references then comes from one judgment, not from scores of
//! different requests.
//!
//! `cross` reads the summaries of existing runs and makes no summary.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use note_tools::storage::{NOTES_FILE, TRANSCRIPT_FILE};
use serde::{Deserialize, Serialize};
use summarize::{Client, OpenRouterRouting};

use crate::judge::{self, JudgeFile};
use crate::run::{
    AgentFile, BaselineFile, Lab, MANIFEST, Manifest, ManifestNote, RESULTS, SNAPSHOT, read_json,
    write_json,
};
use crate::run_config::model_folder;

/// Judge requests that run at the same time.
const PARALLEL: usize = 4;

/// One model of one run: `<run>/<model folder>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Pick {
    pub run: String,
    pub model: String,
}

impl Pick {
    /// `run/model folder`, or `run` alone for each model of the run.
    pub fn parse(text: &str, out: &Path) -> Result<Vec<Pick>, String> {
        match text.split_once('/') {
            Some((run, model)) => Ok(vec![Pick {
                run: run.into(),
                model: model.into(),
            }]),
            None => {
                let manifest: Manifest = read_json(&out.join(text).join(MANIFEST))?;
                Ok(manifest
                    .models
                    .iter()
                    .map(|m| Pick {
                        run: text.into(),
                        model: m.folder.clone(),
                    })
                    .collect())
            }
        }
    }

    fn label(&self) -> String {
        format!("{}/{}", self.run, self.model)
    }

    fn dir(&self, out: &Path, note_id: &str) -> PathBuf {
        out.join(&self.run)
            .join(RESULTS)
            .join(note_id)
            .join(&self.model)
    }
}

/// What `cross` judges.
pub struct Spec {
    pub name: String,
    pub references: Vec<Pick>,
    pub candidates: Vec<Pick>,
    pub judges: Vec<String>,
    pub seed: u64,
}

/// The sources and texts of one judge request.
struct Request {
    note: ManifestNote,
    snapshot: PathBuf,
    candidate: Pick,
    sources: Vec<String>,
    texts: Vec<String>,
}

pub(crate) fn summary_of(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name.starts_with("baseline") {
        serde_json::from_str::<BaselineFile>(&text).ok()?.summary
    } else {
        let agent: AgentFile = serde_json::from_str(&text).ok()?;
        agent
            .result
            .outcome
            .has_summary()
            .then_some(agent.result.summary)
            .flatten()
    }
}

fn requests(spec: &Spec, out: &Path) -> Result<Vec<Request>, String> {
    let mut requests = Vec::new();
    for candidate in &spec.candidates {
        let manifest: Manifest = read_json(&out.join(&candidate.run).join(MANIFEST))?;
        for note in &manifest.notes {
            let id = &note.note.note_id;
            let mut sources = Vec::new();
            let mut texts = Vec::new();
            for reference in &spec.references {
                if let Some(text) = summary_of(&reference.dir(out, id).join("baseline.json")) {
                    sources.push(format!("reference:{}", reference.label()));
                    texts.push(text);
                }
            }
            if sources.len() < spec.references.len() {
                continue;
            }
            let dir = candidate.dir(out, id);
            if let Some(text) = summary_of(&dir.join("baseline.json")) {
                sources.push("baseline".into());
                texts.push(text);
            }
            for run in 1..=manifest.config.runs {
                if let Some(text) = summary_of(&dir.join(format!("agent-{run}.json"))) {
                    sources.push(format!("agent-{run}"));
                    texts.push(text);
                }
            }
            if sources.len() > spec.references.len() {
                requests.push(Request {
                    note: note.clone(),
                    snapshot: out.join(&candidate.run).join(SNAPSHOT),
                    candidate: candidate.clone(),
                    sources,
                    texts,
                });
            }
        }
    }
    Ok(requests)
}

/// Judge each candidate with the references and write the judge files and
/// `report.md` to `<out>/<name>/`. A judge file that has scores is not
/// made again.
pub async fn cross(spec: &Spec, lab: &Lab<'_>) -> Result<String, String> {
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
            let path = folder
                .join(&request.note.note.note_id)
                .join(model_folder(&request.candidate.label()))
                .join(format!("judge-{}.json", model_folder(judge)));
            let done = read_json::<JudgeFile>(&path).is_ok_and(|f| f.scores.is_some());
            if !done {
                jobs.push((request, judge, client, path));
            }
        }
    }
    let results = futures_util::stream::iter(jobs)
        .map(|(request, judge, client, path)| async move {
            let id = &request.note.note.note_id;
            (lab.log)(&format!(
                "judge {judge}: {id} with {}",
                request.candidate.label()
            ));
            let read = |name: &str| {
                std::fs::read_to_string(request.snapshot.join(&request.note.note.folder).join(name))
                    .unwrap_or_default()
            };
            let seed = judge::pair_seed(spec.seed, id, &request.candidate.label(), judge);
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
    let report = report(spec, &requests, &folder);
    std::fs::write(folder.join("report.md"), &report)
        .map_err(|e| format!("cannot write the report: {e}"))?;
    Ok(report)
}

#[derive(Default)]
struct Sums {
    reference: Vec<f64>,
    baseline: Vec<f64>,
    agent: Vec<f64>,
    reference_faith: Vec<f64>,
    baseline_faith: Vec<f64>,
    agent_faith: Vec<f64>,
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn cell(value: Option<f64>) -> String {
    value.map_or("-".into(), |v| format!("{v:.1}"))
}

fn gap(reference: Option<f64>, value: Option<f64>) -> String {
    match (reference, value) {
        (Some(r), Some(v)) => format!("{:+.1}", v - r),
        _ => "-".into(),
    }
}

/// The report: for each candidate and judge, the mean overall and
/// faithfulness scores of the references, the candidate's baseline and its
/// agent summaries, from the same judge requests.
fn report(spec: &Spec, requests: &[Request], folder: &Path) -> String {
    let mut rows: BTreeMap<(String, String), Sums> = BTreeMap::new();
    for request in requests {
        for judge in &spec.judges {
            let path = folder
                .join(&request.note.note.note_id)
                .join(model_folder(&request.candidate.label()))
                .join(format!("judge-{}.json", model_folder(judge)));
            let Ok(file) = read_json::<JudgeFile>(&path) else {
                continue;
            };
            let sums = rows
                .entry((request.candidate.label(), judge.clone()))
                .or_default();
            for source in &request.sources {
                let Some(score) = file.score_of(source) else {
                    continue;
                };
                let (overall, faith) = (f64::from(score.overall), f64::from(score.faithfulness));
                let (a, b) = if source.starts_with("reference:") {
                    (&mut sums.reference, &mut sums.reference_faith)
                } else if source == "baseline" {
                    (&mut sums.baseline, &mut sums.baseline_faith)
                } else {
                    (&mut sums.agent, &mut sums.agent_faith)
                };
                a.push(overall);
                b.push(faith);
            }
        }
    }
    let references: Vec<String> = spec.references.iter().map(Pick::label).collect();
    let mut out = format!(
        "# Cross-model judging {}\n\n- References: {}\n- Judges: {}\n- Seed: {}\n\n\
         Each judge request holds the references' baselines and the summaries of one model \
         for one note, with no labels. Gap is the score less the references' mean score in \
         the same requests: a negative gap means lower than the references.\n\n\
         | Model | Judge | Reference overall | Baseline overall | Baseline gap | Agent overall | Agent gap | Reference faithfulness | Baseline faithfulness | Agent faithfulness |\n\
         |---|---|---|---|---|---|---|---|---|---|\n",
        spec.name,
        references.join(", "),
        spec.judges.join(", "),
        spec.seed
    );
    for ((model, judge), sums) in &rows {
        let reference = mean(&sums.reference);
        let baseline = mean(&sums.baseline);
        let agent = mean(&sums.agent);
        out.push_str(&format!(
            "| {model} | {judge} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            cell(reference),
            cell(baseline),
            gap(reference, baseline),
            cell(agent),
            gap(reference, agent),
            cell(mean(&sums.reference_faith)),
            cell(mean(&sums.baseline_faith)),
            cell(mean(&sums.agent_faith)),
        ));
    }
    out
}
