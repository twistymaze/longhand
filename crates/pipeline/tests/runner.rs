//! The runner end to end over real chunk files, with a scripted recognizer.

mod common;
#[path = "../../summarize/tests/common/mod.rs"]
mod http;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use asr::{AsrError, AsrSegment, Recognizer, WindowText};
use common::{queue_session, speech, store};
use diarize::{Analysis, AudioSource, DiarizeError, DiarizeRequest, SpeakerAnalyzer};
use pipeline::join::TRANSCRIPT_FILE;
use pipeline::runner::Outcome;
use pipeline::transcribe::{self, WindowPlan};
use pipeline::{
    EndpointProblem, EngineProvider, Environment, Runner, RunnerEvent, SharedStore,
    SummaryEndpoint, checkpoint,
};
use session_core::{SessionState, Stage};

#[derive(Default)]
struct Script {
    /// Model ids loaded, in order.
    loads: Mutex<Vec<String>>,
    calls: AtomicUsize,
    running: AtomicUsize,
    overlapped: AtomicBool,
    /// Fail the call with this ordinal (0-based), once.
    fail_on: Mutex<Option<usize>>,
    /// Trip this after the given number of calls.
    cancel_after: Mutex<Option<(usize, Arc<Runner>, String)>>,
    /// Resume this session through the store after the given number of calls.
    resume_after: Mutex<Option<(usize, SharedStore, String)>>,
    analyses: AtomicUsize,
    /// Where summaries go; unset is an unconfigured endpoint.
    summary: Mutex<Option<Result<SummaryEndpoint, EndpointProblem>>>,
    /// Limits the summarize stage learned.
    learned: Mutex<Vec<u32>>,
    /// Filler sentences each window decodes, to make transcripts long.
    sentences: AtomicUsize,
    /// A sentence the given call decodes as well.
    says: Mutex<Option<(usize, String)>>,
    /// Calls (0-based) that decode nothing.
    blank_on: Mutex<Vec<usize>>,
    diarize_fails: AtomicBool,
    weights_missing: AtomicBool,
    /// Where sessions land; unset is no destination chosen.
    destination: Mutex<Option<std::path::PathBuf>>,
}

/// Analyses without finding anyone: the stage runs, no speaker turns come out.
struct FakeAnalyzer(Arc<Script>);

impl SpeakerAnalyzer for FakeAnalyzer {
    fn analyze(
        &mut self,
        audio: &mut dyn AudioSource,
        request: &DiarizeRequest<'_>,
    ) -> Result<Analysis, DiarizeError> {
        self.0.analyses.fetch_add(1, Ordering::SeqCst);
        if self.0.diarize_fails.load(Ordering::SeqCst) {
            return Err(DiarizeError::Format("segmentation exploded".into()));
        }
        if let Some(report) = request.on_progress {
            report(1.0);
        }
        Ok(Analysis {
            total: audio.sample_count().map_err(DiarizeError::Engine)?,
            step: 32_000,
            ..Analysis::default()
        })
    }
}

struct Fake {
    model: String,
    script: Arc<Script>,
}

impl Recognizer for Fake {
    fn model_id(&self) -> &str {
        &self.model
    }

    fn transcribe(&mut self, pcm: &[f32]) -> Result<WindowText, AsrError> {
        let s = &self.script;
        if s.running.fetch_add(1, Ordering::SeqCst) > 0 {
            s.overlapped.store(true, Ordering::SeqCst);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
        let n = s.calls.fetch_add(1, Ordering::SeqCst);
        s.running.fetch_sub(1, Ordering::SeqCst);
        if s.fail_on.lock().unwrap().take_if(|f| *f == n).is_some() {
            return Err(AsrError::Run("decoder exploded".into()));
        }
        if let Some((after, runner, id)) = s.cancel_after.lock().unwrap().as_ref()
            && n + 1 == *after
        {
            runner.cancel(id);
        }
        if let Some((after, store, id)) = s.resume_after.lock().unwrap().as_ref()
            && n + 1 == *after
        {
            store
                .lock()
                .unwrap()
                .apply(
                    id,
                    &session_core::Event::UserResume {
                        now: common::at(150),
                        is_most_recent: true,
                        space_ok: true,
                    },
                )
                .unwrap();
        }
        if s.blank_on.lock().unwrap().contains(&n) {
            return Ok(WindowText {
                text: String::new(),
                segments: vec![],
            });
        }
        let ms = pcm.len() as u64 / 16;
        let mut text = format!("call {n}.");
        for _ in 0..s.sentences.load(Ordering::SeqCst) {
            text.push_str(" Someone talks at length about the rules of the game.");
        }
        if let Some((at, said)) = s.says.lock().unwrap().as_ref()
            && *at == n
        {
            text.push(' ');
            text.push_str(said);
        }
        Ok(WindowText {
            text: text.clone(),
            segments: vec![AsrSegment {
                start_ms: 0,
                end_ms: ms,
                text,
            }],
        })
    }
}

struct Provider(Arc<Script>);

impl EngineProvider for Provider {
    fn ready(&self, _: &str) -> bool {
        true
    }
    fn load(&self, model_id: &str) -> Result<Box<dyn Recognizer>, String> {
        self.0.loads.lock().unwrap().push(model_id.to_string());
        Ok(Box::new(Fake {
            model: model_id.to_string(),
            script: Arc::clone(&self.0),
        }))
    }
    fn default_model(&self) -> String {
        "selected-now".into()
    }
    fn analyzer(&self) -> Result<Box<dyn SpeakerAnalyzer>, DiarizeError> {
        if self.0.weights_missing.load(Ordering::SeqCst) {
            return Err(DiarizeError::MissingWeights("segmentation.onnx".into()));
        }
        Ok(Box::new(FakeAnalyzer(Arc::clone(&self.0))))
    }
    fn summary_endpoint(&self) -> Result<SummaryEndpoint, EndpointProblem> {
        self.0
            .summary
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Err(EndpointProblem::Unconfigured("no endpoint".into())))
    }
    fn learn_limit(&self, _: &str, _: &str, limit: u32) {
        self.0.learned.lock().unwrap().push(limit);
    }
    fn destination(&self) -> Option<std::path::PathBuf> {
        self.0.destination.lock().unwrap().clone()
    }
}

fn endpoint(base: &str) -> SummaryEndpoint {
    SummaryEndpoint {
        base_url: base.into(),
        model: "summarizer".into(),
        credential: Some("sk-secret-test".into()),
        context_limit: None,
        learned_limit: None,
    }
}

/// An endpoint that answers every completion with the same summary.
async fn with_summaries(script: &Arc<Script>) -> http::Mock {
    let mock = http::serve(std::collections::HashMap::from([(
        "/v1/chat/completions",
        http::Reply::Json(200, http::completion("A short summary.")),
    )]))
    .await;
    *script.summary.lock().unwrap() = Some(Ok(endpoint(&mock.base)));
    mock
}

struct Rig {
    runner: Arc<Runner>,
    store: SharedStore,
    script: Arc<Script>,
    events: Arc<Mutex<Vec<RunnerEvent>>>,
    _root: tempfile::TempDir,
    root: std::path::PathBuf,
    _dest: tempfile::TempDir,
    /// The destination the rig starts with.
    dest: std::path::PathBuf,
}

fn rig_with(store: SharedStore, root: tempfile::TempDir, script: Arc<Script>) -> Rig {
    let dest_dir = tempfile::tempdir().unwrap();
    let dest = dest_dir.path().to_path_buf();
    *script.destination.lock().unwrap() = Some(dest.clone());
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let path = root.path().to_path_buf();
    let runner = Runner::new(Environment {
        store: Arc::clone(&store),
        working_root: path.clone(),
        engines: Box::new(Provider(Arc::clone(&script))),
        suspended: Box::new(|| false),
        events: Box::new(move |e| sink.lock().unwrap().push(e)),
    });
    Rig {
        runner,
        store,
        script,
        events,
        _root: root,
        root: path,
        _dest: dest_dir,
        dest,
    }
}

/// The folder a session landed in.
fn landed(r: &Rig, id: &str) -> std::path::PathBuf {
    let record = r.store.lock().unwrap().session(id).unwrap().unwrap();
    assert_eq!(record.state, SessionState::Landed);
    std::path::PathBuf::from(record.landed_path.expect("a landed path"))
}

async fn rig() -> Rig {
    let r = rig_with(store(), tempfile::tempdir().unwrap(), Arc::default());
    with_summaries(&r.script).await;
    r
}

/// Speech, a pause long enough to split windows, and speech again.
fn conversation() -> Vec<f32> {
    let speech = speech();
    let mut audio = speech.clone();
    audio.resize(audio.len() + 16_000 * 5, 0.0);
    audio.extend_from_slice(&speech);
    audio
}

fn plan(store: &SharedStore, id: &str) -> WindowPlan {
    checkpoint::get(store, id, Stage::Transcribe, &checkpoint::plan_key(0))
        .unwrap()
        .unwrap()
}

fn state(store: &SharedStore, id: &str) -> SessionState {
    store.lock().unwrap().session(id).unwrap().unwrap().state
}

#[tokio::test(flavor = "multi_thread")]
async fn two_queued_sessions_run_one_stage_at_a_time_in_stop_order() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "earlier",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    queue_session(
        &r.store,
        &r.root,
        "later",
        200,
        Some("m"),
        &[(16_000, speech())],
    );

    let first = r.runner.run_next().await.unwrap();
    let second = r.runner.run_next().await.unwrap();
    assert!(r.runner.run_next().await.is_none());
    assert_eq!(first, ("earlier".into(), Outcome::Finished));
    assert_eq!(second, ("later".into(), Outcome::Finished));
    assert!(!r.script.overlapped.load(Ordering::SeqCst));

    let started: Vec<(String, Stage)> = r
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            RunnerEvent::StageStarted { session_id, stage } => Some((session_id.clone(), *stage)),
            _ => None,
        })
        .collect();
    assert_eq!(
        started,
        vec![
            ("earlier".into(), Stage::Assemble),
            ("earlier".into(), Stage::Diarize),
            ("earlier".into(), Stage::Transcribe),
            ("earlier".into(), Stage::Join),
            ("earlier".into(), Stage::Verify),
            ("earlier".into(), Stage::Summarize),
            ("earlier".into(), Stage::Land),
            ("later".into(), Stage::Assemble),
            ("later".into(), Stage::Diarize),
            ("later".into(), Stage::Transcribe),
            ("later".into(), Stage::Join),
            ("later".into(), Stage::Verify),
            ("later".into(), Stage::Summarize),
            ("later".into(), Stage::Land),
        ]
    );
    // One engine load serves both sessions.
    assert_eq!(*r.script.loads.lock().unwrap(), vec!["m".to_string()]);
    let results = transcribe::results(&r.store, "earlier").unwrap();
    assert_eq!(results.len(), plan(&r.store, "earlier").windows.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_mid_segment_stops_at_the_next_window_and_keeps_what_is_done() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    *r.script.cancel_after.lock().unwrap() = Some((2, Arc::clone(&r.runner), "s".into()));

    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Cancelled);
    assert!(plan(&r.store, "s").windows.len() > 2);
    assert_eq!(r.script.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transcribe::results(&r.store, "s").unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_dropped_part_way_resumes_at_the_first_unfinished_window() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    let first = rig_with(Arc::clone(&store), root, Arc::default());
    queue_session(
        &first.store,
        &first.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    *first.script.cancel_after.lock().unwrap() = Some((2, Arc::clone(&first.runner), "s".into()));
    first.runner.run_next().await.unwrap();
    let total = plan(&store, "s").windows.len();
    let before = transcribe::results(&store, "s").unwrap();

    // A new process: the old claim is reclaimed at launch.
    let Rig { _root, .. } = first;
    let second = rig_with(Arc::clone(&store), _root, Arc::default());
    with_summaries(&second.script).await;
    assert_eq!(second.runner.reclaim().unwrap(), vec!["s".to_string()]);
    let (_, outcome) = second.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    assert_eq!(second.script.calls.load(Ordering::SeqCst), total - 2);
    let after = transcribe::results(&store, "s").unwrap();
    assert_eq!(after.len(), total);
    assert_eq!(after[..2], before[..]);
    // Assemble was not redone: its stage never started in the new process.
    assert!(second.events.lock().unwrap().iter().all(|e| !matches!(
        e,
        RunnerEvent::StageStarted {
            stage: Stage::Assemble,
            ..
        }
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_engine_error_fails_the_stage_naming_the_window_and_keeps_audio_and_earlier_windows() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    *r.script.fail_on.lock().unwrap() = Some(1);

    let (_, outcome) = r.runner.run_next().await.unwrap();
    let Outcome::Failed { stage, message } = outcome else {
        panic!("expected a failure");
    };
    assert_eq!(stage, Stage::Transcribe);
    assert!(message.contains("window 1"), "{message}");
    assert!(matches!(
        state(&r.store, "s"),
        SessionState::ProcessingFailed {
            stage: Stage::Transcribe,
            ..
        }
    ));
    assert_eq!(transcribe::results(&r.store, "s").unwrap().len(), 1);
    assert!(r.root.join("s/audio/segment-01.opus").exists());
    assert!(r.root.join("s/audio-recovery").read_dir().unwrap().count() > 0);
    assert!(r.events.lock().unwrap().iter().any(|e| matches!(
        e,
        RunnerEvent::StageFailed {
            stage: Stage::Transcribe,
            ..
        }
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_session_uses_the_model_recorded_at_stop() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("chosen-at-stop"),
        &[(16_000, speech())],
    );
    r.runner.run_next().await.unwrap();
    assert_eq!(
        *r.script.loads.lock().unwrap(),
        vec!["chosen-at-stop".to_string()]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_starts_while_capture_is_active() {
    let store = store();
    let root = tempfile::tempdir().unwrap();
    let capturing = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&capturing);
    let script: Arc<Script> = Arc::default();
    with_summaries(&script).await;
    let dest = tempfile::tempdir().unwrap();
    *script.destination.lock().unwrap() = Some(dest.path().to_path_buf());
    let runner = Runner::new(Environment {
        store: Arc::clone(&store),
        working_root: root.path().to_path_buf(),
        engines: Box::new(Provider(script)),
        suspended: Box::new(move || flag.load(Ordering::SeqCst)),
        events: Box::new(|_| {}),
    });
    queue_session(
        &store,
        root.path(),
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    assert!(runner.run_next().await.is_none());
    assert!(matches!(state(&store, "s"), SessionState::Queued { .. }));
    capturing.store(false, Ordering::SeqCst);
    assert_eq!(runner.run_next().await.unwrap().1, Outcome::Finished);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_takes_the_session_back_at_the_next_window_without_being_told() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    *r.script.resume_after.lock().unwrap() = Some((1, Arc::clone(&r.store), "s".into()));

    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Cancelled);
    assert_eq!(r.script.calls.load(Ordering::SeqCst), 1);
    assert_eq!(state(&r.store, "s"), SessionState::Capturing);
    // The completed window survives the resume (R6b).
    assert_eq!(transcribe::results(&r.store, "s").unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_session_has_its_transcript_on_disk() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech()), (16_000, speech())],
    );
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    // One analysis per segment, clustered together.
    assert_eq!(r.script.analyses.load(Ordering::SeqCst), 2);
    let transcript = std::fs::read_to_string(landed(&r, "s").join(TRANSCRIPT_FILE)).unwrap();
    assert!(transcript.contains("call 0"), "{transcript}");
    assert!(r.events.lock().unwrap().iter().any(|e| matches!(
        e,
        RunnerEvent::Progress {
            stage: Stage::Diarize,
            done: 100,
            ..
        }
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_diarization_failure_delivers_the_transcript_without_speakers() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    r.script.diarize_fails.store(true, Ordering::SeqCst);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    assert_eq!(
        pipeline::diarize::speakers(&r.store, "s").unwrap(),
        pipeline::diarize::Speakers::Unavailable
    );
    assert!(r.events.lock().unwrap().iter().any(|e| matches!(
        e,
        RunnerEvent::SpeakersUnavailable { message, .. } if message.contains("segmentation exploded")
    )));
    let transcript = std::fs::read_to_string(landed(&r, "s").join(TRANSCRIPT_FILE)).unwrap();
    assert!(transcript.contains("call 0"), "{transcript}");
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_weights_put_the_session_back_in_line_with_its_audio_ready() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    r.script.weights_missing.store(true, Ordering::SeqCst);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Waiting);
    assert!(matches!(state(&r.store, "s"), SessionState::Queued { .. }));
    assert_eq!(
        r.store.lock().unwrap().queue().unwrap(),
        vec!["s".to_string()]
    );
    assert!(r.root.join("s/audio/segment-01.opus").exists());

    // The weights land: it picks up after assembly.
    r.script.weights_missing.store(false, Ordering::SeqCst);
    r.events.lock().unwrap().clear();
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    assert!(r.events.lock().unwrap().iter().all(|e| !matches!(
        e,
        RunnerEvent::StageStarted {
            stage: Stage::Assemble,
            ..
        }
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_window_that_missed_speech_fails_verification_and_retry_redoes_only_it() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    *r.script.blank_on.lock().unwrap() = vec![1];
    let (_, outcome) = r.runner.run_next().await.unwrap();
    let Outcome::Failed { stage, message } = outcome else {
        panic!("expected a failure, got {outcome:?}");
    };
    assert_eq!(stage, Stage::Verify);
    assert!(message.contains("misses speech"), "{message}");
    assert!(r.root.join("s").join(TRANSCRIPT_FILE).exists());
    let windows = plan(&r.store, "s").windows.len();
    assert_eq!(r.script.calls.load(Ordering::SeqCst), windows);

    // The decoder recovers; only the blank window runs again.
    r.script.blank_on.lock().unwrap().clear();
    r.runner.retry("s").unwrap();
    assert!(matches!(state(&r.store, "s"), SessionState::Queued { .. }));
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    assert_eq!(r.script.calls.load(Ordering::SeqCst), windows + 1);
    let transcript = std::fs::read_to_string(landed(&r, "s").join(TRANSCRIPT_FILE)).unwrap();
    assert!(
        transcript.contains(&format!("call {windows}")),
        "{transcript}"
    );
}

// ------------------------------------------------------------- summarize (U10)

fn user_content(body: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    v["messages"][1]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn chat_calls(mock: &http::Mock) -> Vec<String> {
    mock.seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.path == "/v1/chat/completions")
        .map(|s| user_content(&s.body))
        .collect()
}

/// Point the script's summaries at `reply`, with an optional context setting.
async fn summaries_from(r: &Rig, reply: http::Reply, context_limit: Option<u32>) -> http::Mock {
    let mock = http::serve(std::collections::HashMap::from([(
        "/v1/chat/completions",
        reply,
    )]))
    .await;
    *r.script.summary.lock().unwrap() = Some(Ok(SummaryEndpoint {
        context_limit,
        ..endpoint(&mock.base)
    }));
    mock
}

/// A session long enough to need several calls under a small budget.
fn long_session(r: &Rig) {
    r.script.sentences.store(60, Ordering::SeqCst);
    let segments: Vec<(u32, Vec<f32>)> = (0..4).map(|_| (16_000, conversation())).collect();
    queue_session(&r.store, &r.root, "s", 100, Some("m"), &segments);
}

fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transcript_that_fits_is_summarized_in_one_call_and_names_host_and_model() {
    let r = rig().await;
    let mock = summaries_from(
        &r,
        http::Reply::Json(200, http::completion("One pass.")),
        None,
    )
    .await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    let calls = chat_calls(&mock);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains("<transcript>"));
    let summary = std::fs::read_to_string(landed(&r, "s").join("summary.md")).unwrap();
    assert!(summary.contains("endpoint_host: 127.0.0.1"), "{summary}");
    assert!(summary.contains("model: summarizer"), "{summary}");
    assert!(summary.contains("One pass."));
    assert!(
        mock.seen.lock().unwrap()[0..]
            .iter()
            .all(|s| s.path != "/v1/chat/completions"
                || s.authorization.as_deref() == Some("Bearer sk-secret-test"))
    );
    // The credential and the settings are in no file of the session (R24b,
    // R33, AE13): only the host and model the summary names.
    for file in files_under(&landed(&r, "s")) {
        let text = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).to_string();
        for secret in [
            "sk-secret-test",
            mock.base.as_str(),
            &r.dest.to_string_lossy(),
        ] {
            assert!(!text.contains(secret), "{} holds {secret}", file.display());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_transcript_is_summarized_in_parts_and_merged_in_layers() {
    let r = rig().await;
    let mock = summaries_from(
        &r,
        http::Reply::Json(200, http::completion("Notes on a part.")),
        Some(2_048),
    )
    .await;
    long_session(&r);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    let calls = chat_calls(&mock);
    let parts = calls
        .iter()
        .filter(|c| c.contains("<transcript part="))
        .count();
    let merges = calls
        .iter()
        .filter(|c| c.contains("<summary part="))
        .count();
    assert!(parts > 2, "{parts} parts");
    assert!(merges >= 1);
    // Each prompt stays inside the budget the setting names.
    for c in &calls {
        assert!(summarize::tokens::estimate(c) < 2_048, "{}", c.len());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_error_on_part_five_lands_a_stub_and_regenerating_starts_there() {
    let r = rig().await;
    let n = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&n);
    let reply = http::Reply::With(Arc::new(move |_| {
        if count.fetch_add(1, Ordering::SeqCst) == 4 {
            (500, r#"{"error":"overloaded"}"#.into())
        } else {
            (200, http::completion("Notes on a part."))
        }
    }));
    let mock = summaries_from(&r, reply, Some(2_048)).await;
    long_session(&r);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    // The summary failing costs the summary, not the session (KD13).
    assert_eq!(outcome, Outcome::Finished);
    assert!(r.events.lock().unwrap().iter().any(|e| matches!(
        e,
        RunnerEvent::SummaryUnavailable { message, .. } if message.contains("part 5 of")
    )));
    assert_eq!(chat_calls(&mock).len(), 5);
    let path = landed(&r, "s").join("summary.md");
    let stub =
        session_core::folder::Summary::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        stub.meta,
        session_core::folder::SummaryMeta::Stub {
            failed_stage: "summarize".into()
        }
    );
    assert!(stub.body.contains("part 5 of"), "{}", stub.body);

    // Regenerate (AE17) picks up at the part that failed.
    assert_eq!(r.runner.regenerate_summary("s").await.unwrap(), path);
    let calls = chat_calls(&mock);
    assert!(
        calls[5].contains("<transcript part=\"5\">"),
        "{}",
        &calls[5][..80]
    );
    assert!(
        calls[5..]
            .iter()
            .all(|c| !c.contains("<transcript part=\"4\">"))
    );
    let summary =
        session_core::folder::Summary::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(matches!(
        summary.meta,
        session_core::folder::SummaryMeta::Complete { .. }
    ));
    assert_eq!(state(&r.store, "s"), SessionState::Landed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_locked_keychain_blocks_and_unlocking_resumes_without_redoing_work() {
    let r = rig().await;
    *r.script.summary.lock().unwrap() = Some(Err(EndpointProblem::Locked(
        "the login keychain is locked".into(),
    )));
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Blocked);
    assert!(matches!(state(&r.store, "s"), SessionState::Blocked { .. }));
    assert!(
        r.runner.run_next().await.is_none(),
        "a blocked session is not claimed"
    );
    let transcribed = r.script.calls.load(Ordering::SeqCst);

    let mock = summaries_from(&r, http::Reply::Json(200, http::completion("Done.")), None).await;
    r.runner.unblock("s").unwrap();
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    assert_eq!(r.script.calls.load(Ordering::SeqCst), transcribed);
    assert_eq!(chat_calls(&mock).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejection_the_estimate_did_not_foresee_shrinks_the_budget_and_learns_it() {
    let r = rig().await;
    // The server takes far less than the default budget it never reports.
    let reply = http::Reply::With(Arc::new(|body: &str| {
        if body.len() > 6_000 {
            (
                400,
                r#"{"error":{"message":"This model's maximum context length is 2048 tokens"}}"#
                    .into(),
            )
        } else {
            (200, http::completion("Notes."))
        }
    }));
    let _mock = summaries_from(&r, reply, None).await;
    long_session(&r);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished, "{outcome:?}");
    let learned = r.script.learned.lock().unwrap().clone();
    assert!(!learned.is_empty());
    assert!(learned.windows(2).all(|w| w[1] < w[0]), "{learned:?}");
    assert!(*learned.last().unwrap() < 8_192);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_during_summarization_discards_it_and_keeps_the_transcription() {
    let r = rig().await;
    let _mock = summaries_from(&r, http::Reply::Hang, None).await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    let store = Arc::clone(&r.store);
    let events = Arc::clone(&r.events);
    tokio::spawn(async move {
        loop {
            let summarizing = events.lock().unwrap().iter().any(|e| {
                matches!(
                    e,
                    RunnerEvent::StageStarted {
                        stage: Stage::Summarize,
                        ..
                    }
                )
            });
            if summarizing {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        store
            .lock()
            .unwrap()
            .apply(
                "s",
                &session_core::Event::UserResume {
                    now: common::at(150),
                    is_most_recent: true,
                    space_ok: true,
                },
            )
            .unwrap();
    });
    let started = std::time::Instant::now();
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Cancelled);
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert_eq!(state(&r.store, "s"), SessionState::Capturing);
    let kept = r.store.lock().unwrap();
    assert!(kept.checkpoints("s", Stage::Summarize).unwrap().is_empty());
    assert!(!kept.checkpoints("s", Stage::Transcribe).unwrap().is_empty());
    assert!(!kept.checkpoints("s", Stage::Diarize).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerating_rewrites_the_summary_without_transcribing_again() {
    let r = rig().await;
    let n = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&n);
    let reply = http::Reply::With(Arc::new(move |_| {
        let k = count.fetch_add(1, Ordering::SeqCst);
        (200, http::completion(&format!("Version {k}.")))
    }));
    let _mock = summaries_from(&r, reply, None).await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let transcribed = r.script.calls.load(Ordering::SeqCst);
    let path = r.runner.regenerate_summary("s").await.unwrap();
    assert_eq!(r.script.calls.load(Ordering::SeqCst), transcribed);
    assert_eq!(path, landed(&r, "s").join("summary.md"));
    let summary = std::fs::read_to_string(&path).unwrap();
    assert!(summary.contains("Version 1."), "{summary}");
    let names: Vec<String> = std::fs::read_dir(landed(&r, "s"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.iter().all(|n| !n.starts_with('.')), "{names:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unannotated_decision_survives_the_layers_to_the_final_summary() {
    let r = rig().await;
    // A model that echoes what it was given, so only the framing can lose it.
    let echo = http::Reply::With(Arc::new(|body: &str| {
        let content = user_content(body);
        let data: String = content
            .lines()
            .filter(|l| !l.starts_with('<') && !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        (200, http::completion(&data))
    }));
    let _mock = summaries_from(&r, echo, Some(4_096)).await;
    *r.script.says.lock().unwrap() = Some((5, "We decided to meet on Friday instead.".into()));
    long_session(&r);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished, "{outcome:?}");
    let summary = std::fs::read_to_string(landed(&r, "s").join("summary.md")).unwrap();
    assert!(summary.contains("We decided to meet on Friday instead."));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_endpoint_lands_the_session_with_a_stub_naming_the_setting() {
    let r = rig().await;
    *r.script.summary.lock().unwrap() = None;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    let folder = landed(&r, "s");
    assert!(folder.join(TRANSCRIPT_FILE).exists());
    let summary = std::fs::read_to_string(folder.join("summary.md")).unwrap();
    assert!(summary.contains("status: stub"), "{summary}");
    assert!(summary.contains("failed_stage: summarize"), "{summary}");
    assert!(summary.contains("no endpoint"), "{summary}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_cut_off_gets_more_room_and_teaches_no_limit() {
    let r = rig().await;
    // A reasoning model: it needs 1,500 tokens of reply before it answers.
    let reply = http::Reply::With(Arc::new(|body: &str| {
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        if v["max_tokens"].as_u64().unwrap_or(0) < 1_500 {
            (
                200,
                r#"{"choices":[{"message":{"content":""},"finish_reason":"length"}]}"#.into(),
            )
        } else {
            (200, http::completion("Notes."))
        }
    }));
    let _mock = summaries_from(&r, reply, Some(4_096)).await;
    long_session(&r);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished, "{outcome:?}");
    assert!(r.script.learned.lock().unwrap().is_empty());
}

// ------------------------------------------------------------- landing (U11)

fn add_notes(store: &SharedStore, id: &str) -> Vec<i64> {
    let s = store.lock().unwrap();
    vec![
        s.add_note(
            id,
            "the rules changed",
            session_core::Anchor::At {
                position_ms: 1_000,
                gap_anchored: false,
            },
        )
        .unwrap(),
        s.add_note(id, "bring snacks", session_core::Anchor::Unanchored)
            .unwrap(),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn a_processed_session_lands_once_as_a_folder_that_parses_back() {
    use session_core::folder::{self, SessionDoc, Transcript};
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech()), (16_000, speech())],
    );
    add_notes(&r.store, "s");
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);

    let folder = landed(&r, "s");
    assert_eq!(folder.parent(), Some(r.dest.as_path()));
    assert_eq!(
        folder.file_name().unwrap().to_string_lossy(),
        landing::naming::folder_name(common::at(0))
    );
    assert!(!r.root.join("s").exists(), "the working copy is gone");
    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "audio",
            "notes.md",
            "session.md",
            "summary.md",
            "transcript.md"
        ]
    );
    assert!(folder.join("audio/segment-01.opus").exists());
    assert!(folder.join("audio/segment-02.opus").exists());
    // Only the folder itself is in the destination: no staging left.
    assert_eq!(std::fs::read_dir(&r.dest).unwrap().count(), 1);

    // The folder alone rebuilds the session (R25).
    let doc =
        SessionDoc::parse(&std::fs::read_to_string(folder.join("session.md")).unwrap()).unwrap();
    assert_eq!(
        doc.timeline_segments(),
        r.store.lock().unwrap().segments("s").unwrap()
    );
    let notes =
        folder::parse_notes(&std::fs::read_to_string(folder.join("notes.md")).unwrap()).unwrap();
    let texts: Vec<&str> = notes.iter().map(|n| n.text.as_str()).collect();
    assert_eq!(texts, ["the rules changed", "bring snacks"]);
    let transcript =
        Transcript::parse(&std::fs::read_to_string(folder.join(TRANSCRIPT_FILE)).unwrap()).unwrap();
    assert!(!transcript.lines.is_empty());

    assert!(r.events.lock().unwrap().iter().any(|e| matches!(
        e,
        RunnerEvent::Landed { path, copied: false, .. } if *path == folder
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_edited_after_landing_rewrites_the_landed_file_whole() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    let ids = add_notes(&r.store, "s");
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let folder = landed(&r, "s");

    {
        let mut s = r.store.lock().unwrap();
        s.edit_note(ids[1], "bring more snacks").unwrap();
        assert!(pipeline::land::rewrite_notes(&mut s, "s").unwrap());
    }
    let notes = std::fs::read_to_string(folder.join("notes.md")).unwrap();
    assert_eq!(notes.matches("bring more snacks").count(), 1, "{notes}");
    assert!(!notes.contains("- bring snacks"));
    let names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.iter().all(|n| !n.starts_with('.')), "{names:?}");
    assert_eq!(state(&r.store, "s"), SessionState::Landed);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_destination_the_session_waits_whole_and_lands_in_the_last_one_chosen() {
    let r = rig().await;
    *r.script.destination.lock().unwrap() = None;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    let ids = add_notes(&r.store, "s");
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert!(
        matches!(&outcome, Outcome::AwaitingDestination { problem } if problem.contains("no destination")),
        "{outcome:?}"
    );
    assert!(matches!(
        state(&r.store, "s"),
        SessionState::AwaitingDestination { .. }
    ));
    assert!(r.root.join("s/audio/segment-01.opus").exists());
    assert!(r.root.join("s/proc").exists());
    // Nothing moved, so the session can still be resumed.
    assert!(!pipeline::land::begun(&r.store.lock().unwrap(), "s").unwrap());

    // A destination that has gone missing waits the same way.
    let gone = r.dest.join("unplugged");
    *r.script.destination.lock().unwrap() = Some(gone.clone());
    assert_eq!(r.runner.retry_awaiting().unwrap(), vec!["s".to_string()]);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert!(
        matches!(&outcome, Outcome::AwaitingDestination { problem } if problem.contains("is missing")),
        "{outcome:?}"
    );

    // An edit made while waiting reaches the landed file, and the session
    // lands in the destination chosen last (AE7).
    r.store
        .lock()
        .unwrap()
        .edit_note(ids[0], "the rules changed twice")
        .unwrap();
    let last = r.dest.join("chosen-last");
    std::fs::create_dir(&last).unwrap();
    *r.script.destination.lock().unwrap() = Some(last.clone());
    r.runner.retry_awaiting().unwrap();
    let transcribed = r.script.calls.load(Ordering::SeqCst);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(r.script.calls.load(Ordering::SeqCst), transcribed);
    let folder = landed(&r, "s");
    assert_eq!(folder.parent(), Some(last.as_path()));
    assert!(!gone.exists());
    let notes = std::fs::read_to_string(folder.join("notes.md")).unwrap();
    assert_eq!(notes.matches("the rules changed").count(), 1, "{notes}");
    assert!(notes.contains("the rules changed twice"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_taken_folder_name_gets_a_suffix() {
    let r = rig().await;
    std::fs::create_dir(r.dest.join(landing::naming::folder_name(common::at(0)))).unwrap();
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(
        landed(&r, "s").file_name().unwrap().to_string_lossy(),
        format!("{}-2", landing::naming::folder_name(common::at(0)))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unverified_transcript_is_refused_landing() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    let segments = r.store.lock().unwrap().segments("s").unwrap();
    let err = pipeline::land::run(
        &r.store,
        "s",
        &r.root.join("s"),
        &segments,
        Some(&r.dest),
        &landing::RealFs,
        &pipeline::Unattended,
    )
    .unwrap_err();
    assert!(err.to_string().contains("not been verified"), "{err}");
    assert!(r.root.join("s/audio-recovery").exists());
    assert_eq!(std::fs::read_dir(&r.dest).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relaunch_after_the_move_finishes_landing_without_redoing_any_stage() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    // Land into a missing folder to stop just before the move, then move
    // the session into staging by hand, as a killed process would leave it.
    *r.script.destination.lock().unwrap() = None;
    r.runner.run_next().await.unwrap();
    *r.script.destination.lock().unwrap() = Some(r.dest.clone());
    let segments = r.store.lock().unwrap().segments("s").unwrap();
    {
        let mut s = r.store.lock().unwrap();
        s.apply("s", &session_core::Event::Retry).unwrap();
    }
    let job = landing::Job {
        session_id: "s",
        working: &r.root.join("s"),
        destination: &r.dest,
        folder_name: &landing::naming::folder_name(segments[0].wall_start),
    };
    let staged = landing::step(&landing::LandState::Working, &job, &landing::RealFs).unwrap();
    checkpoint::put(&r.store, "s", Stage::Land, pipeline::land::STATE, &staged).unwrap();
    {
        let mut s = r.store.lock().unwrap();
        s.apply("s", &session_core::Event::ClaimLost).unwrap();
    }
    r.events.lock().unwrap().clear();

    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let started: Vec<Stage> = r
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            RunnerEvent::StageStarted { stage, .. } => Some(*stage),
            _ => None,
        })
        .collect();
    assert_eq!(started, vec![Stage::Land]);
    assert!(landed(&r, "s").join("audio/segment-01.opus").exists());
}
