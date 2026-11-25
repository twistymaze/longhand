//! The summary lab: a dev harness for the summary agent (KTD12).
//!
//! ```text
//! cargo run -p summary-lab -- check-hosts \
//!     [--model <id>]... [--models <file>] [--tries 3] [--zdr]
//! cargo run -p summary-lab -- tool <name> <json> --root <storage folder> \
//!     [--max-chars 20000]
//! cargo run -p summary-lab -- run --config <file> [--resume | --dry-run] \
//!     [--out .context/summary-lab]
//! cargo run -p summary-lab -- report <run> [--out .context/summary-lab]
//! cargo run -p summary-lab -- compare <run> <run>... [--out .context/summary-lab]
//! ```
//!
//! `check-hosts` sends each model a two-turn exchange with one small tool
//! through OpenRouter, and prints whether a host runs tool calls for it.
//! Requests go only to hosts that store no request data. `--zdr` also
//! requires zero data retention. A model file has one model id per line;
//! `#` starts a comment.
//!
//! `tool` calls one note tool with a JSON input and prints its JSON output
//! (R9).
//!
//! `run` runs the baseline and the agent on a snapshot of the notes that
//! the configuration selects, for each model, and writes the results to
//! `<out>/<name>/`. `--resume` fills the missing results of an existing
//! run. `--dry-run` prints the planned calls and writes nothing. At the
//! end, the judges score the summaries, and the run writes `report.md` and
//! `report.json`.
//!
//! `report` writes the report of a run again from its files, with no
//! network. `compare` prints one table from runs with the same notes,
//! models and baselines.
//!
//! A model id that starts with `litellm:` goes to the LiteLLM server in
//! `LITELLM_BASE_URL` with `LITELLM_API_KEY`. Every other model goes to
//! OpenRouter with `OPENROUTER_API_KEY`. Each key can also come from a
//! `.env` file in the current folder or a parent folder.

use std::path::PathBuf;
use std::process::ExitCode;

use note_tools::NoteTools;
use summarize::OpenRouterRouting;
use summary_lab::config::{endpoint_for, find_value, parse_check_hosts};
use summary_lab::cross::{Pick, Spec, cross};
use summary_lab::hosts::check;
use summary_lab::report;
use summary_lab::run::{DEFAULT_OUT, Lab, Mode, run};
use summary_lab::run_config::{RunConfig, check_name};
use summary_lab::versus::{Spec as VersusSpec, versus};

const USAGE: &str = "usage:
  summary-lab check-hosts [--model <id>]... [--models <file>] [--tries 3] [--zdr]
  summary-lab tool <name> <json> --root <storage folder> [--max-chars 20000]
  summary-lab run --config <file> [--resume | --dry-run] [--out .context/summary-lab]
  summary-lab report <run> [--out .context/summary-lab]
  summary-lab compare <run> <run>... [--out .context/summary-lab]
  summary-lab cross --name <name> --reference <run>/<model folder>... \\
      --candidate <run>[/<model folder>]... [--judge <id>]... [--seed 1] [--out .context/summary-lab]
  summary-lab versus --name <name> --run <reference run> --run <run>... [--model <folder>]... \\
      [--judge <id>]... \\
      [--seed 1] [--out .context/summary-lab]";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check-hosts") => check_hosts(&args[1..]).await,
        Some("tool") => tool(&args[1..]),
        Some("run") => run_command(&args[1..]).await,
        Some("report") => report_command(&args[1..], false),
        Some("compare") => report_command(&args[1..], true),
        Some("cross") => cross_command(&args[1..]).await,
        Some("versus") => versus_command(&args[1..]).await,
        _ => Err(String::new()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if error.is_empty() {
                eprintln!("{USAGE}");
            } else {
                eprintln!("{error}");
            }
            ExitCode::FAILURE
        }
    }
}

/// `report <run>` or `compare <run> <run>...`.
fn report_command(args: &[String], compare: bool) -> Result<(), String> {
    let mut out = PathBuf::from(DEFAULT_OUT);
    let mut names = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = PathBuf::from(args.next().ok_or("--out needs a folder")?),
            other => {
                check_name(other)?;
                names.push(other);
            }
        }
    }
    let runs: Vec<PathBuf> = names.iter().map(|name| out.join(name)).collect();
    if compare {
        let folders: Vec<&std::path::Path> = runs.iter().map(PathBuf::as_path).collect();
        print!("{}", report::compare(&folders)?);
        return Ok(());
    }
    let [run] = runs.as_slice() else {
        return Err(format!("report needs one run name\n{USAGE}"));
    };
    report::write(run)?;
    println!("{}", run.join(report::REPORT_MD).display());
    Ok(())
}

fn lookup(name: &str) -> Result<String, String> {
    let start = std::env::current_dir().unwrap_or_default();
    find_value(name, std::env::var(name).ok(), &start)
}

async fn check_hosts(args: &[String]) -> Result<(), String> {
    let options = parse_check_hosts(args, |path| {
        std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
    })
    .map_err(|error| format!("{error}\n{USAGE}"))?;
    let routing = OpenRouterRouting {
        deny_data_collection: true,
        zero_retention: options.zero_retention,
    };
    println!("model\tresult\thosts");
    for model in &options.models {
        let endpoint = endpoint_for(model, lookup).map_err(|e| format!("{model}: {e}"))?;
        let result = check(model, endpoint, options.tries, routing).await;
        let hosts = if result.hosts.is_empty() {
            "-".to_string()
        } else {
            result.hosts.join(", ")
        };
        println!("{}\t{}\t{hosts}", result.model, result.verdict.text());
    }
    Ok(())
}

fn tool(args: &[String]) -> Result<(), String> {
    let mut positional = Vec::new();
    let mut root = None;
    let mut max_chars =
        summary_lab::run_config::result_chars(None, summarize::context::DEFAULT_LIMIT);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(args.next().ok_or("--root needs a folder")?)),
            "--max-chars" => {
                max_chars = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|n| *n > 0)
                    .ok_or("--max-chars needs a number from 1")?;
            }
            other => positional.push(other.to_string()),
        }
    }
    let [name, input] = positional.as_slice() else {
        return Err(format!("tool needs a name and a JSON input\n{USAGE}"));
    };
    let root = root.ok_or(format!("tool needs --root\n{USAGE}"))?;
    let input = serde_json::from_str(input).map_err(|e| format!("the input is not JSON: {e}"))?;
    let output = NoteTools::new(max_chars)
        .call(&root, name, input)
        .map_err(|error| serde_json::to_string_pretty(&error).unwrap_or(error.message))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?
    );
    Ok(())
}

async fn cross_command(args: &[String]) -> Result<(), String> {
    let mut out = PathBuf::from(DEFAULT_OUT);
    let (mut name, mut references, mut candidates, mut judges, mut seed) =
        (None, Vec::new(), Vec::new(), Vec::new(), 1);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or(format!("{arg} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--name" => name = Some(value()?),
            "--reference" => references.push(value()?),
            "--candidate" => candidates.push(value()?),
            "--judge" => judges.push(value()?),
            "--seed" => seed = value()?.parse().map_err(|_| "--seed needs a number")?,
            "--out" => out = PathBuf::from(value()?),
            other => return Err(format!("unknown argument: {other}\n{USAGE}")),
        }
    }
    let name = name.ok_or(format!("cross needs --name\n{USAGE}"))?;
    check_name(&name)?;
    let parse = |picks: Vec<String>| -> Result<Vec<Pick>, String> {
        let mut all = Vec::new();
        for pick in picks {
            all.extend(Pick::parse(&pick, &out)?);
        }
        Ok(all)
    };
    let spec = Spec {
        name,
        references: parse(references)?,
        candidates: parse(candidates)?,
        judges: if judges.is_empty() {
            summary_lab::run_config::default_judges()
        } else {
            judges
        },
        seed,
    };
    if spec.references.is_empty() || spec.candidates.is_empty() {
        return Err(format!("cross needs --reference and --candidate\n{USAGE}"));
    }
    let resolve = |model: &str| endpoint_for(model, lookup);
    let log = |line: &str| eprintln!("{line}");
    let lab = Lab {
        out,
        resolve: &resolve,
        log: &log,
    };
    print!("{}", cross(&spec, &lab).await?);
    Ok(())
}

async fn versus_command(args: &[String]) -> Result<(), String> {
    let mut out = PathBuf::from(DEFAULT_OUT);
    let (mut name, mut runs, mut judges, mut models, mut seed) =
        (None, Vec::new(), Vec::new(), Vec::new(), 1);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or(format!("{arg} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--name" => name = Some(value()?),
            "--run" => runs.push(value()?),
            "--model" => models.push(value()?),
            "--judge" => judges.push(value()?),
            "--seed" => seed = value()?.parse().map_err(|_| "--seed needs a number")?,
            "--out" => out = PathBuf::from(value()?),
            other => return Err(format!("unknown argument: {other}\n{USAGE}")),
        }
    }
    let name = name.ok_or(format!("versus needs --name\n{USAGE}"))?;
    check_name(&name)?;
    if runs.len() < 2 {
        return Err(format!("versus needs two or more --run\n{USAGE}"));
    }
    let spec = VersusSpec {
        name,
        runs,
        judges: if judges.is_empty() {
            summary_lab::run_config::default_judges()
        } else {
            judges
        },
        seed,
        models,
    };
    let resolve = |model: &str| endpoint_for(model, lookup);
    let log = |line: &str| eprintln!("{line}");
    let lab = Lab {
        out,
        resolve: &resolve,
        log: &log,
    };
    print!("{}", versus(&spec, &lab).await?);
    Ok(())
}

async fn run_command(args: &[String]) -> Result<(), String> {
    let mut config = None;
    let mut mode = Mode::New;
    let mut out = PathBuf::from(DEFAULT_OUT);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config = Some(PathBuf::from(args.next().ok_or("--config needs a file")?)),
            "--out" => out = PathBuf::from(args.next().ok_or("--out needs a folder")?),
            "--resume" if mode == Mode::New => mode = Mode::Resume,
            "--dry-run" if mode == Mode::New => mode = Mode::DryRun,
            "--resume" | "--dry-run" => return Err("use --resume or --dry-run, not both".into()),
            other => return Err(format!("unknown argument: {other}\n{USAGE}")),
        }
    }
    let config = RunConfig::read(&config.ok_or(format!("run needs --config\n{USAGE}"))?)?;
    let resolve = |model: &str| endpoint_for(model, lookup);
    let log = |line: &str| eprintln!("{line}");
    let lab = Lab {
        out,
        resolve: &resolve,
        log: &log,
    };
    let summary = run(&config, mode, &lab).await?;
    if mode == Mode::DryRun {
        return Ok(());
    }
    println!(
        "run {}: {} baselines written, {} reused, {} agent runs, {} agent tokens",
        config.name,
        summary.baselines_written,
        summary.baselines_reused,
        summary.agent_runs,
        summary.agent_tokens
    );
    match summary.stopped {
        Some(reason) => Err(format!("the run stopped: {reason}")),
        None => Ok(()),
    }
}
