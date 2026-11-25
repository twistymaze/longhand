//! Harness runs on the fixture storage folder with a scripted endpoint
//! (U6, U7).

#[path = "../../summarize/tests/common/mod.rs"]
mod common;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{Mock, Reply, serve};
use serde_json::{Value, json};
use summarize::{Endpoint, Provider};
use summary_agent::Outcome;
use summary_lab::judge::JudgeFile;
use summary_lab::report::{self, Report};
use summary_lab::run::{AgentFile, BaselineFile, Lab, Mode, RunSummary, read_manifest, run};
use summary_lab::run_config::{ModelEntry, NotesFilter, RunConfig, Shape};

const KEY: &str = "k-secret-test-key";
const MODEL: &str = "test/model-a";

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

struct Setup {
    storage: tempfile::TempDir,
    out: tempfile::TempDir,
    mock: Mock,
}

/// The text of all the messages of a chat request.
fn request_text(request: &Value) -> String {
    let mut text = String::new();
    for message in request["messages"].as_array().into_iter().flatten() {
        match &message["content"] {
            Value::String(s) => text.push_str(s),
            Value::Array(parts) => {
                for part in parts {
                    text.push_str(part["text"].as_str().unwrap_or_default());
                }
            }
            _ => {}
        }
        text.push('\n');
    }
    text
}

/// The reply of a scripted judge: 9 for the summary with the baseline's
/// text, 6 for each other summary.
fn judge_reply(request: &Value) -> String {
    let text = request_text(request);
    let scores: Vec<Value> = text
        .split("<summary label=\"")
        .skip(1)
        .map(|rest| {
            let (label, body) = rest.split_once("\">").unwrap();
            let score = if body.contains("A point of the meeting") {
                9
            } else {
                6
            };
            json!({"label": label, "coverage": score, "faithfulness": score, "overall": score})
        })
        .collect();
    json!({ "scores": scores }).to_string()
}

/// A reply of the scripted endpoint: agent turns (requests with tools)
/// get a summary with usage, other requests a short text. `judge/...`
/// models answer as judges, and `judge/broken` with no JSON. The model
/// `test/no-summary` gives agent turns no text.
async fn setup() -> Setup {
    let storage = tempfile::tempdir().unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/storage-folder");
    copy_dir(&fixture, storage.path());
    let responder: common::Responder = Arc::new(|body: &str| {
        let request: Value = serde_json::from_str(body).unwrap_or_default();
        let model = request["model"].as_str().unwrap_or_default();
        let judged;
        let text = if model == "judge/broken" {
            "I cannot answer in JSON."
        } else if model.starts_with("judge/") {
            judged = judge_reply(&request);
            judged.as_str()
        } else if request.get("tools").is_some() {
            if model == "test/no-summary" {
                ""
            } else {
                "The agent summary."
            }
        } else {
            "- [00:00:01] A point of the meeting."
        };
        let reply = json!({
            "provider": "HostA",
            "choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120, "cost": 0.001}
        });
        (200, reply.to_string())
    });
    let mock = serve(HashMap::from([(
        "/v1/chat/completions",
        Reply::With(responder),
    )]))
    .await;
    Setup {
        storage,
        out: tempfile::tempdir().unwrap(),
        mock,
    }
}

fn config(setup: &Setup, name: &str) -> RunConfig {
    RunConfig::parse(
        &json!({
            "name": name,
            "storage": setup.storage.path(),
            "notes": {"ids": [
                "note-budget-review", "note-design-review", "note-hiring-sync",
                "note-standup", "note-vendor-call", "note-quick-idea"
            ]},
            "models": [{"model": MODEL, "context_tokens": 32000}],
            "judges": [],
        })
        .to_string(),
    )
    .unwrap()
}

async fn start(setup: &Setup, config: &RunConfig, mode: Mode) -> Result<RunSummary, String> {
    let base = setup.mock.base.clone();
    let resolve = move |model: &str| {
        Ok(Endpoint {
            provider: Provider::OpenRouter,
            base_url: base.clone(),
            model: model.to_string(),
            credential: Some(KEY.to_string()),
        })
    };
    let log = |_: &str| {};
    let lab = Lab {
        out: setup.out.path().to_path_buf(),
        resolve: &resolve,
        log: &log,
    };
    run(config, mode, &lab).await
}

/// Every file under `root` with its bytes.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let entry = entry.unwrap();
            let kind = fs::symlink_metadata(entry.path()).unwrap().file_type();
            if kind.is_dir() {
                folders.push(entry.path());
            } else if kind.is_file() {
                let relative = entry.path().strip_prefix(root).unwrap().to_path_buf();
                files.insert(relative, fs::read(entry.path()).unwrap());
            }
        }
    }
    files
}

fn results(setup: &Setup, run: &str, note: &str) -> PathBuf {
    setup
        .out
        .path()
        .join(run)
        .join("results")
        .join(note)
        .join("test_model-a")
}

fn baseline(setup: &Setup, run: &str, note: &str) -> BaselineFile {
    let text = fs::read_to_string(results(setup, run, note).join("baseline.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn agent_files(setup: &Setup, run: &str, note: &str) -> Vec<AgentFile> {
    let mut files: Vec<AgentFile> = fs::read_dir(results(setup, run, note))
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            let name = path.file_name()?.to_string_lossy().to_string();
            name.starts_with("agent-")
                .then(|| serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap())
        })
        .collect();
    files.sort_by_key(|f| f.run);
    files
}

const USED: [&str; 3] = [
    "note-budget-review",
    "note-design-review",
    "note-hiring-sync",
];

#[tokio::test]
async fn a_run_writes_a_manifest_one_baseline_and_three_agent_results_per_note() {
    let setup = setup().await;
    let before = tree(setup.storage.path());
    let summary = start(&setup, &config(&setup, "first"), Mode::New)
        .await
        .unwrap();
    assert_eq!(summary.baselines_written, 3);
    assert_eq!(summary.agent_runs, 9);
    assert_eq!(summary.agent_tokens, 9 * 120);

    let manifest = read_manifest(&setup.out.path().join("first/manifest.json")).unwrap();
    let ids: Vec<&str> = manifest
        .notes
        .iter()
        .map(|n| n.note.note_id.as_str())
        .collect();
    assert_eq!(ids, USED);
    assert_eq!(manifest.models[0].context_tokens, 32_000);
    assert_eq!(manifest.models[0].budget_source, "set");
    let reason = |id: &str| {
        manifest
            .skipped
            .iter()
            .find(|s| s.note == id)
            .map(|s| s.reason.clone())
            .unwrap_or_default()
    };
    assert!(reason("note-standup").starts_with("busy"));
    assert!(reason("note-vendor-call").starts_with("busy"));
    assert!(reason("note-quick-idea").starts_with("no transcript"));

    for note in USED {
        let base = baseline(&setup, "first", note);
        assert!(!base.reused);
        assert!(base.summary.unwrap().contains("A point of the meeting"));
        let agents = agent_files(&setup, "first", note);
        assert_eq!(agents.len(), 3);
        assert!(agents.iter().all(|a| a.shape == Shape::Single));
        assert!(agents.iter().all(|a| a.result.outcome == Outcome::NoReads));
    }

    assert_eq!(tree(setup.storage.path()), before);
    for (path, bytes) in tree(setup.out.path()) {
        assert!(
            !String::from_utf8_lossy(&bytes).contains(KEY),
            "{} holds the key",
            path.display()
        );
    }
    // Every request, also the baseline requests, denies data collection.
    let seen = setup.mock.seen.lock().unwrap();
    let baselines: Vec<Value> = seen
        .iter()
        .filter(|s| s.method == "POST")
        .map(|s| s.json())
        .filter(|body| body.get("tools").is_none())
        .collect();
    assert!(!baselines.is_empty());
    assert!(
        seen.iter()
            .filter(|s| s.method == "POST")
            .all(|s| s.json()["provider"]["data_collection"] == "deny")
    );
}

#[tokio::test]
async fn a_name_is_used_once_and_a_new_run_reuses_the_baseline_of_an_unchanged_note() {
    let setup = setup().await;
    start(&setup, &config(&setup, "first"), Mode::New)
        .await
        .unwrap();
    let error = start(&setup, &config(&setup, "first"), Mode::New)
        .await
        .unwrap_err();
    assert!(error.contains("exists"), "{error}");

    // The transcript of one note changes in the source.
    let transcript = setup
        .storage
        .path()
        .join("2026-10-03 1000 Design review/transcript.md");
    let text = fs::read_to_string(&transcript).unwrap();
    fs::write(
        &transcript,
        format!("{text}\n[00:00:40] Speaker 1: one more line\n"),
    )
    .unwrap();
    // A change to the agent's reading strategy does not change a baseline.
    let mut second = config(&setup, "second");
    second.strategy = Some("Read only the first page.".into());
    let summary = start(&setup, &second, Mode::New).await.unwrap();
    assert_eq!(summary.baselines_reused, 2);
    assert_eq!(summary.baselines_written, 1);
    assert!(baseline(&setup, "second", "note-budget-review").reused);
    assert!(!baseline(&setup, "second", "note-design-review").reused);
}

#[tokio::test]
async fn resume_fills_only_the_missing_results_and_refuses_another_configuration() {
    let setup = setup().await;
    let mut capped = config(&setup, "capped");
    capped.token_cap = Some(100);
    let summary = start(&setup, &capped, Mode::New).await.unwrap();
    assert_eq!(summary.agent_runs, 1);
    assert!(summary.stopped.unwrap().contains("token cap"));
    let first = fs::read(results(&setup, "capped", USED[0]).join("agent-1.json")).unwrap();

    let mut other = capped.clone();
    other.models.push(ModelEntry::Id("test/model-b".into()));
    let error = start(&setup, &other, Mode::Resume).await.unwrap_err();
    assert!(error.contains("not the configuration"), "{error}");

    let mut raised = capped.clone();
    raised.token_cap = None;
    let summary = start(&setup, &raised, Mode::Resume).await.unwrap();
    assert_eq!(summary.agent_runs, 8);
    assert_eq!(summary.baselines_written + summary.baselines_reused, 2);
    for note in USED {
        assert_eq!(agent_files(&setup, "capped", note).len(), 3);
    }
    // The result of the first start is not written again.
    assert_eq!(
        fs::read(results(&setup, "capped", USED[0]).join("agent-1.json")).unwrap(),
        first
    );
    let manifest = read_manifest(&setup.out.path().join("capped/manifest.json")).unwrap();
    assert!(manifest.stopped.is_none());
}

#[tokio::test]
async fn resume_makes_a_transport_failure_and_its_judge_result_again() {
    let setup = setup().await;
    let config = judged(&setup, "redo", &["judge/a"]);
    start(&setup, &config, Mode::New).await.unwrap();
    let dir = results(&setup, "redo", USED[0]);
    let agent = dir.join("agent-2.json");
    let mut file: Value = serde_json::from_str(&fs::read_to_string(&agent).unwrap()).unwrap();
    file["result"]["outcome"] = json!("transport");
    fs::write(&agent, file.to_string()).unwrap();
    let judge = dir.join("judge-judge_a.json");
    let mut verdict: Value = serde_json::from_str(&fs::read_to_string(&judge).unwrap()).unwrap();
    verdict["scores"] = Value::Null;
    fs::write(&judge, verdict.to_string()).unwrap();
    let kept = fs::read(dir.join("agent-1.json")).unwrap();

    let summary = start(&setup, &config, Mode::Resume).await.unwrap();
    assert_eq!(summary.agent_runs, 1);
    assert_eq!(summary.judge_files, 1);
    let again = agent_files(&setup, "redo", USED[0]);
    assert_eq!(again[1].result.outcome, Outcome::NoReads);
    assert_eq!(fs::read(dir.join("agent-1.json")).unwrap(), kept);
    let verdict: JudgeFile = serde_json::from_str(&fs::read_to_string(&judge).unwrap()).unwrap();
    assert!(verdict.scores.is_some());
}

#[tokio::test]
async fn a_dry_run_writes_no_file_and_sends_no_request() {
    let setup = setup().await;
    start(&setup, &config(&setup, "plan"), Mode::DryRun)
        .await
        .unwrap();
    assert!(fs::read_dir(setup.out.path()).unwrap().next().is_none());
    assert!(setup.mock.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_new_run_that_fails_before_its_manifest_leaves_no_folder() {
    let setup = setup().await;
    let mut bad = config(&setup, "bad");
    bad.prompt = Some("no-such-prompt".into());
    let error = start(&setup, &bad, Mode::New).await.unwrap_err();
    assert!(error.contains("no-such-prompt"), "{error}");
    assert!(!setup.out.path().join("bad").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_note_file_that_is_a_symlink_is_not_in_the_snapshot() {
    let setup = setup().await;
    let outside = setup.out.path().join("outside.md");
    fs::write(&outside, "private text").unwrap();
    let note = setup.storage.path().join("2026-10-01 0900 Budget review");
    std::os::unix::fs::symlink(&outside, note.join("extra.md")).unwrap();
    let mut only = config(&setup, "links");
    only.notes = NotesFilter {
        ids: vec!["note-budget-review".into()],
        list: None,
    };
    only.runs = 1;
    start(&setup, &only, Mode::New).await.unwrap();
    let copy = setup
        .out
        .path()
        .join("links/snapshot/2026-10-01 0900 Budget review");
    assert!(copy.join("transcript.md").exists());
    assert!(!copy.join("extra.md").exists());
}

#[tokio::test]
async fn each_agent_shape_writes_three_results_per_note_and_the_manifest_records_it() {
    for shape in [Shape::Single, Shape::Readers, Shape::Steps] {
        let setup = setup().await;
        let mut shaped = config(&setup, "shape");
        shaped.shape = shape;
        start(&setup, &shaped, Mode::New).await.unwrap();
        let manifest = read_manifest(&setup.out.path().join("shape/manifest.json")).unwrap();
        assert_eq!(manifest.config.shape, shape);
        for note in USED {
            let agents = agent_files(&setup, "shape", note);
            assert_eq!(agents.len(), 3, "{shape:?}");
            assert!(agents.iter().all(|a| a.shape == shape));
            assert!(agents.iter().all(|a| a.result.outcome.has_summary()));
        }
    }
}

#[test]
fn the_tool_command_prints_the_json_of_the_library_call() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/storage-folder");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_summary-lab"))
        .args(["tool", "list_notes", r#"{"tags": ["budget"]}"#, "--root"])
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(output.status.success());
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    let direct = note_tools::NoteTools::new(20_000)
        .call(&fixture, "list_notes", json!({"tags": ["budget"]}))
        .unwrap();
    assert_eq!(printed, direct);
}

fn judged(setup: &Setup, name: &str, judges: &[&str]) -> RunConfig {
    let mut config = config(setup, name);
    config.judges = judges.iter().map(|j| j.to_string()).collect();
    config.seed = Some(7);
    config
}

fn report_of(setup: &Setup, name: &str) -> Report {
    let text = fs::read_to_string(setup.out.path().join(name).join("report.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn the_judges_score_unlabelled_summaries_and_the_report_maps_the_scores_back() {
    let setup = setup().await;
    start(
        &setup,
        &judged(&setup, "judged", &["judge/a", "judge/b"]),
        Mode::New,
    )
    .await
    .unwrap();
    let report = report_of(&setup, "judged");
    assert_eq!(report.pairs.len(), 3);
    for pair in &report.pairs {
        assert_eq!(pair.failure_rate, Some(0.0));
        assert_eq!(pair.hosts, ["HostA"]);
        for judge in &pair.judges {
            assert!(judge.missing.is_none(), "{judge:?}");
            assert_eq!(judge.baseline.unwrap().overall, 9);
            assert_eq!(judge.agents.len(), 3);
            assert!(judge.agents.values().all(|s| s.overall == 6));
            assert_eq!(judge.spread, Some((6, 6)));
            assert_eq!(judge.mean_loss, Some(3.0));
        }
    }
    let model = &report.models[0];
    assert!(
        model
            .judges
            .iter()
            .all(|j| j.judged_notes == 3 && j.mean_loss == Some(3.0))
    );

    // The judge requests carry the transcript, and no text that names
    // the baseline.
    let requests: Vec<Value> = setup
        .mock
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.method == "POST")
        .map(|s| s.json())
        .filter(|b| {
            b["model"]
                .as_str()
                .unwrap_or_default()
                .starts_with("judge/")
        })
        .collect();
    assert_eq!(requests.len(), 6);
    for request in &requests {
        let text = request_text(request);
        assert!(text.contains("<transcript>") && text.contains("untrusted data"));
        let data = text.split_once("<transcript>").unwrap().1;
        assert!(!data.to_lowercase().contains("baseline"));
        assert_eq!(request["provider"]["data_collection"], "deny");
    }

    // The same seed gives the same order in a second run.
    let order = |run: &str| {
        let path = results(&setup, run, USED[0]).join("judge-judge_a.json");
        let file: JudgeFile = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        file.order
    };
    start(&setup, &judged(&setup, "again", &["judge/a"]), Mode::New)
        .await
        .unwrap();
    assert_eq!(order("judged"), order("again"));
}

#[tokio::test]
async fn a_run_with_no_summary_is_not_judged_and_counts_as_no_summary() {
    let setup = setup().await;
    let mut config = judged(&setup, "empty", &["judge/a"]);
    config.models = vec![ModelEntry::Full {
        model: "test/no-summary".into(),
        context_tokens: Some(32_000),
        output_tokens: None,
        thinking: None,
    }];
    start(&setup, &config, Mode::New).await.unwrap();
    let report = report_of(&setup, "empty");
    let pair = &report.pairs[0];
    assert_eq!(pair.failure_rate, Some(1.0));
    assert_eq!(pair.outcomes.get("no_summary"), Some(&3));
    let judge = &pair.judges[0];
    assert!(judge.agents.is_empty());
    assert_eq!(judge.baseline.unwrap().overall, 9);
    assert_eq!(judge.mean_loss, None);
    assert_eq!(report.models[0].failure_rate, Some(1.0));
}

#[tokio::test]
async fn a_judge_with_no_valid_reply_is_missing_after_one_more_try_and_is_not_a_zero() {
    let setup = setup().await;
    start(
        &setup,
        &judged(&setup, "broken", &["judge/a", "judge/broken"]),
        Mode::New,
    )
    .await
    .unwrap();
    let broken_requests = setup
        .mock
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.method == "POST" && s.json()["model"] == "judge/broken")
        .count();
    assert_eq!(broken_requests, 3 * 2);
    let report = report_of(&setup, "broken");
    let model = &report.models[0];
    let broken = model
        .judges
        .iter()
        .find(|j| j.judge == "judge/broken")
        .unwrap();
    assert_eq!((broken.judged_notes, broken.missing_notes), (0, 3));
    assert_eq!(broken.mean_loss, None);
    assert_eq!(broken.mean_agent_overall, None);
    let good = model.judges.iter().find(|j| j.judge == "judge/a").unwrap();
    assert_eq!(good.mean_loss, Some(3.0));
    let md = fs::read_to_string(setup.out.path().join("broken/report.md")).unwrap();
    assert!(md.contains("judge/broken: missing"));
}

#[tokio::test]
async fn the_report_gives_no_verdict_and_writes_again_with_no_network() {
    let setup = setup().await;
    start(&setup, &judged(&setup, "words", &["judge/a"]), Mode::New)
        .await
        .unwrap();
    let folder = setup.out.path().join("words");
    let md = fs::read_to_string(folder.join("report.md"))
        .unwrap()
        .to_lowercase();
    for word in ["pass", "fail", "verdict", "minimum", "good enough"] {
        assert!(!md.contains(word), "the report has {word:?}");
    }
    let json = fs::read(folder.join("report.json")).unwrap();
    let requests = setup.mock.seen.lock().unwrap().len();
    fs::remove_file(folder.join("report.md")).unwrap();
    report::write(&folder).unwrap();
    assert_eq!(fs::read(folder.join("report.json")).unwrap(), json);
    assert!(folder.join("report.md").exists());
    assert_eq!(setup.mock.seen.lock().unwrap().len(), requests);
}

#[tokio::test]
async fn compare_gives_a_row_per_shape_and_model_and_refuses_other_notes() {
    let setup = setup().await;
    start(&setup, &judged(&setup, "one", &["judge/a"]), Mode::New)
        .await
        .unwrap();
    let mut steps = judged(&setup, "two", &["judge/a"]);
    steps.shape = Shape::Steps;
    start(&setup, &steps, Mode::New).await.unwrap();
    let one = setup.out.path().join("one");
    let two = setup.out.path().join("two");
    let table = report::compare(&[&one, &two]).unwrap();
    let rows: Vec<&str> = table.lines().skip(2).collect();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].contains("| one | Auto | test/model-a | judge/a |"));
    assert!(rows[1].contains("| two | Steps | test/model-a | judge/a |"));

    let mut fewer = judged(&setup, "three", &["judge/a"]);
    fewer.notes = NotesFilter {
        ids: vec!["note-budget-review".into()],
        list: None,
    };
    start(&setup, &fewer, Mode::New).await.unwrap();
    let three = setup.out.path().join("three");
    let error = report::compare(&[&one, &three]).unwrap_err();
    assert!(error.contains("different notes"), "{error}");
}

#[tokio::test]
async fn the_snapshot_has_no_summary_file_unless_the_run_keeps_it() {
    let setup = setup().await;
    start(&setup, &config(&setup, "plain"), Mode::New)
        .await
        .unwrap();
    let mut keep = config(&setup, "keep");
    keep.existing_summary = true;
    start(&setup, &keep, Mode::New).await.unwrap();
    let note = "snapshot/2026-10-01 0900 Budget review/summary.md";
    assert!(!setup.out.path().join("plain").join(note).exists());
    assert!(setup.out.path().join("keep").join(note).exists());
    // The baseline does not read the summary file: the second run reuses
    // the first run's baseline.
    assert!(baseline(&setup, "keep", "note-budget-review").reused);
}

#[tokio::test]
async fn a_second_baseline_is_written_and_judged_with_the_others() {
    let setup = setup().await;
    let mut two = config(&setup, "two");
    two.baselines = 2;
    two.judges = vec!["judge/a".into()];
    let summary = start(&setup, &two, Mode::New).await.unwrap();
    assert_eq!(summary.baselines_written, 6);
    let dir = results(&setup, "two", "note-budget-review");
    assert!(dir.join("baseline-2.json").exists());
    let judged: JudgeFile =
        serde_json::from_str(&fs::read_to_string(dir.join("judge-judge_a.json")).unwrap()).unwrap();
    let sources: Vec<&str> = judged.order.iter().map(|c| c.source.as_str()).collect();
    assert!(sources.contains(&"baseline") && sources.contains(&"baseline-2"));
    // Both baselines score 9 and the agents 6: the loss is 3.
    let report = report::build(&setup.out.path().join("two")).unwrap();
    let loss = report.pairs[0].judges[0].mean_loss.unwrap();
    assert!((loss - 3.0).abs() < 1e-9, "{loss}");
}

#[tokio::test]
async fn versus_puts_each_run_in_one_request_and_reports_the_differences() {
    let setup = setup().await;
    for name in ["control", "variant"] {
        start(&setup, &config(&setup, name), Mode::New)
            .await
            .unwrap();
    }
    let base = setup.mock.base.clone();
    let resolve = move |model: &str| {
        Ok(Endpoint {
            provider: Provider::OpenRouter,
            base_url: base.clone(),
            model: model.to_string(),
            credential: Some(KEY.to_string()),
        })
    };
    let log = |_: &str| {};
    let lab = Lab {
        out: setup.out.path().to_path_buf(),
        resolve: &resolve,
        log: &log,
    };
    let spec = summary_lab::versus::Spec {
        name: "vs".into(),
        runs: vec!["control".into(), "variant".into()],
        judges: vec!["judge/a".into()],
        seed: 7,
        models: Vec::new(),
    };
    let text = summary_lab::versus::versus(&spec, &lab).await.unwrap();
    // 3 samples with 1 judge for each note.
    let dir = setup.out.path().join("vs/note-budget-review/test_model-a");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 3);
    let judged: JudgeFile =
        serde_json::from_str(&fs::read_to_string(dir.join("judge-judge_a-1.json")).unwrap())
            .unwrap();
    let mut sources: Vec<&str> = judged.order.iter().map(|c| c.source.as_str()).collect();
    sources.sort();
    assert_eq!(sources, ["baseline", "run:control", "run:variant"]);
    // Equal agent texts get equal scores: no difference and three ties.
    let row = text
        .lines()
        .find(|l| l.starts_with("| run:variant"))
        .unwrap();
    assert!(
        row.contains("| 0.00 |") && row.contains("| 0/3/0 |"),
        "{row}"
    );
    assert!(row.contains("| 0/9 |"), "no summary: {row}");
}

#[test]
fn the_bootstrap_interval_holds_the_mean_and_repeats_for_a_seed() {
    let values = [0.5, -0.25, 1.0, 0.0, 0.75, 0.25, -0.5, 0.5];
    let (low, high) = summary_lab::versus::bootstrap(&values, 3).unwrap();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    assert!(low < mean && mean < high, "{low} {mean} {high}");
    assert_eq!(
        summary_lab::versus::bootstrap(&values, 3),
        Some((low, high))
    );
    assert_eq!(summary_lab::versus::bootstrap(&[1.0], 3), None);
}
