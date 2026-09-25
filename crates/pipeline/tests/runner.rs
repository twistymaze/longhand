//! The runner end to end over real chunk files, with a scripted recognizer.

mod common;
#[path = "../../summarize/tests/common/mod.rs"]
mod http;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use asr::{
    AsrError, AsrSegment, LiveStream, LiveText, PartPolicy, Parts, Recognizer, TimedToken,
    WindowText,
};
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
    /// Engines dropped.
    unloads: AtomicUsize,
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
    /// The model streams: each stream says one word for each full second it
    /// hears.
    streaming: AtomicBool,
    /// Where sessions are saved; unset is no destination chosen.
    destination: Mutex<Option<std::path::PathBuf>>,
    /// The model of each transcribe call, in order.
    models_called: Mutex<Vec<String>>,
    /// Set this flag, as capture starts, after the given number of calls.
    capture_after: Mutex<Option<(usize, Arc<AtomicBool>)>>,
    /// Models that are not on disk.
    missing: Mutex<Vec<String>>,
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

impl Drop for Fake {
    fn drop(&mut self) {
        self.script.unloads.fetch_add(1, Ordering::SeqCst);
    }
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
        s.models_called.lock().unwrap().push(self.model.clone());
        if let Some((after, flag)) = s.capture_after.lock().unwrap().as_ref()
            && n + 1 == *after
        {
            flag.store(true, Ordering::SeqCst);
        }
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
                        audio_intact: true,
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

    fn start_stream(&mut self) -> Result<Option<Box<dyn LiveStream + '_>>, AsrError> {
        if !self.script.streaming.load(Ordering::SeqCst) {
            return Ok(None);
        }
        Ok(Some(Box::new(FakeStream { samples: 0 })))
    }
}

struct FakeStream {
    samples: u64,
}

impl LiveStream for FakeStream {
    fn feed(&mut self, pcm: &[f32]) -> Result<bool, AsrError> {
        let before = self.samples / 16_000;
        self.samples += pcm.len() as u64;
        Ok(self.samples / 16_000 > before)
    }
    fn text(&self) -> LiveText {
        LiveText {
            committed: format!("{} seconds", self.samples / 16_000),
            tentative: String::new(),
        }
    }
    fn finish(&mut self) -> Result<Vec<TimedToken>, AsrError> {
        Ok((0..self.samples / 16_000)
            .map(|n| TimedToken {
                text: format!(" s{n}"),
                start_ms: n * 1_000,
                end_ms: n * 1_000 + 500,
            })
            .collect())
    }
    fn reset(&mut self) {
        self.samples = 0;
    }
}

struct Provider(Arc<Script>);

impl EngineProvider for Provider {
    fn ready(&self, model_id: &str) -> bool {
        !self.0.missing.lock().unwrap().iter().any(|m| m == model_id)
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

/// An endpoint that answers every completion with the same summary, and
/// fails every title call, so a session keeps the title it has.
async fn with_summaries(script: &Arc<Script>) -> http::Mock {
    let mock = http::serve(std::collections::HashMap::from([(
        "/v1/chat/completions",
        without_titles(http::Reply::Json(200, http::completion("A short summary."))),
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

/// The folder that holds a saved session.
fn saved_folder(r: &Rig, id: &str) -> std::path::PathBuf {
    let record = r.store.lock().unwrap().session(id).unwrap().unwrap();
    assert_eq!(record.state, SessionState::Saved);
    std::path::PathBuf::from(record.home_path.expect("a home path"))
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

#[test]
fn a_loaded_engine_streams_parts_from_the_engine_slot_when_its_model_streams() {
    let script = Arc::new(Script::default());
    let provider = Provider(Arc::clone(&script));
    // The runner's engine slot holds the loaded recognizer (KTD9).
    let slot: Mutex<Option<Box<dyn Recognizer>>> =
        Mutex::new(Some(provider.load("streams").unwrap()));
    let audio = vec![0.0f32; 3_500 * 16];
    let run = |slot: &Mutex<Option<Box<dyn Recognizer>>>| {
        let mut guard = slot.lock().unwrap();
        let recognizer = guard.as_mut().unwrap();
        let mut parts = Parts::with_classifier(0, PartPolicy::default(), |_: &[f32]| true);
        asr::stream_all(&mut **recognizer, &mut parts, &audio, 320).unwrap()
    };

    assert!(run(&slot).is_none(), "a model that does not stream");

    script.streaming.store(true, Ordering::SeqCst);
    let parts = run(&slot).unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!((parts[0].start_ms, parts[0].end_ms), (0, 3_500));
    assert_eq!(asr::join_words(&parts[0].words), "s0 s1 s2");
    // The slot still holds the engine after the stream.
    assert_eq!(slot.lock().unwrap().as_ref().unwrap().model_id(), "streams");
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
            ("earlier".into(), Stage::Save),
            ("later".into(), Stage::Assemble),
            ("later".into(), Stage::Diarize),
            ("later".into(), Stage::Transcribe),
            ("later".into(), Stage::Join),
            ("later".into(), Stage::Verify),
            ("later".into(), Stage::Summarize),
            ("later".into(), Stage::Save),
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
    assert_eq!(
        second
            .runner
            .reclaim(session_core::jobs::Liveness::OnlyInstance)
            .unwrap(),
        vec!["s".to_string()]
    );
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
async fn a_recording_during_the_transcribe_stage_leaves_the_other_note_its_own_model() {
    let store = store();
    let root = tempfile::tempdir().unwrap();
    let capturing = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&capturing);
    let script: Arc<Script> = Arc::default();
    with_summaries(&script).await;
    let dest = tempfile::tempdir().unwrap();
    *script.destination.lock().unwrap() = Some(dest.path().to_path_buf());
    // A recording starts after the first window.
    *script.capture_after.lock().unwrap() = Some((1, Arc::clone(&capturing)));
    let runner = Runner::new(Environment {
        store: Arc::clone(&store),
        working_root: root.path().to_path_buf(),
        engines: Box::new(Provider(Arc::clone(&script))),
        suspended: Box::new(move || flag.load(Ordering::SeqCst)),
        events: Box::new(|_| {}),
    });
    queue_session(
        &store,
        root.path(),
        "s",
        100,
        Some("batch"),
        &[(16_000, conversation())],
    );

    // The live worker takes the slot while the stage waits out the
    // recording, and loads the recording's model into it (KTD9).
    let access = runner.engine_access();
    let recording = Arc::clone(&capturing);
    let worker = std::thread::spawn(move || {
        while !recording.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let held = access.hold("live").unwrap().expect("not shut down");
        assert_eq!(held.as_ref().unwrap().model_id(), "live");
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(held);
        // Stop: the worker saved its last part and released the slot.
        recording.store(false, Ordering::SeqCst);
    });

    let (_, outcome) = runner.run_next().await.unwrap();
    worker.join().unwrap();
    assert_eq!(outcome, Outcome::Finished);
    let called = script.models_called.lock().unwrap().clone();
    assert!(called.len() >= 2, "{called:?}");
    assert!(called.iter().all(|m| m == "batch"), "{called:?}");
    assert_eq!(
        *script.loads.lock().unwrap(),
        vec!["batch".to_string(), "live".into(), "batch".into()]
    );
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
    let transcript = std::fs::read_to_string(saved_folder(&r, "s").join(TRANSCRIPT_FILE)).unwrap();
    assert!(transcript.contains("call 0"), "{transcript}");
    // The join records the number of speakers it found (KTD11).
    let parsed = session_core::folder::Transcript::parse(&transcript).unwrap();
    let found = r.store.lock().unwrap().found_speakers("s").unwrap();
    assert_eq!(found, pipeline::join::found_speakers(&parsed));
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
    let transcript = std::fs::read_to_string(saved_folder(&r, "s").join(TRANSCRIPT_FILE)).unwrap();
    assert!(transcript.contains("call 0"), "{transcript}");
    // No speaker marks (R20a): no found speaker count.
    assert_eq!(r.store.lock().unwrap().found_speakers("s").unwrap(), None);
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

    // The weights finish downloading: it picks up after assembly.
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
    let transcript = std::fs::read_to_string(saved_folder(&r, "s").join(TRANSCRIPT_FILE)).unwrap();
    assert!(
        transcript.contains(&format!("call {windows}")),
        "{transcript}"
    );
}

// ------------------------------------------------------------- gap fill (U7)

#[tokio::test(flavor = "multi_thread")]
async fn a_streamed_session_is_saved_from_its_live_text_without_a_batch_window() {
    // Covers AE3 through the runner.
    let r = rig().await;
    let audio = speech();
    let duration_ms = audio.len() as u64 / 16;
    queue_session(&r.store, &r.root, "s", 100, Some("m"), &[(16_000, audio)]);
    // The live worker saved one part with a word in every second.
    let words = (0..=duration_ms / 1_000)
        .map(|s| asr::LiveWord {
            start_ms: s * 1_000,
            end_ms: s * 1_000 + 300,
            text: format!("live{s}"),
        })
        .collect();
    let part = asr::LivePart {
        start_ms: 0,
        end_ms: duration_ms,
        words,
    };
    pipeline::live::save_part(&r.store, "s", 0, 0, &part).unwrap();

    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    assert_eq!(r.script.calls.load(Ordering::SeqCst), 0);
    assert!(r.script.analyses.load(Ordering::SeqCst) > 0, "speakers ran");
    let transcript = std::fs::read_to_string(saved_folder(&r, "s").join(TRANSCRIPT_FILE)).unwrap();
    assert!(transcript.contains("live0 live1 live2"), "{transcript}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_waits_for_its_model_lets_the_next_ready_session_go_first() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "waits",
        100,
        Some("downloading"),
        &[(16_000, speech())],
    );
    queue_session(
        &r.store,
        &r.root,
        "ready",
        200,
        Some("m"),
        &[(16_000, speech())],
    );
    r.script.missing.lock().unwrap().push("downloading".into());

    let (id, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!((id.as_str(), outcome), ("ready", Outcome::Finished));
    // The waiting session keeps its place, and nothing else can start.
    assert!(matches!(
        state(&r.store, "waits"),
        SessionState::Queued { .. }
    ));
    assert!(r.runner.run_next().await.is_none());

    // The model finishes downloading: the waiting session processes.
    r.script.missing.lock().unwrap().clear();
    let (id, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!((id.as_str(), outcome), ("waits", Outcome::Finished));
    assert_eq!(
        *r.script.loads.lock().unwrap(),
        vec!["m".to_string(), "downloading".into()]
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

/// Whether a completion request is the title call.
fn is_title_call(body: &str) -> bool {
    user_content(body).contains("Propose a title")
}

fn completions(mock: &http::Mock) -> Vec<String> {
    mock.seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.path == "/v1/chat/completions")
        .map(|s| s.body.clone())
        .collect()
}

/// The summary calls, without the title call.
fn chat_calls(mock: &http::Mock) -> Vec<String> {
    completions(mock)
        .iter()
        .filter(|b| !is_title_call(b))
        .map(|b| user_content(b))
        .collect()
}

fn title_calls(mock: &http::Mock) -> Vec<String> {
    completions(mock)
        .iter()
        .filter(|b| is_title_call(b))
        .map(|b| user_content(b))
        .collect()
}

/// `reply` for the summary calls, and a server error for the title call.
fn without_titles(reply: http::Reply) -> http::Reply {
    let failed = || (500, r#"{"error":"no titles here"}"#.to_string());
    match reply {
        http::Reply::Json(status, body) => http::Reply::With(Arc::new(move |b: &str| {
            if is_title_call(b) {
                failed()
            } else {
                (status, body.clone())
            }
        })),
        http::Reply::With(f) => {
            http::Reply::With(Arc::new(
                move |b: &str| {
                    if is_title_call(b) { failed() } else { f(b) }
                },
            ))
        }
        other => other,
    }
}

/// Point the script's summaries at `reply`, with an optional context
/// setting. The title call fails.
async fn summaries_from(r: &Rig, reply: http::Reply, context_limit: Option<u32>) -> http::Mock {
    let mock = http::serve(std::collections::HashMap::from([(
        "/v1/chat/completions",
        without_titles(reply),
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
    let summary = std::fs::read_to_string(saved_folder(&r, "s").join("summary.md")).unwrap();
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
    for file in files_under(&saved_folder(&r, "s")) {
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
async fn a_server_error_on_part_five_saves_a_stub_and_regenerating_starts_there() {
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
    let path = saved_folder(&r, "s").join("summary.md");
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
    assert_eq!(state(&r.store, "s"), SessionState::Saved);
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
                    audio_intact: true,
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
    assert_eq!(path, saved_folder(&r, "s").join("summary.md"));
    let summary = std::fs::read_to_string(&path).unwrap();
    assert!(summary.contains("Version 1."), "{summary}");
    let names: Vec<String> = std::fs::read_dir(saved_folder(&r, "s"))
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
    let summary = std::fs::read_to_string(saved_folder(&r, "s").join("summary.md")).unwrap();
    assert!(summary.contains("We decided to meet on Friday instead."));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_endpoint_saves_the_session_with_a_stub_naming_the_setting() {
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
    let folder = saved_folder(&r, "s");
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

// -------------------------------------------------------------- saving (U11)

const NOTES: &str = "- [00:00:01] the rules changed\n- bring snacks\n";

fn add_notes(store: &SharedStore, id: &str) {
    let mut s = store.lock().unwrap();
    assert!(!pipeline::save::set_notes(&mut s, id, NOTES).unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_processed_session_is_saved_once_as_a_folder_that_parses_back() {
    use session_core::folder::{SessionDoc, Transcript};
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

    let folder = saved_folder(&r, "s");
    assert_eq!(folder.parent(), Some(r.dest.as_path()));
    assert_eq!(
        folder.file_name().unwrap().to_string_lossy(),
        session_save::naming::folder_name(common::at(0), None)
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
    // notes.md is the notes document byte for byte (R25).
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.md")).unwrap(),
        NOTES
    );
    let transcript =
        Transcript::parse(&std::fs::read_to_string(folder.join(TRANSCRIPT_FILE)).unwrap()).unwrap();
    assert!(!transcript.lines.is_empty());

    assert!(r.events.lock().unwrap().iter().any(|e| matches!(
        e,
        RunnerEvent::Saved { path, copied: false, .. } if *path == folder
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_set_while_recording_is_in_notes_md_after_the_save() {
    let r = rig().await;
    common::record_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    add_notes(&r.store, "s");
    common::stop_session(&r.store, "s", 100);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(
        std::fs::read_to_string(saved_folder(&r, "s").join("notes.md")).unwrap(),
        NOTES
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_set_after_the_save_rewrites_the_saved_file_whole() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    add_notes(&r.store, "s");
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let folder = saved_folder(&r, "s");

    let edited = "# Plan\n\n- bring more snacks\n";
    {
        let mut s = r.store.lock().unwrap();
        assert!(pipeline::save::set_notes(&mut s, "s", edited).unwrap());
        assert_eq!(s.notes("s").unwrap(), edited);
    }
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.md")).unwrap(),
        edited
    );
    let names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.iter().all(|n| !n.starts_with('.')), "{names:?}");
    assert_eq!(state(&r.store, "s"), SessionState::Saved);
}

/// A filesystem that sets the notes document at its first rename, as the
/// editor can while the save stage runs.
struct EditsWhileSaving {
    store: SharedStore,
    document: &'static str,
    edited: AtomicBool,
}

impl session_save::Fs for EditsWhileSaving {
    fn rename(&self, from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        self.edit();
        session_save::RealFs.rename(from, to)
    }
    fn rename_new(&self, from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        self.edit();
        session_save::RealFs.rename_new(from, to)
    }
}

impl EditsWhileSaving {
    fn edit(&self) {
        if !self.edited.swap(true, Ordering::SeqCst) {
            let mut s = self.store.lock().unwrap();
            // Not saved yet: there is no home folder to write.
            assert!(!pipeline::save::set_notes(&mut s, "s", self.document).unwrap());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_set_while_the_save_runs_is_in_notes_md_once() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    add_notes(&r.store, "s");
    // Stop just before the save, then run the save stage by hand with a
    // filesystem that edits the notes part-way through it.
    *r.script.destination.lock().unwrap() = None;
    r.runner.run_next().await.unwrap();
    r.store
        .lock()
        .unwrap()
        .apply("s", &session_core::Event::Retry)
        .unwrap();
    let segments = r.store.lock().unwrap().segments("s").unwrap();
    let edited = "- the rules changed twice\n- bring snacks\n";
    let fs = EditsWhileSaving {
        store: Arc::clone(&r.store),
        document: edited,
        edited: AtomicBool::new(false),
    };
    let saved = pipeline::save::run(
        &r.store,
        "s",
        &r.root.join("s"),
        &segments,
        Some(&r.dest),
        &fs,
        &pipeline::Unattended,
    )
    .unwrap();
    assert!(fs.edited.load(Ordering::SeqCst));
    assert_eq!(state(&r.store, "s"), SessionState::Saved);
    assert_eq!(
        std::fs::read_to_string(saved.path.join("notes.md")).unwrap(),
        edited
    );
}

#[test]
fn notes_for_an_unknown_session_are_refused_and_change_nothing() {
    let store = store();
    common::queue_session(
        &store,
        tempfile::tempdir().unwrap().path(),
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    let mut s = store.lock().unwrap();
    s.set_notes("s", NOTES).unwrap();
    let err = pipeline::save::set_notes(&mut s, "gone", "- lost\n").unwrap_err();
    assert!(err.to_string().contains("gone"), "{err}");
    assert!(s.session("gone").unwrap().is_none());
    assert_eq!(s.notes("s").unwrap(), NOTES);
}

#[test]
fn with_no_session_no_notes_are_accepted() {
    // The app keeps no notes outside a session (R27): every notes call
    // names a session, and an empty store has none to take them.
    let store = store();
    let mut s = store.lock().unwrap();
    assert!(pipeline::save::set_notes(&mut s, "", "- stray\n").is_err());
    assert!(pipeline::save::set_notes(&mut s, "s", "- stray\n").is_err());
    assert!(s.sessions().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_destination_the_session_waits_whole_and_is_saved_in_the_last_one_chosen() {
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
    add_notes(&r.store, "s");
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
    assert!(!pipeline::save::begun(&r.store.lock().unwrap(), "s").unwrap());

    // A destination that has gone missing waits the same way.
    let gone = r.dest.join("unplugged");
    *r.script.destination.lock().unwrap() = Some(gone.clone());
    assert_eq!(r.runner.retry_awaiting().unwrap(), vec!["s".to_string()]);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert!(
        matches!(&outcome, Outcome::AwaitingDestination { problem } if problem.contains("is missing")),
        "{outcome:?}"
    );

    // An edit made while waiting reaches the saved file, and the session
    // is saved in the destination chosen last (AE7).
    r.store
        .lock()
        .unwrap()
        .set_notes(
            "s",
            &NOTES.replace("the rules changed", "the rules changed twice"),
        )
        .unwrap();
    let last = r.dest.join("chosen-last");
    std::fs::create_dir(&last).unwrap();
    *r.script.destination.lock().unwrap() = Some(last.clone());
    r.runner.retry_awaiting().unwrap();
    let transcribed = r.script.calls.load(Ordering::SeqCst);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(r.script.calls.load(Ordering::SeqCst), transcribed);
    let folder = saved_folder(&r, "s");
    assert_eq!(folder.parent(), Some(last.as_path()));
    assert!(!gone.exists());
    let notes = std::fs::read_to_string(folder.join("notes.md")).unwrap();
    assert_eq!(notes.matches("the rules changed").count(), 1, "{notes}");
    assert!(notes.contains("the rules changed twice"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_taken_folder_name_gets_a_suffix() {
    let r = rig().await;
    std::fs::create_dir(
        r.dest
            .join(session_save::naming::folder_name(common::at(0), None)),
    )
    .unwrap();
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
        saved_folder(&r, "s").file_name().unwrap().to_string_lossy(),
        format!(
            "{}-2",
            session_save::naming::folder_name(common::at(0), None)
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unverified_transcript_is_not_saved() {
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
    let err = pipeline::save::run(
        &r.store,
        "s",
        &r.root.join("s"),
        &segments,
        Some(&r.dest),
        &session_save::RealFs,
        &pipeline::Unattended,
    )
    .unwrap_err();
    assert!(err.to_string().contains("not been verified"), "{err}");
    assert!(r.root.join("s/audio-recovery").exists());
    assert_eq!(std::fs::read_dir(&r.dest).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relaunch_after_the_move_finishes_the_save_without_redoing_any_stage() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    // Save into a missing folder to stop just before the move, then move
    // the session into staging by hand, as a killed process would leave it.
    *r.script.destination.lock().unwrap() = None;
    r.runner.run_next().await.unwrap();
    *r.script.destination.lock().unwrap() = Some(r.dest.clone());
    let segments = r.store.lock().unwrap().segments("s").unwrap();
    {
        let mut s = r.store.lock().unwrap();
        s.apply("s", &session_core::Event::Retry).unwrap();
    }
    let job = session_save::Job {
        session_id: "s",
        working: &r.root.join("s"),
        destination: &r.dest,
        folder_name: &session_save::naming::folder_name(segments[0].wall_start, None),
    };
    let staged = session_save::step(
        &session_save::SaveState::Working,
        &job,
        &session_save::RealFs,
    )
    .unwrap();
    checkpoint::put(&r.store, "s", Stage::Save, pipeline::save::STATE, &staged).unwrap();
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
    assert_eq!(started, vec![Stage::Save]);
    assert!(saved_folder(&r, "s").join("audio/segment-01.opus").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relaunch_after_the_final_rename_is_recorded_finishes_the_first_save() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    // Stop just before the move, then take the save as far as the final
    // name by hand and record it, as a process killed before `Done` leaves it.
    *r.script.destination.lock().unwrap() = None;
    r.runner.run_next().await.unwrap();
    *r.script.destination.lock().unwrap() = Some(r.dest.clone());
    let segments = r.store.lock().unwrap().segments("s").unwrap();
    {
        let mut s = r.store.lock().unwrap();
        s.apply("s", &session_core::Event::Retry).unwrap();
    }
    let job = session_save::Job {
        session_id: "s",
        working: &r.root.join("s"),
        destination: &r.dest,
        folder_name: &session_save::naming::folder_name(segments[0].wall_start, None),
    };
    let staged = session_save::step(
        &session_save::SaveState::Working,
        &job,
        &session_save::RealFs,
    )
    .unwrap();
    let saved_state = session_save::step(&staged, &job, &session_save::RealFs).unwrap();
    let session_save::SaveState::Saved { path, .. } = &saved_state else {
        panic!("{saved_state:?}");
    };
    r.store
        .lock()
        .unwrap()
        .set_home_path("s", &path.to_string_lossy())
        .unwrap();
    checkpoint::put(
        &r.store,
        "s",
        Stage::Save,
        pipeline::save::STATE,
        &saved_state,
    )
    .unwrap();
    {
        let mut s = r.store.lock().unwrap();
        s.apply("s", &session_core::Event::ClaimLost).unwrap();
    }

    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let home = saved_folder(&r, "s");
    assert!(home.join("audio/segment-01.opus").exists());
    assert!(!home.join(session_save::MARKER).exists());
}

// ------------------------------------------------------- folder names (U7)

/// A filesystem whose first final rename fails: before the rename, or after
/// it, as a process killed before it records `Saved` leaves it.
struct FailsFinalRename {
    after_the_rename: bool,
    failed: AtomicBool,
}

impl session_save::Fs for FailsFinalRename {
    fn rename(&self, from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        session_save::RealFs.rename(from, to)
    }
    fn rename_new(&self, from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        if self.failed.swap(true, Ordering::SeqCst) {
            return session_save::RealFs.rename_new(from, to);
        }
        if self.after_the_rename {
            session_save::RealFs.rename_new(from, to)?;
        }
        Err(std::io::Error::other("the process stopped"))
    }
}

fn set_title(store: &SharedStore, id: &str, title: &str) {
    store
        .lock()
        .unwrap()
        .set_title(id, title, session_core::TitleSource::User)
        .unwrap();
}

/// Run a titled session up to the save, then run the save stage by hand
/// with `fs`, which fails it at the final rename. The session goes back to
/// the queue.
async fn save_fails_at_the_final_rename(r: &Rig, after_the_rename: bool) {
    stage_then_fail_the_final_rename(r, after_the_rename).await;
    r.store
        .lock()
        .unwrap()
        .apply("s", &session_core::Event::ClaimLost)
        .unwrap();
}

/// [`save_fails_at_the_final_rename`] without the last step: the session
/// stays in processing with the `Staged` save state.
async fn stage_then_fail_the_final_rename(r: &Rig, after_the_rename: bool) {
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    set_title(&r.store, "s", "First");
    *r.script.destination.lock().unwrap() = None;
    r.runner.run_next().await.unwrap();
    *r.script.destination.lock().unwrap() = Some(r.dest.clone());
    r.store
        .lock()
        .unwrap()
        .apply("s", &session_core::Event::Retry)
        .unwrap();
    let segments = r.store.lock().unwrap().segments("s").unwrap();
    let fs = FailsFinalRename {
        after_the_rename,
        failed: AtomicBool::new(false),
    };
    pipeline::save::run(
        &r.store,
        "s",
        &r.root.join("s"),
        &segments,
        Some(&r.dest),
        &fs,
        &pipeline::Unattended,
    )
    .unwrap_err();
    assert!(fs.failed.load(Ordering::SeqCst));
    assert!(matches!(
        checkpoint::get::<session_save::SaveState>(
            &r.store,
            "s",
            Stage::Save,
            pipeline::save::STATE
        )
        .unwrap(),
        Some(session_save::SaveState::Staged { .. })
    ));
}

fn destination_entries(r: &Rig) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(&r.dest)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn a_titled_session_is_saved_under_its_title() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    set_title(&r.store, "s", "Budget: Q3 / review?");
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(
        destination_entries(&r),
        [session_save::naming::folder_name(
            common::at(0),
            Some("Budget Q3 review")
        )]
    );
}

/// Where the save stage put the folder, before any rename after `Done`.
fn saved_at(r: &Rig) -> std::path::PathBuf {
    match checkpoint::get::<session_save::SaveState>(
        &r.store,
        "s",
        Stage::Save,
        pipeline::save::STATE,
    )
    .unwrap()
    {
        Some(session_save::SaveState::Done { path }) => path,
        other => panic!("the save is not done: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_title_changed_between_two_save_runs_saves_under_the_first_name_then_renames() {
    let r = rig().await;
    save_fails_at_the_final_rename(&r, false).await;
    set_title(&r.store, "s", "Second");

    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let first = session_save::naming::folder_name(common::at(0), Some("First"));
    let second = session_save::naming::folder_name(common::at(0), Some("Second"));
    assert_eq!(saved_at(&r), r.dest.join(&first));
    // The reconcile after Done gives the folder the new title's name (KTD8).
    assert_eq!(destination_entries(&r), [second.as_str()]);
    assert_eq!(saved_folder(&r, "s"), r.dest.join(&second));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_final_rename_whose_record_was_lost_is_found_by_the_recorded_name() {
    let r = rig().await;
    save_fails_at_the_final_rename(&r, true).await;
    // The rename happened: the folder has the first name, and a new title
    // must not send the scan for the marker to another name.
    let first = session_save::naming::folder_name(common::at(0), Some("First"));
    assert!(r.dest.join(&first).join(session_save::MARKER).exists());
    set_title(&r.store, "s", "Second");

    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(saved_at(&r), r.dest.join(&first));
    let second = session_save::naming::folder_name(common::at(0), Some("Second"));
    assert_eq!(destination_entries(&r), [second.as_str()]);
    let home = saved_folder(&r, "s");
    assert_eq!(home, r.dest.join(&second));
    assert!(home.join("audio/segment-01.opus").exists());
    assert!(!home.join(session_save::MARKER).exists());
    assert!(!r.root.join("s").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_save_begun_with_no_recorded_name_finishes_under_the_no_title_name() {
    // A save that began before names were recorded used the no-title name.
    // Its final rename happened and its record was lost; the session has a
    // title now.
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    *r.script.destination.lock().unwrap() = None;
    r.runner.run_next().await.unwrap();
    *r.script.destination.lock().unwrap() = Some(r.dest.clone());
    r.store
        .lock()
        .unwrap()
        .apply("s", &session_core::Event::Retry)
        .unwrap();
    let segments = r.store.lock().unwrap().segments("s").unwrap();
    let old = session_save::naming::folder_name(segments[0].wall_start, None);
    let job = session_save::Job {
        session_id: "s",
        working: &r.root.join("s"),
        destination: &r.dest,
        folder_name: &old,
    };
    let staged = session_save::step(
        &session_save::SaveState::Working,
        &job,
        &session_save::RealFs,
    )
    .unwrap();
    session_save::step(&staged, &job, &session_save::RealFs).unwrap();
    checkpoint::put(&r.store, "s", Stage::Save, pipeline::save::STATE, &staged).unwrap();
    r.store
        .lock()
        .unwrap()
        .apply("s", &session_core::Event::ClaimLost)
        .unwrap();
    set_title(&r.store, "s", "Later title");

    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(saved_at(&r), r.dest.join(&old));
    // The reconcile after Done then gives the folder the title's name.
    let titled = session_save::naming::folder_name(segments[0].wall_start, Some("Later title"));
    assert_eq!(destination_entries(&r), [titled.as_str()]);
    assert_eq!(saved_folder(&r, "s"), r.dest.join(&titled));
}

// ------------------------------------------------------------- progress (U12)

#[tokio::test(flavor = "multi_thread")]
async fn a_view_opened_later_rebuilds_progress_and_the_failure_from_the_store() {
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
    assert!(matches!(
        outcome,
        Outcome::Failed {
            stage: Stage::Verify,
            ..
        }
    ));

    let snapshot = pipeline::progress::snapshot(&r.store, "s").unwrap();
    assert_eq!(
        snapshot.done,
        vec![
            Stage::Assemble,
            Stage::Diarize,
            Stage::Transcribe,
            Stage::Join
        ]
    );
    assert_eq!(snapshot.current, Some(Stage::Verify));
    let windows = plan(&r.store, "s").windows.len();
    assert_eq!(snapshot.windows, Some((windows, windows)));
    let message = pipeline::progress::error(&r.store, "s", Stage::Verify)
        .unwrap()
        .unwrap();
    assert!(message.contains("misses speech"), "{message}");
}

/// Resume a completed session and record one more segment into it (R6d).
fn resume_completed(r: &Rig, id: &str, audio: &[f32]) {
    let mut s = r.store.lock().unwrap();
    s.apply(
        id,
        &session_core::Event::UserResume {
            space_ok: true,
            audio_intact: true,
        },
    )
    .unwrap();
    let last = s.segments(id).unwrap().last().unwrap().clone();
    let segment = common::write_segment(
        &r.root.join(id),
        last.index + 1,
        last.end_ms() + 86_400_000,
        16_000,
        audio,
        false,
    );
    s.put_segment(id, &segment).unwrap();
    s.apply(
        id,
        &session_core::Event::UserStop {
            at: common::at(90_000),
            retained_audio: true,
        },
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resumed_completed_session_updates_its_folder_in_place() {
    use session_core::folder::{SessionDoc, Transcript, TranscriptLine};
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech()), (16_000, speech())],
    );
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let folder = saved_folder(&r, "s");
    let before: Vec<Vec<u8>> = ["audio/segment-01.opus", "audio/segment-02.opus"]
        .iter()
        .map(|f| std::fs::read(folder.join(f)).unwrap())
        .collect();
    let calls = r.script.calls.load(Ordering::SeqCst);

    // A day later.
    resume_completed(&r, "s", &speech());
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);

    assert_eq!(saved_folder(&r, "s"), folder, "the same folder");
    assert_eq!(std::fs::read_dir(&r.dest).unwrap().count(), 1);
    assert!(!r.root.join("s").exists(), "the working copy is gone");
    for (file, bytes) in ["audio/segment-01.opus", "audio/segment-02.opus"]
        .iter()
        .zip(&before)
    {
        assert_eq!(
            &std::fs::read(folder.join(file)).unwrap(),
            bytes,
            "{file} rewritten"
        );
    }
    assert!(folder.join("audio/segment-03.opus").exists());
    let hidden: Vec<_> = std::fs::read_dir(&folder)
        .unwrap()
        .chain(std::fs::read_dir(folder.join("audio")).unwrap())
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(hidden.is_empty(), "{hidden:?}");

    let doc =
        SessionDoc::parse(&std::fs::read_to_string(folder.join("session.md")).unwrap()).unwrap();
    assert_eq!(doc.timeline_segments().len(), 3);
    let transcript =
        Transcript::parse(&std::fs::read_to_string(folder.join(TRANSCRIPT_FILE)).unwrap()).unwrap();
    let gaps = transcript
        .lines
        .iter()
        .filter(|l| matches!(l, TranscriptLine::Gap { .. }))
        .count();
    assert_eq!(gaps, 2, "both recordings and the day between them");
    // Only the new segment was transcribed again.
    let plan: WindowPlan =
        checkpoint::get(&r.store, "s", Stage::Transcribe, &checkpoint::plan_key(2))
            .unwrap()
            .unwrap();
    assert_eq!(
        r.script.calls.load(Ordering::SeqCst) - calls,
        plan.windows.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resumed_session_whose_folder_went_missing_names_it_and_retries_once_it_is_back() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let folder = saved_folder(&r, "s");

    resume_completed(&r, "s", &speech());
    let moved = r.dest.join("moved");
    std::fs::rename(&folder, &moved).unwrap();
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert!(matches!(outcome, Outcome::Failed { .. }), "{outcome:?}");
    let message = pipeline::progress::error(&r.store, "s", Stage::Assemble)
        .unwrap()
        .unwrap();
    assert!(message.contains(&folder.display().to_string()), "{message}");
    assert!(r.root.join("s").exists(), "the new recording is kept");

    std::fs::rename(&moved, &folder).unwrap();
    r.runner.retry("s").unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert!(folder.join("audio/segment-02.opus").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_frees_the_engine_and_nothing_loads_it_again() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "a",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(
        r.script.unloads.load(Ordering::SeqCst),
        0,
        "kept across sessions"
    );

    r.runner.shutdown();
    assert_eq!(r.script.unloads.load(Ordering::SeqCst), 1);
    queue_session(
        &r.store,
        &r.root,
        "b",
        200,
        Some("m"),
        &[(16_000, speech())],
    );
    assert!(
        r.runner.run_next().await.is_none(),
        "nothing starts after shutdown"
    );
    assert_eq!(r.script.loads.lock().unwrap().len(), 1);
}

/// A process that runs one stage of `id` and is killed as the next starts:
/// its events cancel the session at the second stage it begins (U21).
fn one_stage_process(
    store: &SharedStore,
    root: &std::path::Path,
    dest: &std::path::Path,
    id: &'static str,
) -> (
    Arc<Runner>,
    Arc<Script>,
    Arc<std::sync::OnceLock<Arc<Runner>>>,
) {
    let script: Arc<Script> = Arc::default();
    *script.destination.lock().unwrap() = Some(dest.to_path_buf());
    let this: Arc<std::sync::OnceLock<Arc<Runner>>> = Arc::default();
    let killer = Arc::clone(&this);
    let started = AtomicUsize::new(0);
    let runner = Runner::new(Environment {
        store: Arc::clone(store),
        working_root: root.to_path_buf(),
        engines: Box::new(Provider(Arc::clone(&script))),
        suspended: Box::new(|| false),
        events: Box::new(move |e| {
            if matches!(e, RunnerEvent::StageStarted { .. })
                && started.fetch_add(1, Ordering::SeqCst) == 1
                && let Some(runner) = killer.get()
            {
                runner.cancel(id);
            }
        }),
    });
    let _ = this.set(Arc::clone(&runner));
    (runner, script, this)
}

#[tokio::test(flavor = "multi_thread")]
async fn killed_after_every_stage_the_session_is_saved_once_with_the_same_transcript() {
    // The uninterrupted run to compare against.
    let clean = rig().await;
    queue_session(
        &clean.store,
        &clean.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    assert_eq!(clean.runner.run_next().await.unwrap().1, Outcome::Finished);
    let expected =
        std::fs::read_to_string(saved_folder(&clean, "s").join(TRANSCRIPT_FILE)).unwrap();

    let root = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    queue_session(
        &store,
        root.path(),
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    let mut processes = 0;
    loop {
        processes += 1;
        assert!(processes <= 12, "the session never finished");
        let (runner, script, _) = one_stage_process(&store, root.path(), dest.path(), "s");
        let _summaries = with_summaries(&script).await;
        runner
            .reclaim(session_core::jobs::Liveness::OnlyInstance)
            .unwrap();
        let (_, outcome) = runner.run_next().await.unwrap();
        if outcome == Outcome::Finished {
            break;
        }
    }

    // One process per stage, so every stage boundary saw a kill.
    assert!(processes >= 5, "{processes} processes");
    let folders: Vec<_> = std::fs::read_dir(dest.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(folders.len(), 1, "{folders:?}");
    let record = store.lock().unwrap().session("s").unwrap().unwrap();
    assert_eq!(record.state, SessionState::Saved);
    let folder = std::path::PathBuf::from(record.home_path.unwrap());
    let transcript = std::fs::read_to_string(folder.join(TRANSCRIPT_FILE)).unwrap();
    assert_eq!(transcript, expected);
    assert!(!root.path().join("s").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_counts_as_regenerating_only_while_its_summary_is_regenerated() {
    let r = rig().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let runner = Arc::clone(&r.runner);
    let record = Arc::clone(&seen);
    let reply = http::Reply::With(Arc::new(move |_| {
        record.lock().unwrap().push(runner.is_regenerating("s"));
        (200, http::completion("A summary."))
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
    // The first summary is part of processing, not a regeneration.
    assert!(seen.lock().unwrap().iter().all(|s| !s));
    seen.lock().unwrap().clear();
    r.runner.regenerate_summary("s").await.unwrap();
    let during = seen.lock().unwrap().clone();
    assert!(
        !during.is_empty() && during.iter().all(|s| *s),
        "{during:?}"
    );
    assert!(!r.runner.is_regenerating("s"));
    assert!(!r.runner.is_regenerating("other"));
}

// ------------------------------------------------------------- folder rename (U8)

use pipeline::folder_rename::{self, FolderRename};
use session_core::RenameIntent;

/// A session processed and saved, titled `title` before processing.
async fn saved_session(r: &Rig, title: Option<&str>) -> std::path::PathBuf {
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    if let Some(title) = title {
        set_title(&r.store, "s", title);
    }
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    saved_folder(r, "s")
}

fn name_for(title: Option<&str>) -> String {
    session_save::naming::folder_name(common::at(0), title)
}

fn document_title(folder: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(folder.join(pipeline::save::SESSION_FILE)).unwrap();
    session_core::folder::SessionDoc::parse(&text)
        .unwrap()
        .title
}

fn intent(r: &Rig) -> Option<RenameIntent> {
    r.store.lock().unwrap().rename_intent("s").unwrap()
}

/// Record a move as the rename does before it takes it.
fn record_intent(r: &Rig, from: &std::path::Path, to: &std::path::Path, temp: Option<&str>) {
    r.store
        .lock()
        .unwrap()
        .set_rename_intent(
            "s",
            &RenameIntent {
                from: from.to_string_lossy().into_owned(),
                to: to.to_string_lossy().into_owned(),
                temp: temp.map(|t| r.dest.join(t).to_string_lossy().into_owned()),
            },
        )
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_saved_session_renamed_has_one_folder_under_the_new_name() {
    // Covers AE2.
    let r = rig().await;
    let old = saved_session(&r, None).await;
    assert_eq!(old, r.dest.join(name_for(None)));

    let renamed = r.runner.rename_session("s", "Budget review").unwrap();
    let new = r.dest.join(name_for(Some("Budget review")));
    assert_eq!(renamed, FolderRename::Moved(new.clone()));
    assert_eq!(destination_entries(&r), [name_for(Some("Budget review"))]);
    assert_eq!(saved_folder(&r, "s"), new);
    assert_eq!(document_title(&new).as_deref(), Some("Budget review"));
    assert!(new.join("audio/segment-01.opus").exists());
    assert_eq!(intent(&r), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rename_during_processing_changes_the_title_at_once_and_the_folder_after_done() {
    // Covers AE1.
    let r = rig().await;
    save_fails_at_the_final_rename(&r, false).await;
    assert_eq!(
        r.runner.rename_session("s", "Second").unwrap(),
        FolderRename::Later
    );
    assert_eq!(
        r.store.lock().unwrap().title("s").unwrap().unwrap().text,
        "Second"
    );

    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let second = r.dest.join(name_for(Some("Second")));
    assert_eq!(destination_entries(&r), [name_for(Some("Second"))]);
    assert_eq!(saved_folder(&r, "s"), second);
    assert_eq!(document_title(&second).as_deref(), Some("Second"));
    // The save itself used the name it recorded when it began.
    assert_eq!(
        checkpoint::get::<session_save::SaveState>(
            &r.store,
            "s",
            Stage::Save,
            pipeline::save::STATE
        )
        .unwrap(),
        Some(session_save::SaveState::Done {
            path: r.dest.join(name_for(Some("First")))
        })
    );
    assert!(r.events.lock().unwrap().iter().any(|e| matches!(
        e,
        RunnerEvent::Saved { path, .. } if *path == second
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_after_the_intent_and_before_the_rename_resolves_at_launch_to_the_old_path() {
    let r = rig().await;
    let old = saved_session(&r, None).await;
    set_title(&r.store, "s", "Budget review");
    let new = r.dest.join(name_for(Some("Budget review")));
    record_intent(&r, &old, &new, None);

    assert_eq!(
        folder_rename::resolve_intents(&r.store, &session_save::RealFs).unwrap(),
        ["s"]
    );
    assert_eq!(saved_folder(&r, "s"), old);
    assert_eq!(intent(&r), None);
    assert_eq!(destination_entries(&r), [name_for(None)]);

    // The reconcile at launch then completes the rename.
    folder_rename::recover(&r.store, &session_save::RealFs).unwrap();
    assert_eq!(saved_folder(&r, "s"), new);
    assert_eq!(destination_entries(&r), [name_for(Some("Budget review"))]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_after_the_rename_and_before_home_path_resolves_at_launch_to_the_new_path() {
    let r = rig().await;
    let old = saved_session(&r, None).await;
    set_title(&r.store, "s", "Budget review");
    let new = r.dest.join(name_for(Some("Budget review")));
    record_intent(&r, &old, &new, None);
    std::fs::rename(&old, &new).unwrap();

    folder_rename::recover(&r.store, &session_save::RealFs).unwrap();
    assert_eq!(saved_folder(&r, "s"), new);
    assert_eq!(intent(&r), None);
    assert_eq!(document_title(&new).as_deref(), Some("Budget review"));
    assert_eq!(destination_entries(&r), [name_for(Some("Budget review"))]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_after_session_md_and_before_home_path_resolves_at_launch_to_the_new_path() {
    let r = rig().await;
    let old = saved_session(&r, None).await;
    set_title(&r.store, "s", "Budget review");
    let new = r.dest.join(name_for(Some("Budget review")));
    record_intent(&r, &old, &new, None);
    std::fs::rename(&old, &new).unwrap();
    let text = std::fs::read_to_string(new.join("session.md")).unwrap();
    let mut doc = session_core::folder::SessionDoc::parse(&text).unwrap();
    doc.title = Some("Budget review".into());
    std::fs::write(new.join("session.md"), doc.render()).unwrap();

    folder_rename::recover(&r.store, &session_save::RealFs).unwrap();
    assert_eq!(saved_folder(&r, "s"), new);
    assert_eq!(intent(&r), None);
    assert_eq!(document_title(&new).as_deref(), Some("Budget review"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_after_home_path_and_before_the_intent_is_cleared_keeps_the_new_path() {
    let r = rig().await;
    let old = saved_session(&r, None).await;
    let first = r.runner.rename_session("s", "Budget review").unwrap();
    let FolderRename::Moved(new) = first else {
        panic!("{first:?}");
    };
    // The record that the rename ended is lost.
    record_intent(&r, &old, &new, None);

    folder_rename::recover(&r.store, &session_save::RealFs).unwrap();
    assert_eq!(saved_folder(&r, "s"), new);
    assert_eq!(intent(&r), None);
    assert_eq!(destination_entries(&r), [name_for(Some("Budget review"))]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rename_to_a_name_another_folder_uses_takes_the_next_suffix() {
    let r = rig().await;
    saved_session(&r, None).await;
    let taken = name_for(Some("Budget review"));
    std::fs::create_dir(r.dest.join(&taken)).unwrap();

    let suffixed = format!("{taken}-2");
    assert_eq!(
        r.runner.rename_session("s", "Budget review").unwrap(),
        FolderRename::Moved(r.dest.join(&suffixed))
    );
    assert_eq!(destination_entries(&r), [taken.clone(), suffixed.clone()]);
    // The suffixed folder has its name: a launch does not move it again.
    folder_rename::recover(&r.store, &session_save::RealFs).unwrap();
    assert_eq!(saved_folder(&r, "s"), r.dest.join(&suffixed));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rename_that_changes_only_the_case_ends_with_the_new_case() {
    let r = rig().await;
    saved_session(&r, Some("budget review")).await;
    assert_eq!(destination_entries(&r), [name_for(Some("budget review"))]);

    let new = r.dest.join(name_for(Some("Budget review")));
    assert_eq!(
        r.runner.rename_session("s", "Budget review").unwrap(),
        FolderRename::Moved(new.clone())
    );
    assert_eq!(destination_entries(&r), [name_for(Some("Budget review"))]);
    assert_eq!(saved_folder(&r, "s"), new);
    assert_eq!(document_title(&new).as_deref(), Some("Budget review"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_of_case_stopped_at_the_temporary_name_resolves_at_launch_to_the_new_name() {
    let r = rig().await;
    let old = saved_session(&r, Some("budget review")).await;
    set_title(&r.store, "s", "Budget review");
    let new = r.dest.join(name_for(Some("Budget review")));
    let temp = session_save::rename_home::temp_name("s");
    record_intent(&r, &old, &new, Some(&temp));
    std::fs::rename(&old, r.dest.join(&temp)).unwrap();
    assert_eq!(destination_entries(&r), [temp]);

    folder_rename::recover(&r.store, &session_save::RealFs).unwrap();
    assert_eq!(destination_entries(&r), [name_for(Some("Budget review"))]);
    assert_eq!(saved_folder(&r, "s"), new);
    assert_eq!(intent(&r), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_after_done_and_before_a_deferred_rename_ends_renames_at_the_next_launch() {
    let r = rig().await;
    saved_session(&r, Some("First")).await;
    // The title changed while the session was busy, and the process
    // stopped before the reconcile after Done ran.
    set_title(&r.store, "s", "Second");

    folder_rename::recover(&r.store, &session_save::RealFs).unwrap();
    let second = r.dest.join(name_for(Some("Second")));
    assert_eq!(destination_entries(&r), [name_for(Some("Second"))]);
    assert_eq!(saved_folder(&r, "s"), second);
    assert_eq!(document_title(&second).as_deref(), Some("Second"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rename_during_a_regeneration_moves_the_folder_after_it() {
    let r = rig().await;
    let old = saved_session(&r, None).await;
    assert_eq!(
        folder_rename::rename_session(
            &r.store,
            "s",
            "Budget review",
            &|_| true,
            &session_save::RealFs
        )
        .unwrap(),
        FolderRename::Later
    );
    assert_eq!(saved_folder(&r, "s"), old);

    let summary = r.runner.regenerate_summary("s").await.unwrap();
    let new = r.dest.join(name_for(Some("Budget review")));
    assert_eq!(summary, new.join("summary.md"));
    assert!(summary.exists());
    assert_eq!(saved_folder(&r, "s"), new);
    assert_eq!(destination_entries(&r), [name_for(Some("Budget review"))]);
}

/// Refuses every rename as `PermissionDenied`, as Windows does while
/// another process holds a file in the folder.
struct AlwaysInUse(AtomicUsize);

impl session_save::Fs for AlwaysInUse {
    fn rename(&self, _: &std::path::Path, _: &std::path::Path) -> std::io::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
    }
    fn rename_new(&self, _: &std::path::Path, _: &std::path::Path) -> std::io::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_in_use_keeps_its_old_name_and_the_new_title() {
    let r = rig().await;
    let old = saved_session(&r, None).await;
    let fs = AlwaysInUse(AtomicUsize::new(0));
    let result =
        folder_rename::rename_session(&r.store, "s", "Budget review", &|_| false, &fs).unwrap();
    let FolderRename::Kept(message) = result else {
        panic!("{result:?}");
    };
    assert!(message.contains("in use"), "{message}");
    assert!(!message.contains("no free name"), "{message}");
    assert_eq!(
        fs.0.load(Ordering::SeqCst),
        1 + session_save::rename_home::IN_USE_RETRIES
    );
    assert_eq!(
        r.store.lock().unwrap().title("s").unwrap().unwrap().text,
        "Budget review"
    );
    assert_eq!(saved_folder(&r, "s"), old);
    assert_eq!(destination_entries(&r), [name_for(None)]);
    assert_eq!(intent(&r), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_rename_notes_and_a_resume_write_into_the_renamed_folder() {
    let r = rig().await;
    saved_session(&r, None).await;
    let FolderRename::Moved(new) = r.runner.rename_session("s", "Budget review").unwrap() else {
        panic!("not moved");
    };

    pipeline::save::set_notes(&mut r.store.lock().unwrap(), "s", "- after the rename\n").unwrap();
    let notes = std::fs::read_to_string(new.join(pipeline::save::NOTES_FILE)).unwrap();
    assert!(notes.contains("after the rename"), "{notes}");

    resume_completed(&r, "s", &speech());
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(saved_folder(&r, "s"), new);
    assert!(new.join("audio/segment-02.opus").exists());
    assert_eq!(destination_entries(&r), [name_for(Some("Budget review"))]);
    assert_eq!(document_title(&new).as_deref(), Some("Budget review"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_title_names_the_folder_by_its_start_time_again() {
    let r = rig().await;
    saved_session(&r, Some("Budget review")).await;
    let old = r.dest.join(name_for(None));
    assert_eq!(
        r.runner.rename_session("s", "  ").unwrap(),
        FolderRename::Moved(old.clone())
    );
    assert_eq!(r.store.lock().unwrap().title("s").unwrap(), None);
    assert_eq!(document_title(&old), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_title_change_that_keeps_the_folder_name_rewrites_session_md() {
    let r = rig().await;
    let folder = saved_session(&r, Some("Budget review")).await;
    assert_eq!(
        r.runner.rename_session("s", "Budget review?").unwrap(),
        FolderRename::Unchanged
    );
    assert_eq!(saved_folder(&r, "s"), folder);
    assert_eq!(document_title(&folder).as_deref(), Some("Budget review?"));
}

// ------------------------------------------------- notes and titles (U5)

use session_core::{Title, TitleSource};

/// The reply that the next title call gets, as a status and a body.
type TitleReply = Arc<Mutex<(u16, String)>>;

fn title_reply(text: &str) -> TitleReply {
    Arc::new(Mutex::new((200, http::completion(text))))
}

/// Summary calls answered by `summary`, and title calls by what `title`
/// holds when they arrive.
async fn summaries_and_titles(
    r: &Rig,
    summary: http::Responder,
    title: &TitleReply,
    context_limit: Option<u32>,
) -> http::Mock {
    let title = Arc::clone(title);
    let reply = http::Reply::With(Arc::new(move |b: &str| {
        if is_title_call(b) {
            title.lock().unwrap().clone()
        } else {
            summary(b)
        }
    }));
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

fn a_summary() -> http::Responder {
    Arc::new(|_| (200, http::completion("A short summary.")))
}

fn title_of(r: &Rig) -> Option<Title> {
    r.store.lock().unwrap().title("s").unwrap()
}

fn proposed(text: &str) -> Option<Title> {
    Some(Title {
        text: text.into(),
        source: TitleSource::Proposed,
    })
}

fn summary_is_complete(folder: &std::path::Path) -> bool {
    let text = std::fs::read_to_string(folder.join("summary.md")).unwrap();
    matches!(
        session_core::folder::Summary::parse(&text).unwrap().meta,
        session_core::folder::SummaryMeta::Complete { .. }
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proposed_title_is_cleaned_and_names_the_saved_folder() {
    let r = rig().await;
    let title = title_reply("  \"Budget Review.\"  ");
    let mock = summaries_and_titles(&r, a_summary(), &title, None).await;
    let folder = saved_session(&r, None).await;
    assert_eq!(title_calls(&mock).len(), 1);
    assert_eq!(title_of(&r), proposed("Budget Review"));
    assert_eq!(folder, r.dest.join(name_for(Some("Budget Review"))));
    assert_eq!(document_title(&folder).as_deref(), Some("Budget Review"));
    // The title call carries the summary.
    assert!(title_calls(&mock)[0].contains("A short summary."));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_proposed_title_is_cut_to_the_folder_name_length() {
    let r = rig().await;
    let long = "word ".repeat(100);
    let title = title_reply(&format!("{long}\nA second line"));
    summaries_and_titles(&r, a_summary(), &title, None).await;
    saved_session(&r, None).await;
    let stored = title_of(&r).unwrap().text;
    assert!(stored.chars().count() <= session_save::naming::MAX_TITLE_CHARS);
    assert!(stored.starts_with("word word"), "{stored}");
    assert!(!stored.contains("second"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_with_a_user_title_makes_no_title_call() {
    // Covers AE9.
    let r = rig().await;
    let title = title_reply("Something else");
    let mock = summaries_and_titles(&r, a_summary(), &title, None).await;
    let folder = saved_session(&r, Some("Budget review")).await;
    assert!(title_calls(&mock).is_empty());
    assert_eq!(chat_calls(&mock).len(), 1);
    assert_eq!(title_of(&r).unwrap().text, "Budget review");
    assert_eq!(folder, r.dest.join(name_for(Some("Budget review"))));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_title_call_leaves_no_title_and_the_summary_complete() {
    let r = rig().await;
    let title: TitleReply = Arc::new(Mutex::new((500, r#"{"error":"overloaded"}"#.into())));
    let mock = summaries_and_titles(&r, a_summary(), &title, None).await;
    let folder = saved_session(&r, None).await;
    assert_eq!(title_calls(&mock).len(), 1);
    assert_eq!(title_of(&r), None);
    assert_eq!(folder, r.dest.join(name_for(None)));
    assert!(summary_is_complete(&folder));
    assert!(
        !r.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, RunnerEvent::SummaryUnavailable { .. }))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_on_a_proposed_title_proposes_again_and_renames_the_folder() {
    // Covers AE4.
    let r = rig().await;
    let title = title_reply("First");
    let mock = summaries_and_titles(&r, a_summary(), &title, None).await;
    let old = saved_session(&r, None).await;
    assert_eq!(old, r.dest.join(name_for(Some("First"))));

    *title.lock().unwrap() = (200, http::completion("Second"));
    let summary = r.runner.regenerate_summary("s").await.unwrap();
    let new = r.dest.join(name_for(Some("Second")));
    assert_eq!(title_calls(&mock).len(), 2);
    assert_eq!(title_of(&r), proposed("Second"));
    assert_eq!(summary, new.join("summary.md"));
    assert_eq!(saved_folder(&r, "s"), new);
    assert_eq!(destination_entries(&r), [name_for(Some("Second"))]);
    assert_eq!(document_title(&new).as_deref(), Some("Second"));
    assert!(summary_is_complete(&new));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_on_a_user_title_keeps_the_title() {
    // Covers AE3.
    let r = rig().await;
    let title = title_reply("Proposed");
    let mock = summaries_and_titles(&r, a_summary(), &title, None).await;
    let folder = saved_session(&r, Some("Budget review")).await;
    r.runner.regenerate_summary("s").await.unwrap();
    assert!(title_calls(&mock).is_empty());
    assert_eq!(title_of(&r).unwrap().source, TitleSource::User);
    assert_eq!(title_of(&r).unwrap().text, "Budget review");
    assert_eq!(saved_folder(&r, "s"), folder);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_on_a_session_saved_with_no_title_proposes_one_and_renames_the_folder() {
    // R32: a session saved before titles has no title.
    let r = rig().await;
    let title: TitleReply = Arc::new(Mutex::new((500, "{}".into())));
    summaries_and_titles(&r, a_summary(), &title, None).await;
    let old = saved_session(&r, None).await;
    assert_eq!(old, r.dest.join(name_for(None)));

    *title.lock().unwrap() = (200, http::completion("Budget review"));
    r.runner.regenerate_summary("s").await.unwrap();
    let new = r.dest.join(name_for(Some("Budget review")));
    assert_eq!(title_of(&r), proposed("Budget review"));
    assert_eq!(saved_folder(&r, "s"), new);
    assert_eq!(destination_entries(&r), [name_for(Some("Budget review"))]);
    assert_eq!(document_title(&new).as_deref(), Some("Budget review"));
}

const DOCUMENT: &str = "# Plan\n\n- [00:00:01] the rules changed\n- **bring snacks**\n";

#[tokio::test(flavor = "multi_thread")]
async fn a_notes_edit_after_the_run_starts_reaches_no_request_of_that_run() {
    // R36, and each request carries the notes document once.
    let r = rig().await;
    let store = Arc::clone(&r.store);
    let summary: http::Responder = Arc::new(move |_| {
        let mut s = store.lock().unwrap();
        pipeline::save::set_notes(&mut s, "s", "an edit made later").unwrap();
        (200, http::completion("Notes on a part."))
    });
    let title = title_reply("Plan");
    let mock = summaries_and_titles(&r, summary, &title, Some(2_048)).await;
    long_session(&r);
    {
        let mut s = r.store.lock().unwrap();
        pipeline::save::set_notes(&mut s, "s", DOCUMENT).unwrap();
    }
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let calls = chat_calls(&mock);
    assert!(
        calls
            .iter()
            .filter(|c| c.contains("<transcript part="))
            .count()
            > 2
    );
    let titles = title_calls(&mock);
    assert_eq!(titles.len(), 1);
    for c in calls.iter().chain(&titles) {
        assert_eq!(c.matches(DOCUMENT).count(), 1, "{c}");
        assert!(!c.contains("an edit made later"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notes_document_longer_than_the_budget_is_cut_and_every_request_fits() {
    let r = rig().await;
    let title = title_reply("Plan");
    let mock = summaries_and_titles(&r, a_summary(), &title, Some(2_048)).await;
    long_session(&r);
    let long: String = (0..1_000)
        .map(|i| format!("- note {i} about something the user found important\n"))
        .collect();
    {
        let mut s = r.store.lock().unwrap();
        pipeline::save::set_notes(&mut s, "s", &long).unwrap();
    }
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let all = completions(&mock);
    assert!(all.len() > 2);
    for body in &all {
        let content = user_content(body);
        assert!(content.contains(summarize::prompts::NOTES_CUT), "{content}");
        assert!(
            summarize::tokens::estimate(&content) < 2_048,
            "{}",
            content.len()
        );
    }
}

// ------------------------------------------------------------- delete (U9)

/// A remover that removes the directory, as the Trash would move it away.
fn remove_now(path: &std::path::Path) -> Result<(), String> {
    std::fs::remove_dir_all(path).map_err(|e| e.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rename_while_a_delete_removes_the_folder_leaves_no_folder() {
    let r = rig().await;
    let home = saved_session(&r, None).await;
    let working = r.root.join("s");
    let mut renamed = None;
    let claims = r.runner.claims().clone();
    session_core::jobs::delete_session(&r.store, "s", &working, &claims, |path| {
        // The title commits after the delete read the home folder and
        // before the delete removes it.
        if path == home {
            renamed = Some(r.runner.rename_session("s", "Budget review").unwrap());
        }
        remove_now(path)
    })
    .unwrap();
    assert_eq!(renamed, Some(FolderRename::Later));
    assert!(
        destination_entries(&r).is_empty(),
        "{:?}",
        destination_entries(&r)
    );
    assert!(r.store.lock().unwrap().session("s").unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_while_a_delete_removes_the_folder_is_refused() {
    let r = rig().await;
    let home = saved_session(&r, None).await;
    let store = Arc::clone(&r.store);
    let runner = Arc::clone(&r.runner);
    let working = r.root.join("s");
    let handle = tokio::runtime::Handle::current();
    let regenerated = tokio::task::spawn_blocking(move || {
        let mut regenerated = None;
        let claims = runner.claims().clone();
        session_core::jobs::delete_session(&store, "s", &working, &claims, |path| {
            if path == home {
                regenerated = Some(handle.block_on(runner.regenerate_summary("s")));
            }
            remove_now(path)
        })
        .unwrap();
        regenerated
    })
    .await
    .unwrap();
    let regenerated = regenerated.expect("the remover ran on the home folder");
    assert!(regenerated.is_err(), "{regenerated:?}");
    assert!(
        destination_entries(&r).is_empty(),
        "{:?}",
        destination_entries(&r)
    );
    assert!(r.store.lock().unwrap().session("s").unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_session_whose_save_stopped_when_staged_removes_the_staged_copy() {
    let r = rig().await;
    stage_then_fail_the_final_rename(&r, false).await;
    r.store
        .lock()
        .unwrap()
        .apply(
            "s",
            &session_core::Event::SaveFailed("the process stopped".into()),
        )
        .unwrap();
    let Some(session_save::SaveState::Staged { staging, .. }) =
        checkpoint::get::<session_save::SaveState>(
            &r.store,
            "s",
            Stage::Save,
            pipeline::save::STATE,
        )
        .unwrap()
    else {
        panic!("the save stopped when staged");
    };
    assert!(staging.join("audio/segment-01.opus").exists());
    let claims = r.runner.claims().clone();
    session_core::jobs::delete_session(&r.store, "s", &r.root.join("s"), &claims, remove_now)
        .unwrap();
    assert!(!staging.exists());
    assert!(
        destination_entries(&r).is_empty(),
        "{:?}",
        destination_entries(&r)
    );
    assert!(r.store.lock().unwrap().session("s").unwrap().is_none());
}
