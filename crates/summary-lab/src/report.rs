//! The report of a run (U7, R21, R22): `report.md` and `report.json`, per
//! note and model and per model, from the files of the run folder with no
//! network. The report gives measurements only: no verdict on a model.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::judge::JudgeFile;
use crate::run::{
    AgentFile, BaselineFile, MANIFEST, Manifest, RESULTS, read_json, read_manifest, write_json,
};
use crate::run_config::{Shape, model_folder};

pub const REPORT_JSON: &str = "report.json";
pub const REPORT_MD: &str = "report.md";

/// The KTD7 note about the baseline.
pub const BASELINE_NOTE: &str = "The baseline is the summary from the app's summary pipeline. \
It does not have the \"Recognition limitation\" line that comes from recognition data.";

/// The scores of one summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scores {
    pub coverage: u8,
    pub faithfulness: u8,
    pub overall: u8,
}

/// The results of one judge for one note and model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairJudge {
    pub judge: String,
    /// Why the judge result is missing, when it is.
    pub missing: Option<String>,
    pub baseline: Option<Scores>,
    /// The scores of each judged agent run, by run number.
    pub agents: BTreeMap<usize, Scores>,
    pub mean_agent_overall: Option<f64>,
    /// The lowest and the highest overall score of the agent runs.
    pub spread: Option<(u8, u8)>,
    /// The baseline's overall score less the mean agent overall score.
    pub mean_loss: Option<f64>,
}

/// The means of the agent runs of one note and model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Means {
    pub model_calls: f64,
    pub tool_calls: f64,
    pub input_tokens: f64,
    pub output_tokens: f64,
    pub cost: f64,
    pub seconds: f64,
}

/// One note and model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairReport {
    pub note_id: String,
    pub model: String,
    pub baseline_key: Option<String>,
    pub baseline_error: Option<String>,
    pub baseline_reused: bool,
    pub agent_runs: usize,
    /// The number of agent runs with each outcome.
    pub outcomes: BTreeMap<String, usize>,
    /// The part of the agent runs with no summary.
    pub failure_rate: Option<f64>,
    pub means: Means,
    pub hosts: Vec<String>,
    /// True when the agent turns went to more than one host.
    pub host_change: bool,
    pub judges: Vec<PairJudge>,
}

/// The results of one judge over the notes of one model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelJudge {
    pub judge: String,
    pub judged_notes: usize,
    pub missing_notes: usize,
    pub mean_baseline_overall: Option<f64>,
    pub mean_agent_overall: Option<f64>,
    pub mean_loss: Option<f64>,
}

/// One model over the notes of the run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelReport {
    pub model: String,
    pub context_tokens: u32,
    pub budget_source: String,
    pub agent_runs: usize,
    pub outcomes: BTreeMap<String, usize>,
    pub failure_rate: Option<f64>,
    pub means: Means,
    pub hosts: Vec<String>,
    pub host_change: bool,
    pub judges: Vec<ModelJudge>,
}

/// The report of a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub run: String,
    pub shape: Shape,
    pub strategy: Option<String>,
    pub prompt: Option<String>,
    pub runs_per_note: usize,
    pub judges: Vec<String>,
    pub seed: u64,
    pub notes: Vec<String>,
    pub skipped: usize,
    pub stopped: Option<String>,
    pub baseline_note: String,
    pub models: Vec<ModelReport>,
    pub pairs: Vec<PairReport>,
}

/// Build the report of the run folder `folder` from its files.
pub fn build(folder: &Path) -> Result<Report, String> {
    let manifest = read_manifest(&folder.join(MANIFEST))?;
    let mut pairs = Vec::new();
    for note in &manifest.notes {
        for model in &manifest.models {
            let dir = folder
                .join(RESULTS)
                .join(&note.note.note_id)
                .join(&model.folder);
            pairs.push(pair(&manifest, &dir, &note.note.note_id, &model.model)?);
        }
    }
    let models = manifest
        .models
        .iter()
        .map(|model| {
            let mine: Vec<&PairReport> = pairs.iter().filter(|p| p.model == model.model).collect();
            model_report(&manifest, model, &mine)
        })
        .collect();
    Ok(Report {
        run: manifest.config.name.clone(),
        shape: manifest.config.shape,
        strategy: manifest.config.strategy.clone(),
        prompt: manifest.config.prompt.clone(),
        runs_per_note: manifest.config.runs,
        judges: manifest.config.judges.clone(),
        seed: manifest.seed,
        notes: manifest
            .notes
            .iter()
            .map(|n| n.note.note_id.clone())
            .collect(),
        skipped: manifest.skipped.len(),
        stopped: manifest.stopped.clone(),
        baseline_note: BASELINE_NOTE.into(),
        models,
        pairs,
    })
}

/// Build the report and write `report.json` and `report.md`.
pub fn write(folder: &Path) -> Result<Report, String> {
    let report = build(folder)?;
    write_json(&folder.join(REPORT_JSON), &report)?;
    std::fs::write(folder.join(REPORT_MD), markdown(&report))
        .map_err(|e| format!("cannot write the report: {e}"))?;
    Ok(report)
}

fn mean(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let values: Vec<f64> = values.into_iter().collect();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn pair(manifest: &Manifest, dir: &Path, note_id: &str, model: &str) -> Result<PairReport, String> {
    let baseline: Option<BaselineFile> = {
        let path = dir.join("baseline.json");
        path.exists().then(|| read_json(&path)).transpose()?
    };
    let mut agents: Vec<AgentFile> = Vec::new();
    for run in 1..=manifest.config.runs {
        let path = dir.join(format!("agent-{run}.json"));
        if path.exists() {
            agents.push(read_json(&path)?);
        }
    }
    let mut outcomes = BTreeMap::new();
    let mut hosts = BTreeSet::new();
    for agent in &agents {
        let outcome = serde_json::to_value(agent.result.outcome)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        *outcomes.entry(outcome).or_insert(0) += 1;
        hosts.extend(agent.result.turns.iter().filter_map(|t| t.host.clone()));
    }
    let failures = agents
        .iter()
        .filter(|a| !a.result.outcome.has_summary())
        .count();
    let count = agents.len().max(1) as f64;
    let means = Means {
        model_calls: agents
            .iter()
            .map(|a| a.result.totals.model_calls as f64)
            .sum::<f64>()
            / count,
        tool_calls: agents
            .iter()
            .map(|a| a.result.totals.tool_calls as f64)
            .sum::<f64>()
            / count,
        input_tokens: agents
            .iter()
            .map(|a| a.result.totals.input_tokens as f64)
            .sum::<f64>()
            / count,
        output_tokens: agents
            .iter()
            .map(|a| a.result.totals.output_tokens as f64)
            .sum::<f64>()
            / count,
        cost: agents.iter().map(|a| a.result.totals.cost).sum::<f64>() / count,
        seconds: agents
            .iter()
            .map(|a| a.result.totals.elapsed_ms as f64)
            .sum::<f64>()
            / count
            / 1000.0,
    };
    let judges = manifest
        .config
        .judges
        .iter()
        .map(|judge| pair_judge(dir, judge, &agents))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PairReport {
        note_id: note_id.to_string(),
        model: model.to_string(),
        baseline_key: baseline.as_ref().map(|b| b.key.clone()),
        baseline_error: match &baseline {
            Some(b) => b.error.clone(),
            None => Some("no baseline file".into()),
        },
        baseline_reused: baseline.as_ref().is_some_and(|b| b.reused),
        agent_runs: agents.len(),
        outcomes,
        failure_rate: (!agents.is_empty()).then(|| failures as f64 / agents.len() as f64),
        means,
        host_change: hosts.len() > 1,
        hosts: hosts.into_iter().collect(),
        judges,
    })
}

fn pair_judge(dir: &Path, judge: &str, agents: &[AgentFile]) -> Result<PairJudge, String> {
    let path = dir.join(format!("judge-{}.json", model_folder(judge)));
    let missing = |reason: String| PairJudge {
        judge: judge.to_string(),
        missing: Some(reason),
        baseline: None,
        agents: BTreeMap::new(),
        mean_agent_overall: None,
        spread: None,
        mean_loss: None,
    };
    if !path.exists() {
        return Ok(missing("no judge file".into()));
    }
    let file: JudgeFile = read_json(&path)?;
    if file.scores.is_none() {
        return Ok(missing(
            file.error.unwrap_or_else(|| "no valid reply".into()),
        ));
    }
    let scores = |source: &str| {
        file.score_of(source).map(|s| Scores {
            coverage: s.coverage,
            faithfulness: s.faithfulness,
            overall: s.overall,
        })
    };
    let baseline = scores("baseline");
    let agent_scores: BTreeMap<usize, Scores> = agents
        .iter()
        .filter_map(|a| scores(&format!("agent-{}", a.run)).map(|s| (a.run, s)))
        .collect();
    let overall: Vec<u8> = agent_scores.values().map(|s| s.overall).collect();
    let mean_agent_overall = mean(overall.iter().map(|o| f64::from(*o)));
    // With more than one baseline, the loss uses their mean.
    let mean_baseline = mean(
        file.order
            .iter()
            .filter(|c| c.source.starts_with("baseline"))
            .filter_map(|c| file.score_of(&c.source))
            .map(|s| f64::from(s.overall)),
    );
    Ok(PairJudge {
        judge: judge.to_string(),
        missing: None,
        mean_loss: mean_baseline.zip(mean_agent_overall).map(|(b, a)| b - a),
        spread: overall
            .iter()
            .min()
            .copied()
            .zip(overall.iter().max().copied()),
        baseline,
        agents: agent_scores,
        mean_agent_overall,
    })
}

fn model_report(
    manifest: &Manifest,
    model: &crate::run::ManifestModel,
    pairs: &[&PairReport],
) -> ModelReport {
    let agent_runs: usize = pairs.iter().map(|p| p.agent_runs).sum();
    let mut outcomes = BTreeMap::new();
    for p in pairs {
        for (outcome, n) in &p.outcomes {
            *outcomes.entry(outcome.clone()).or_insert(0) += n;
        }
    }
    let failures: usize = pairs
        .iter()
        .map(|p| (p.failure_rate.unwrap_or(0.0) * p.agent_runs as f64).round() as usize)
        .sum();
    // The means over the runs: each note counts by its number of runs.
    let weighted = |f: fn(&Means) -> f64| {
        if agent_runs == 0 {
            0.0
        } else {
            pairs
                .iter()
                .map(|p| f(&p.means) * p.agent_runs as f64)
                .sum::<f64>()
                / agent_runs as f64
        }
    };
    let hosts: BTreeSet<String> = pairs.iter().flat_map(|p| p.hosts.clone()).collect();
    let judges = manifest
        .config
        .judges
        .iter()
        .map(|judge| {
            let results: Vec<&PairJudge> = pairs
                .iter()
                .filter_map(|p| p.judges.iter().find(|j| &j.judge == judge))
                .collect();
            let judged: Vec<&&PairJudge> = results.iter().filter(|j| j.missing.is_none()).collect();
            ModelJudge {
                judge: judge.clone(),
                judged_notes: judged.len(),
                missing_notes: results.len() - judged.len(),
                mean_baseline_overall: mean(
                    judged
                        .iter()
                        .filter_map(|j| j.baseline.map(|b| f64::from(b.overall))),
                ),
                mean_agent_overall: mean(judged.iter().filter_map(|j| j.mean_agent_overall)),
                mean_loss: mean(judged.iter().filter_map(|j| j.mean_loss)),
            }
        })
        .collect();
    ModelReport {
        model: model.model.clone(),
        context_tokens: model.context_tokens,
        budget_source: model.budget_source.clone(),
        agent_runs,
        outcomes,
        failure_rate: (agent_runs > 0).then(|| failures as f64 / agent_runs as f64),
        means: Means {
            model_calls: weighted(|m| m.model_calls),
            tool_calls: weighted(|m| m.tool_calls),
            input_tokens: weighted(|m| m.input_tokens),
            output_tokens: weighted(|m| m.output_tokens),
            cost: weighted(|m| m.cost),
            seconds: weighted(|m| m.seconds),
        },
        host_change: hosts.len() > 1,
        hosts: hosts.into_iter().collect(),
        judges,
    }
}

fn number(value: Option<f64>) -> String {
    value.map_or("missing".into(), |v| format!("{v:.2}"))
}

fn percent(value: Option<f64>) -> String {
    value.map_or("-".into(), |v| format!("{:.0}%", v * 100.0))
}

fn outcome_text(outcomes: &BTreeMap<String, usize>) -> String {
    if outcomes.is_empty() {
        return "-".into();
    }
    outcomes
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The Markdown text of a report.
pub fn markdown(report: &Report) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# Summary lab run {}\n", report.run);
    let _ = writeln!(
        md,
        "- Shape: {:?}\n- Agent runs per note and model: {}\n- Notes: {} ({} skipped)\n- Judges: {}\n- Seed: {}\n- Strategy: {}\n- Prompt: {}",
        report.shape,
        report.runs_per_note,
        report.notes.len(),
        report.skipped,
        if report.judges.is_empty() {
            "none".into()
        } else {
            report.judges.join(", ")
        },
        report.seed,
        report
            .strategy
            .as_deref()
            .map_or("default", |_| "set in the configuration"),
        report
            .prompt
            .as_deref()
            .unwrap_or("the prompt of each note's type"),
    );
    if let Some(stopped) = &report.stopped {
        let _ = writeln!(md, "- The run stopped: {stopped}");
    }
    let _ = writeln!(md, "\n{}\n", report.baseline_note);
    let _ = writeln!(
        md,
        "Scores are from 1 to 10. Loss is the baseline's overall score less the mean overall score of the agent runs: a positive loss means the agent summaries got lower scores.\n"
    );

    let _ = writeln!(md, "## Models\n");
    let _ = writeln!(
        md,
        "| Model | Budget | Agent runs | No summary | Outcomes | Tool calls | Input tokens | Output tokens | Cost | Seconds | Hosts |"
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|---|---|---|---|");
    for m in &report.models {
        let _ = writeln!(
            md,
            "| {} | {} ({}) | {} | {} | {} | {:.1} | {:.0} | {:.0} | {:.4} | {:.1} | {}{} |",
            m.model,
            m.context_tokens,
            m.budget_source,
            m.agent_runs,
            percent(m.failure_rate),
            outcome_text(&m.outcomes),
            m.means.tool_calls,
            m.means.input_tokens,
            m.means.output_tokens,
            m.means.cost,
            m.means.seconds,
            m.hosts.join(", "),
            if m.host_change {
                " (more than one host)"
            } else {
                ""
            },
        );
    }
    if !report.judges.is_empty() {
        let _ = writeln!(
            md,
            "\n| Model | Judge | Judged notes | Missing | Baseline overall | Agent overall | Loss |"
        );
        let _ = writeln!(md, "|---|---|---|---|---|---|---|");
        for m in &report.models {
            for j in &m.judges {
                let _ = writeln!(
                    md,
                    "| {} | {} | {} | {} | {} | {} | {} |",
                    m.model,
                    j.judge,
                    j.judged_notes,
                    j.missing_notes,
                    number(j.mean_baseline_overall),
                    number(j.mean_agent_overall),
                    number(j.mean_loss),
                );
            }
        }
    }

    let _ = writeln!(md, "\n## Notes\n");
    for p in &report.pairs {
        let _ = writeln!(md, "### {} with {}\n", p.note_id, p.model);
        let _ = writeln!(
            md,
            "- Baseline: {}{}",
            match &p.baseline_error {
                Some(error) => format!("no summary ({error})"),
                None => "summary".into(),
            },
            if p.baseline_reused {
                ", from an earlier run"
            } else {
                ""
            },
        );
        let _ = writeln!(
            md,
            "- Agent runs: {}, no summary: {}, outcomes: {}",
            p.agent_runs,
            percent(p.failure_rate),
            outcome_text(&p.outcomes)
        );
        let _ = writeln!(
            md,
            "- Means: {:.1} model calls, {:.1} tool calls, {:.0} input and {:.0} output tokens, cost {:.4}, {:.1} s",
            p.means.model_calls,
            p.means.tool_calls,
            p.means.input_tokens,
            p.means.output_tokens,
            p.means.cost,
            p.means.seconds
        );
        let _ = writeln!(
            md,
            "- Hosts: {}{}",
            if p.hosts.is_empty() {
                "-".into()
            } else {
                p.hosts.join(", ")
            },
            if p.host_change {
                " (more than one host)"
            } else {
                ""
            }
        );
        for j in &p.judges {
            match &j.missing {
                Some(reason) => {
                    let _ = writeln!(md, "- {}: missing ({reason})", j.judge);
                }
                None => {
                    let runs: Vec<String> = j
                        .agents
                        .iter()
                        .map(|(run, s)| {
                            format!("run {run} {}/{}/{}", s.coverage, s.faithfulness, s.overall)
                        })
                        .collect();
                    let _ = writeln!(
                        md,
                        "- {}: baseline {}, agents {} (coverage/faithfulness/overall), spread {}, loss {}",
                        j.judge,
                        j.baseline.map_or("missing".into(), |b| format!(
                            "{}/{}/{}",
                            b.coverage, b.faithfulness, b.overall
                        )),
                        if runs.is_empty() {
                            "-".into()
                        } else {
                            runs.join(", ")
                        },
                        j.spread
                            .map_or("-".into(), |(lo, hi)| format!("{lo} to {hi}")),
                        number(j.mean_loss),
                    );
                }
            }
        }
        let _ = writeln!(md);
    }
    md
}

/// One table from the reports of runs with the same notes, models and
/// baselines: one row per run, model and judge.
pub fn compare(folders: &[&Path]) -> Result<String, String> {
    if folders.len() < 2 {
        return Err("compare needs two runs or more".into());
    }
    let reports = folders
        .iter()
        .map(|f| build(f))
        .collect::<Result<Vec<_>, _>>()?;
    let first = &reports[0];
    let models = |r: &Report| -> Vec<String> { r.models.iter().map(|m| m.model.clone()).collect() };
    let baselines = |r: &Report| -> BTreeMap<(String, String), Option<String>> {
        r.pairs
            .iter()
            .map(|p| ((p.note_id.clone(), p.model.clone()), p.baseline_key.clone()))
            .collect()
    };
    for other in &reports[1..] {
        let mut a = first.notes.clone();
        let mut b = other.notes.clone();
        a.sort();
        b.sort();
        if a != b {
            return Err(format!(
                "the runs {} and {} have different notes",
                first.run, other.run
            ));
        }
        if models(first) != models(other) {
            return Err(format!(
                "the runs {} and {} have different models",
                first.run, other.run
            ));
        }
        if baselines(first) != baselines(other) {
            return Err(format!(
                "the runs {} and {} have different baselines",
                first.run, other.run
            ));
        }
    }
    let mut md = String::new();
    let _ = writeln!(
        md,
        "| Run | Shape | Model | Judge | Agent overall | Loss | No summary | Tool calls | Tokens | Cost | Seconds |"
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|---|---|---|---|");
    for report in &reports {
        for m in &report.models {
            let judges: Vec<Option<&ModelJudge>> = if m.judges.is_empty() {
                vec![None]
            } else {
                m.judges.iter().map(Some).collect()
            };
            for j in judges {
                let _ = writeln!(
                    md,
                    "| {} | {:?} | {} | {} | {} | {} | {} | {:.1} | {:.0} | {:.4} | {:.1} |",
                    report.run,
                    report.shape,
                    m.model,
                    j.map_or("-", |j| j.judge.as_str()),
                    number(j.and_then(|j| j.mean_agent_overall)),
                    number(j.and_then(|j| j.mean_loss)),
                    percent(m.failure_rate),
                    m.means.tool_calls,
                    m.means.input_tokens + m.means.output_tokens,
                    m.means.cost,
                    m.means.seconds,
                );
            }
        }
    }
    Ok(md)
}
