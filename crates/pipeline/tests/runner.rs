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
use models::recognition::{ExecutionSpec, ProcessingMode, RecognitionConfig, SpeakerSource};
use pipeline::join::TRANSCRIPT_FILE;
use pipeline::progress::{SpeakerProgress, SpeakerWork};
use pipeline::reprocess::{Readiness, Refusal};
use pipeline::runner::Outcome;
use pipeline::transcribe::{self, WindowPlan};
use pipeline::{
    EndpointProblem, EngineProvider, Environment, Runner, RunnerEvent, SharedStore,
    SummaryEndpoint, checkpoint,
};
use session_core::{SessionState, Stage};

type Hook = Box<dyn Fn(usize) + Send + Sync>;

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
    /// Called with the ordinal (0-based) of each speaker analysis, halfway
    /// through it.
    on_analysis: Mutex<Option<Hook>>,
    /// The attempt of the saved speaker progress at each start of the
    /// speaker stage.
    saved_at_start: Mutex<Vec<Option<String>>>,
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
    /// A capture is active: the runner is suspended.
    capturing: AtomicBool,
    /// The active summary prompt; unset is the "Topic summary" built-in.
    prompt: Mutex<Option<summarize::Prompt>>,
    /// Named sentences that the recognizer hears: when set, each window
    /// decodes the name of each sentence once for each time it finds the
    /// sentence in the window's audio (see [`heard`]).
    sentences_heard: Mutex<Vec<(String, Vec<f32>)>>,
}

/// The names of `sentences` in the order that `pcm` holds them, one for each
/// copy: a place where the normalized cross-correlation with the sentence is
/// at least 0.5 and the highest within 100 ms. An echo 250 ms late is a
/// second copy.
fn heard(pcm: &[f32], sentences: &[(String, Vec<f32>)]) -> Vec<String> {
    heard_at(pcm, sentences)
        .into_iter()
        .map(|(_, name)| name)
        .collect()
}

/// [`heard`] with the first sample of each copy in `pcm`.
fn heard_at(pcm: &[f32], sentences: &[(String, Vec<f32>)]) -> Vec<(usize, String)> {
    use realfft::RealFftPlanner;
    let mut found: Vec<(usize, String)> = Vec::new();
    for (name, sentence) in sentences {
        if pcm.len() < sentence.len() {
            continue;
        }
        let n = (pcm.len() + sentence.len()).next_power_of_two();
        let mut planner = RealFftPlanner::<f64>::new();
        let forward = planner.plan_fft_forward(n);
        let inverse = planner.plan_fft_inverse(n);
        let mut a: Vec<f64> = pcm.iter().map(|&s| f64::from(s)).collect();
        a.resize(n, 0.0);
        let mut b: Vec<f64> = sentence.iter().map(|&s| f64::from(s)).collect();
        b.resize(n, 0.0);
        let mut fa = forward.make_output_vec();
        let mut fb = forward.make_output_vec();
        forward.process(&mut a, &mut fa).unwrap();
        forward.process(&mut b, &mut fb).unwrap();
        for (x, y) in fa.iter_mut().zip(&fb) {
            *x *= y.conj();
        }
        let mut cross = inverse.make_output_vec();
        inverse.process(&mut fa, &mut cross).unwrap();
        let norm: f64 = sentence.iter().map(|&s| f64::from(s).powi(2)).sum();
        let mut window_energy = vec![0.0f64; pcm.len() - sentence.len() + 1];
        let mut sum: f64 = pcm[..sentence.len()]
            .iter()
            .map(|&s| f64::from(s).powi(2))
            .sum();
        for (i, e) in window_energy.iter_mut().enumerate() {
            *e = sum;
            if i + sentence.len() < pcm.len() {
                sum += f64::from(pcm[i + sentence.len()]).powi(2) - f64::from(pcm[i]).powi(2);
            }
        }
        let ncc: Vec<f64> = window_energy
            .iter()
            .enumerate()
            .map(|(i, &e)| cross[i] / n as f64 / (norm * e.max(1e-12)).sqrt())
            .collect();
        let near = 1_600;
        for i in 0..ncc.len() {
            let (lo, hi) = (i.saturating_sub(near), (i + near + 1).min(ncc.len()));
            if ncc[i] >= 0.5 && ncc[lo..hi].iter().all(|&v| v <= ncc[i]) {
                found.push((i, name.clone()));
            }
        }
    }
    found.sort();
    found
}

/// Analyses without finding anyone: the stage runs, no speaker turns come out.
struct FakeAnalyzer(Arc<Script>);

impl SpeakerAnalyzer for FakeAnalyzer {
    fn analyze(
        &mut self,
        audio: &mut dyn AudioSource,
        request: &DiarizeRequest<'_>,
    ) -> Result<Analysis, DiarizeError> {
        let ordinal = self.0.analyses.fetch_add(1, Ordering::SeqCst);
        if self.0.diarize_fails.load(Ordering::SeqCst) {
            return Err(DiarizeError::Format("segmentation exploded".into()));
        }
        if let Some(report) = request.on_progress {
            report(0.5);
        }
        if let Some(hook) = self.0.on_analysis.lock().unwrap().as_ref() {
            hook(ordinal);
        }
        // The real stack polls between windows.
        if request.is_cancelled.is_some_and(|stop| stop()) {
            return Err(DiarizeError::Format("stopped".into()));
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
        let sentences = s.sentences_heard.lock().unwrap();
        if !sentences.is_empty() {
            // One segment for the call number over the window, and one for
            // each sentence heard, at its time, so a gap fill keeps the
            // sentence words by their time (U10).
            let call = format!("call {n}.");
            let mut text = call.clone();
            let mut segments = vec![AsrSegment {
                start_ms: 0,
                end_ms: ms,
                text: call,
            }];
            let lengths: std::collections::HashMap<&str, u64> = sentences
                .iter()
                .map(|(name, pcm)| (name.as_str(), pcm.len() as u64 / 16))
                .collect();
            for (at, name) in heard_at(pcm, &sentences) {
                text.push(' ');
                text.push_str(&name);
                let start_ms = at as u64 / 16;
                segments.push(AsrSegment {
                    start_ms,
                    end_ms: start_ms + lengths[name.as_str()],
                    text: name,
                });
            }
            return Ok(WindowText { text, segments });
        }
        drop(sentences);
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

struct ExactFake(Box<dyn Recognizer>);
/// One speaker over the whole audio. Fails when `Script::diarize_fails` is set.
struct ExactAnalyzer(Arc<Script>);
impl SpeakerAnalyzer for ExactAnalyzer {
    fn analyze(
        &mut self,
        audio: &mut dyn AudioSource,
        _: &DiarizeRequest<'_>,
    ) -> Result<Analysis, DiarizeError> {
        if self.0.diarize_fails.load(Ordering::SeqCst) {
            return Err(DiarizeError::Format("segmentation exploded".into()));
        }
        use pyannote_local::segmentation::{
            FRAME_SIZE, WINDOW_SAMPLES, WindowActivity, frames_for_samples,
        };
        let total = audio.sample_count()?;
        let step = 59 * FRAME_SIZE;
        let windows = total.saturating_sub(WINDOW_SAMPLES).div_ceil(step) + 1;
        Ok(Analysis {
            total,
            step,
            activities: vec![
                WindowActivity {
                    frames: vec![[true, false, false]; frames_for_samples(WINDOW_SAMPLES)]
                };
                windows
            ],
            embeddings: (0..windows)
                .map(|window| diarize::WindowEmbedding {
                    window,
                    local_speaker: 0,
                    embedding: vec![1.0, 0.0, 0.0, 0.0],
                    anchor: true,
                })
                .collect(),
        })
    }
}
impl Recognizer for ExactFake {
    fn model_id(&self) -> &str {
        self.0.model_id()
    }
    fn native_token_timestamps(&self) -> bool {
        true
    }
    fn transcribe(&mut self, pcm: &[f32]) -> Result<WindowText, AsrError> {
        self.0.transcribe(pcm)
    }
    fn transcribe_tokens(&mut self, pcm: &[f32]) -> Result<Vec<asr::TimedToken>, AsrError> {
        Ok(self
            .0
            .transcribe(pcm)?
            .segments
            .into_iter()
            .map(|s| asr::TimedToken {
                text: s.text,
                start_ms: s.start_ms,
                end_ms: s.end_ms,
            })
            .collect())
    }
}

impl EngineProvider for Provider {
    fn analyzer_exact(
        &self,
        _: &models::recognition::ExecutionSpec,
    ) -> Result<Box<dyn SpeakerAnalyzer>, DiarizeError> {
        Ok(Box::new(ExactAnalyzer(Arc::clone(&self.0))))
    }
    fn ready_exact(&self, spec: &models::recognition::ExecutionSpec) -> bool {
        self.ready(&spec.config.asr_model)
    }
    fn load_exact(
        &self,
        spec: &models::recognition::ExecutionSpec,
    ) -> Result<Box<dyn Recognizer>, String> {
        Ok(Box::new(ExactFake(self.load(&spec.config.asr_model)?)))
    }
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
    fn summary_prompt(&self) -> summarize::Prompt {
        self.0.prompt.lock().unwrap().clone().unwrap_or_else(|| {
            summarize::prompts::built_in(summarize::prompts::TOPIC_SUMMARY_ID).unwrap()
        })
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
    let capturing = Arc::clone(&script);
    let recording = Arc::clone(&script);
    let saved = Arc::clone(&store);
    let runner = Runner::new(Environment {
        store: Arc::clone(&store),
        working_root: path.clone(),
        engines: Box::new(Provider(Arc::clone(&script))),
        suspended: Box::new(move || capturing.capturing.load(Ordering::SeqCst)),
        events: Box::new(move |e| {
            if let RunnerEvent::StageStarted {
                session_id,
                stage: Stage::Diarize,
            } = &e
            {
                let progress = pipeline::progress::speakers(&saved, session_id).unwrap();
                recording
                    .saved_at_start
                    .lock()
                    .unwrap()
                    .push(progress.map(|p| p.attempt));
            }
            sink.lock().unwrap().push(e)
        }),
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

/// The parsed `session.md` of a saved folder.
fn session_md(folder: &std::path::Path) -> session_core::folder::SessionDoc {
    let text = std::fs::read_to_string(folder.join(pipeline::save::SESSION_FILE)).unwrap();
    session_core::folder::SessionDoc::parse(&text).unwrap()
}

/// `session.md` records the fingerprints of the folder's `transcript.md`
/// and `summary.md` as they are now (KTD2).
fn assert_fingerprints_match(folder: &std::path::Path) {
    use session_core::folder::fingerprint;
    let doc = session_md(folder);
    let of = |name: &str| fingerprint(&std::fs::read(folder.join(name)).unwrap());
    assert_eq!(doc.transcript_fingerprint, Some(of(TRANSCRIPT_FILE)));
    assert_eq!(doc.summary_fingerprint, Some(of("summary.md")));
}

/// The store records the fingerprint of the `transcript.md` that this
/// computer wrote into the folder (KTD12).
fn assert_stored_fingerprint(r: &Rig, id: &str, folder: &std::path::Path) {
    use session_core::folder::fingerprint;
    let written = fingerprint(&std::fs::read(folder.join(TRANSCRIPT_FILE)).unwrap());
    let stored = r.store.lock().unwrap().transcript_fingerprint(id).unwrap();
    assert_eq!(stored, Some(written));
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
    assert!(!speaker_progress(&r).is_empty());
}

/// The speaker progress events, in order.
fn speaker_progress(r: &Rig) -> Vec<SpeakerProgress> {
    r.events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            RunnerEvent::SpeakerProgress { progress, .. } => Some(progress.clone()),
            _ => None,
        })
        .collect()
}

/// Audio longer than one speaker analysis window.
fn long_conversation() -> Vec<f32> {
    [conversation(), conversation(), conversation()].concat()
}

#[tokio::test(flavor = "multi_thread")]
async fn speaker_progress_covers_the_whole_note_and_stays_below_100() {
    // Covers AE2, AE4.
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech()), (16_000, long_conversation())],
    );
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);

    let seen = speaker_progress(&r);
    let attempt = &seen[0].attempt;
    assert!(seen.iter().all(|p| &p.attempt == attempt), "{seen:?}");
    // Active before the first measurement.
    assert_eq!(
        seen[0].work,
        SpeakerWork {
            percent: None,
            measuring: false
        }
    );
    let percents: Vec<u8> = seen.iter().filter_map(|p| p.work.percent).collect();
    assert!(
        percents.windows(2).all(|pair| pair[0] <= pair[1]),
        "the value never goes back at a segment boundary: {percents:?}"
    );
    // Each half of each segment is a measured step.
    let measured: Vec<u8> = seen
        .iter()
        .filter(|p| p.work.measuring)
        .filter_map(|p| p.work.percent)
        .collect();
    assert!(measured.len() >= 4, "{measured:?}");
    assert!(measured.iter().any(|p| (1..50).contains(p)), "{measured:?}");
    assert!(
        measured.iter().any(|p| (50..99).contains(p)),
        "{measured:?}"
    );
    assert!(percents.iter().all(|p| *p < 100), "{percents:?}");
    assert_eq!(
        seen.last().unwrap().work,
        SpeakerWork {
            percent: Some(99),
            measuring: false
        },
        "clustering and the writes are active work below 100"
    );
    // The old per-segment meter is gone.
    assert!(!r.events.lock().unwrap().iter().any(|e| matches!(
        e,
        RunnerEvent::Progress {
            stage: Stage::Diarize,
            ..
        }
    )));
    // A complete stage shows no meter.
    assert_eq!(
        pipeline::progress::snapshot(&r.store, "s")
            .unwrap()
            .speakers,
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_view_opened_during_the_speaker_stage_reads_its_progress() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, long_conversation()), (16_000, long_conversation())],
    );
    let read = Arc::new(Mutex::new(None));
    let (store, sink) = (Arc::clone(&r.store), Arc::clone(&read));
    *r.script.on_analysis.lock().unwrap() = Some(Box::new(move |ordinal| {
        if ordinal == 1 {
            *sink.lock().unwrap() = Some(pipeline::progress::snapshot(&store, "s").unwrap());
        }
    }));
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);

    let snapshot = read.lock().unwrap().take().unwrap();
    assert_eq!(snapshot.current, Some(Stage::Diarize));
    let speakers = snapshot.speakers.expect("the stage saved its progress");
    assert_eq!(speakers.attempt, speaker_progress(&r)[0].attempt);
    // The first segment is complete and the second is half done.
    assert_eq!(speakers.work.percent, Some(75));
    // A newer live update then advances it.
    assert!(
        speaker_progress(&r)
            .iter()
            .any(|p| p.work.percent > Some(75))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_attempt_replaces_speaker_progress_and_counts_saved_segments_first() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    let first = rig_with(Arc::clone(&store), root, Arc::default());
    with_summaries(&first.script).await;
    queue_session(
        &first.store,
        &first.root,
        "s",
        100,
        Some("m"),
        &[(16_000, long_conversation()), (16_000, long_conversation())],
    );
    let runner = Arc::clone(&first.runner);
    *first.script.on_analysis.lock().unwrap() = Some(Box::new(move |ordinal| {
        if ordinal == 1 {
            runner.cancel("s");
        }
    }));
    let (_, outcome) = first.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Cancelled);
    let earlier = speaker_progress(&first)[0].attempt.clone();
    assert!(
        pipeline::progress::speakers(&first.store, "s")
            .unwrap()
            .is_some()
    );

    // A new process: the old claim is reclaimed at launch.
    let Rig { _root, .. } = first;
    let second = rig_with(Arc::clone(&store), _root, Arc::default());
    with_summaries(&second.script).await;
    second
        .runner
        .reclaim(session_core::jobs::Liveness::OnlyInstance)
        .unwrap();
    let (_, outcome) = second.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    // Only the second segment is analysed again.
    assert_eq!(second.script.analyses.load(Ordering::SeqCst), 1);
    // The new attempt starts without the earlier attempt's saved progress.
    assert_eq!(*second.script.saved_at_start.lock().unwrap(), vec![None]);
    let seen = speaker_progress(&second);
    assert!(seen.iter().all(|p| p.attempt != earlier), "{seen:?}");
    // The saved first segment counts before the second one runs.
    let first_measured = seen.iter().find(|p| p.work.percent.is_some()).unwrap();
    assert_eq!(first_measured.work.percent, Some(50));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_speaker_stage_shows_no_meter_and_no_completion() {
    let r = rig().await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, speech())],
    );
    // The speaker stage stops, and the view reads the store after it.
    let store = Arc::clone(&r.store);
    *r.script.on_analysis.lock().unwrap() = Some(Box::new(move |_| {
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
    }));
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Cancelled);
    assert!(
        speaker_progress(&r)
            .iter()
            .all(|p| p.work.percent < Some(100))
    );
    assert_eq!(
        pipeline::progress::snapshot(&r.store, "s")
            .unwrap()
            .speakers,
        None
    );
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
    // No result copies and no transition file (R1).
    assert!(names.iter().all(|n| !n.starts_with('.')), "{names:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_md_holds_the_complete_record_after_a_save_and_a_regeneration() {
    use session_core::{Title, TitleSource};
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
    set_title(&r.store, "s", "Budget review");
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let folder = saved_folder(&r, "s");
    let record = r.store.lock().unwrap().session("s").unwrap().unwrap();
    let doc = session_md(&folder);
    assert_eq!(doc.asr_model.as_deref(), Some("m"));
    assert_eq!(doc.found_speakers, record.found_speakers);
    assert_eq!(
        doc.title_and_source(),
        Some(Title {
            text: "Budget review".into(),
            source: TitleSource::User
        })
    );
    assert_fingerprints_match(&folder);
    assert_stored_fingerprint(&r, "s", &folder);

    let before = read(&folder, "summary.md");
    r.runner.regenerate_summary("s").await.unwrap();
    let folder = saved_folder(&r, "s");
    assert_ne!(read(&folder, "summary.md"), before);
    assert_fingerprints_match(&folder);
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

// ------------------------------------------------------ summary prompts (U2)

/// The system message and the user message of each summary call, without
/// the title call.
fn chat_messages(mock: &http::Mock) -> Vec<(String, String)> {
    completions(mock)
        .iter()
        .filter(|b| !is_title_call(b))
        .map(|b| {
            let v: serde_json::Value = serde_json::from_str(b).unwrap();
            let content = |i: usize| {
                v["messages"][i]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            };
            (content(0), content(1))
        })
        .collect()
}

/// A saved custom prompt whose text holds `marker`.
fn custom_prompt(name: &str, marker: &str) -> summarize::Prompt {
    summarize::Prompt {
        id: format!("custom-{marker}"),
        name: name.into(),
        text: format!("Write the summary as a short list. {marker}"),
        title_text: String::new(),
    }
}

fn use_prompt(r: &Rig, prompt: &summarize::Prompt) {
    *r.script.prompt.lock().unwrap() = Some(prompt.clone());
}

/// The prompt name that the `summary.md` at `path` records.
fn prompt_name(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).unwrap();
    match session_core::folder::Summary::parse(&text).unwrap().meta {
        session_core::folder::SummaryMeta::Complete { prompt_name, .. } => prompt_name,
        session_core::folder::SummaryMeta::Stub { .. } => None,
    }
}

fn summarize_keys(r: &Rig) -> Vec<String> {
    r.store
        .lock()
        .unwrap()
        .checkpoints("s", Stage::Summarize)
        .unwrap()
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

/// A long session whose fifth part note fails, so it is saved with a stub.
async fn stub_after_part_five(r: &Rig) -> (http::Mock, std::path::PathBuf) {
    let n = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&n);
    let reply = http::Reply::With(Arc::new(move |_| {
        if count.fetch_add(1, Ordering::SeqCst) == 4 {
            (500, r#"{"error":"overloaded"}"#.into())
        } else {
            (200, http::completion("Notes on a part."))
        }
    }));
    let mock = summaries_from(r, reply, Some(2_048)).await;
    long_session(r);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(chat_calls(&mock).len(), 5);
    let path = saved_folder(r, "s").join("summary.md");
    assert_eq!(prompt_name(&path), None, "a stub records no prompt name");
    (mock, path)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_whole_transcript_run_sends_the_active_prompt_and_records_its_name() {
    let r = rig().await;
    let recap = custom_prompt("Team sync", "RECAP-MARKER");
    use_prompt(&r, &recap);
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
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let calls = chat_messages(&mock);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].0.contains("RECAP-MARKER"), "{}", calls[0].0);
    let path = saved_folder(&r, "s").join("summary.md");
    assert_eq!(prompt_name(&path), Some("Team sync".into()));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("prompt_name: Team sync"), "{text}");
    // The snapshot is saved next to the plan, and the whole call has its
    // own key, apart from the first part note's.
    let saved: summarize::Prompt =
        checkpoint::get(&r.store, "s", Stage::Summarize, pipeline::summarize::PROMPT)
            .unwrap()
            .unwrap();
    assert_eq!(saved, recap);
    let keys = summarize_keys(&r);
    assert!(
        keys.iter().any(|k| k == pipeline::summarize::WHOLE),
        "{keys:?}"
    );
    assert!(keys.iter().all(|k| k != "l00-00000"), "{keys:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chunked_run_at_the_floor_sends_the_prompt_text_only_in_the_last_merge() {
    let r = rig().await;
    let mock = summaries_from(
        &r,
        http::Reply::Json(200, http::completion("Notes on a part.")),
        Some(2_048),
    )
    .await;
    long_session(&r);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let topic = summarize::prompts::built_in(summarize::prompts::TOPIC_SUMMARY_ID).unwrap();
    let calls = chat_messages(&mock);
    assert!(calls.len() > 2, "{} calls", calls.len());
    let with_text: Vec<usize> = (0..calls.len())
        .filter(|&i| calls[i].0.contains(topic.text.trim()))
        .collect();
    assert_eq!(with_text, vec![calls.len() - 1]);
    assert!(
        calls[calls.len() - 1]
            .1
            .contains("Combine these into the summary of the whole session.")
    );
    for (system, user) in &calls {
        let cost = summarize::tokens::estimate_messages([system.as_str(), user.as_str()]);
        assert!(cost < 2_048, "{cost}");
    }
    let path = saved_folder(&r, "s").join("summary.md");
    assert_eq!(prompt_name(&path), Some("Topic summary".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_resumed_after_the_active_prompt_changes_keeps_its_first_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let store = store();
    let first = rig_with(Arc::clone(&store), root, Arc::default());
    use_prompt(&first, &custom_prompt("Team sync", "FIRST-MARKER"));
    // The process stops after two part notes.
    let runner = Arc::clone(&first.runner);
    let count = Arc::new(AtomicUsize::new(0));
    let reply = http::Reply::With(Arc::new(move |_| {
        if count.fetch_add(1, Ordering::SeqCst) == 1 {
            runner.cancel("s");
        }
        (200, http::completion("Notes on a part."))
    }));
    let first_mock = summaries_from(&first, reply, Some(2_048)).await;
    long_session(&first);
    assert_eq!(first.runner.run_next().await.unwrap().1, Outcome::Cancelled);
    assert_eq!(chat_calls(&first_mock).len(), 2);

    let Rig { _root, .. } = first;
    let second = rig_with(Arc::clone(&store), _root, Arc::default());
    use_prompt(&second, &custom_prompt("Business meeting", "SECOND-MARKER"));
    let mock = summaries_from(
        &second,
        http::Reply::Json(200, http::completion("Notes on a part.")),
        Some(2_048),
    )
    .await;
    assert_eq!(
        second
            .runner
            .reclaim(session_core::jobs::Liveness::OnlyInstance)
            .unwrap(),
        vec!["s".to_string()]
    );
    assert_eq!(second.runner.run_next().await.unwrap().1, Outcome::Finished);
    let calls = chat_messages(&mock);
    assert!(calls.iter().all(|(s, _)| !s.contains("SECOND-MARKER")));
    assert!(calls.iter().any(|(s, _)| s.contains("FIRST-MARKER")));
    // The two part notes made before the stop are not made again.
    for done in ["<transcript part=\"1\">", "<transcript part=\"2\">"] {
        assert!(calls.iter().all(|(_, u)| !u.contains(done)), "{done}");
    }
    let path = saved_folder(&second, "s").join("summary.md");
    assert_eq!(prompt_name(&path), Some("Team sync".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerating_a_complete_summary_uses_the_prompt_active_now_and_records_its_name() {
    let r = rig().await;
    let mock = summaries_from(
        &r,
        http::Reply::Json(200, http::completion("A summary.")),
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
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let path = saved_folder(&r, "s").join("summary.md");
    assert_eq!(prompt_name(&path), Some("Topic summary".into()));
    // A change of the active prompt leaves the saved summary as it is.
    use_prompt(&r, &custom_prompt("Team sync", "RECAP-MARKER"));
    assert_eq!(prompt_name(&path), Some("Topic summary".into()));
    assert_eq!(r.runner.regenerate_summary("s").await.unwrap(), path);
    let calls = chat_messages(&mock);
    assert_eq!(calls.len(), 2);
    assert!(calls[1].0.contains("RECAP-MARKER"));
    assert_eq!(prompt_name(&path), Some("Team sync".into()));
}

fn snapshot_of(prompt: &summarize::Prompt) -> session_core::PromptSnapshot {
    session_core::PromptSnapshot {
        id: prompt.id.clone(),
        name: prompt.name.clone(),
        text: prompt.text.clone(),
        title_text: prompt.title_text.clone(),
    }
}

/// "s" started with `started` as the active prompt, and the active prompt
/// is `now` when it runs, as after a change during the recording.
async fn note_started_with(
    r: &Rig,
    started: &summarize::Prompt,
    now: &summarize::Prompt,
) -> (http::Mock, std::path::PathBuf) {
    let mock = numbered_summaries(r).await;
    common::queue_session_with_prompt(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
        &snapshot_of(started),
    );
    use_prompt(r, now);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    (mock, saved_folder(r, "s").join("summary.md"))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_summary_uses_the_prompt_that_was_active_when_the_note_started() {
    // R20, KTD15: a change of the active prompt during the recording
    // applies from the next note.
    let r = rig().await;
    let started = summarize::Prompt {
        title_text: "Name the team.".into(),
        ..custom_prompt("Team sync", "FIRST-MARKER")
    };
    let now = custom_prompt("Business meeting", "SECOND-MARKER");
    let (mock, path) = note_started_with(&r, &started, &now).await;
    let calls = chat_messages(&mock);
    assert!(calls.iter().any(|(s, _)| s.contains("FIRST-MARKER")));
    assert!(calls.iter().all(|(s, _)| !s.contains("SECOND-MARKER")));
    assert_eq!(prompt_name(&path), Some("Team sync".into()));
    // The snapshot keeps its title text (KTD7).
    let saved: summarize::Prompt =
        checkpoint::get(&r.store, "s", Stage::Summarize, pipeline::summarize::PROMPT)
            .unwrap()
            .unwrap();
    assert_eq!(saved, started);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_from_before_the_update_with_no_snapshot_uses_the_active_prompt() {
    let r = rig().await;
    let mock = numbered_summaries(&r).await;
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    assert_eq!(r.store.lock().unwrap().prompt_snapshot("s").unwrap(), None);
    use_prompt(&r, &custom_prompt("Business meeting", "ACTIVE-MARKER"));
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert!(
        chat_messages(&mock)
            .iter()
            .any(|(s, _)| s.contains("ACTIVE-MARKER"))
    );
    let path = saved_folder(&r, "s").join("summary.md");
    assert_eq!(prompt_name(&path), Some("Business meeting".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn summarize_again_on_a_note_with_a_snapshot_uses_the_prompt_that_the_user_selects() {
    let r = rig().await;
    let started = custom_prompt("Team sync", "FIRST-MARKER");
    let (mock, path) = note_started_with(&r, &started, &started).await;
    let before = chat_messages(&mock).len();
    use_prompt(&r, &custom_prompt("Business meeting", "SELECTED-MARKER"));
    assert_eq!(r.runner.regenerate_summary("s").await.unwrap(), path);
    let calls = chat_messages(&mock);
    assert!(
        calls[before..]
            .iter()
            .any(|(s, _)| s.contains("SELECTED-MARKER"))
    );
    assert!(
        calls[before..]
            .iter()
            .all(|(s, _)| !s.contains("FIRST-MARKER"))
    );
    assert_eq!(prompt_name(&path), Some("Business meeting".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn transcribe_again_on_a_note_with_a_snapshot_uses_the_active_prompt() {
    // Transcribe Again is a later action of the user, as Summarize Again.
    let r = rig().await;
    let started = custom_prompt("Team sync", "FIRST-MARKER");
    let (mock, path) = note_started_with(&r, &started, &started).await;
    let before = chat_messages(&mock).len();
    use_prompt(&r, &custom_prompt("Business meeting", "ACTIVE-MARKER"));
    confirm_again(&r).unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let calls = chat_messages(&mock);
    assert!(
        calls[before..]
            .iter()
            .any(|(s, _)| s.contains("ACTIVE-MARKER"))
    );
    assert_eq!(prompt_name(&path), Some("Business meeting".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerating_a_stub_with_the_same_prompt_reuses_its_part_notes() {
    let r = rig().await;
    let (mock, path) = stub_after_part_five(&r).await;
    assert_eq!(r.runner.regenerate_summary("s").await.unwrap(), path);
    let calls = chat_calls(&mock);
    assert!(
        calls[5].contains("<transcript part=\"5\">"),
        "{}",
        &calls[5][..80]
    );
    assert_eq!(prompt_name(&path), Some("Topic summary".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerating_a_stub_after_the_prompt_changes_makes_every_call_again() {
    let r = rig().await;
    let (mock, path) = stub_after_part_five(&r).await;
    use_prompt(&r, &custom_prompt("Team sync", "RECAP-MARKER"));
    assert_eq!(r.runner.regenerate_summary("s").await.unwrap(), path);
    let calls = chat_messages(&mock);
    assert!(
        calls[5].1.contains("<transcript part=\"1\">"),
        "{}",
        &calls[5].1[..80]
    );
    assert!(calls.last().unwrap().0.contains("RECAP-MARKER"));
    assert_eq!(prompt_name(&path), Some("Team sync".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerating_a_stub_with_a_changed_text_under_the_same_id_makes_every_call_again() {
    let r = rig().await;
    let before = custom_prompt("Team sync", "FIRST-MARKER");
    use_prompt(&r, &before);
    let (mock, path) = stub_after_part_five(&r).await;
    use_prompt(
        &r,
        &summarize::Prompt {
            text: "Write the summary as one paragraph. EDITED-MARKER".into(),
            ..before
        },
    );
    assert_eq!(r.runner.regenerate_summary("s").await.unwrap(), path);
    let calls = chat_messages(&mock);
    assert!(calls[5].1.contains("<transcript part=\"1\">"));
    assert!(calls.last().unwrap().0.contains("EDITED-MARKER"));
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerating_a_stub_with_no_saved_prompt_makes_every_call_again() {
    let r = rig().await;
    let (mock, path) = stub_after_part_five(&r).await;
    // A stub from before prompts were saved has no snapshot.
    r.store
        .lock()
        .unwrap()
        .delete_checkpoint("s", Stage::Summarize, pipeline::summarize::PROMPT)
        .unwrap();
    assert_eq!(r.runner.regenerate_summary("s").await.unwrap(), path);
    let calls = chat_calls(&mock);
    assert!(
        calls[5].contains("<transcript part=\"1\">"),
        "{}",
        &calls[5][..80]
    );
    assert_eq!(prompt_name(&path), Some("Topic summary".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerating_a_stub_after_its_prompt_is_renamed_reuses_the_calls_and_records_the_new_name()
{
    let r = rig().await;
    let before = custom_prompt("Team sync", "RECAP-MARKER");
    use_prompt(&r, &before);
    let (mock, path) = stub_after_part_five(&r).await;
    use_prompt(
        &r,
        &summarize::Prompt {
            name: "Campaign recap".into(),
            ..before
        },
    );
    assert_eq!(r.runner.regenerate_summary("s").await.unwrap(), path);
    let calls = chat_calls(&mock);
    assert!(
        calls[5].contains("<transcript part=\"5\">"),
        "{}",
        &calls[5][..80]
    );
    assert_eq!(prompt_name(&path), Some("Campaign recap".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_saved_whole_call_result_is_never_used_as_a_part_note() {
    let r = rig().await;
    let count = Arc::new(AtomicUsize::new(0));
    let reply = http::Reply::With(Arc::new(move |_| {
        let k = count.fetch_add(1, Ordering::SeqCst);
        (200, http::completion(&format!("Reply {k}.")))
    }));
    let mock = summaries_from(&r, reply, Some(32_000)).await;
    long_session(&r);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(chat_calls(&mock).len(), 1, "the transcript fits one call");
    // A stub whose plan is gone: the next run decides the fit again, under
    // a smaller context, and the transcript no longer fits one call.
    {
        let s = r.store.lock().unwrap();
        s.delete_checkpoint("s", Stage::Summarize, checkpoint::DONE)
            .unwrap();
        s.delete_checkpoint("s", Stage::Summarize, pipeline::summarize::PLAN)
            .unwrap();
    }
    *r.script.summary.lock().unwrap() = Some(Ok(SummaryEndpoint {
        context_limit: Some(2_048),
        ..endpoint(&mock.base)
    }));
    r.runner.regenerate_summary("s").await.unwrap();
    let calls = chat_calls(&mock);
    assert!(
        calls[1].contains("<transcript part=\"1\">"),
        "{}",
        &calls[1][..80]
    );
    let get = |key: &str| {
        checkpoint::get::<String>(&r.store, "s", Stage::Summarize, key)
            .unwrap()
            .unwrap()
    };
    assert_eq!(get(pipeline::summarize::WHOLE), "Reply 0.");
    assert_eq!(get("l00-00000"), "Reply 1.");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_too_long_for_the_context_fails_the_summary_before_any_call() {
    let r = rig().await;
    let text: String = "Keep every detail that was said. "
        .repeat(200)
        .chars()
        .take(summarize::prompts::PROMPT_TEXT_LIMIT)
        .collect();
    use_prompt(
        &r,
        &summarize::Prompt {
            id: "custom-long".into(),
            name: "Long".into(),
            text,
            title_text: String::new(),
        },
    );
    let mock = summaries_from(
        &r,
        http::Reply::Json(200, http::completion("Notes on a part.")),
        Some(2_048),
    )
    .await;
    long_session(&r);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert!(chat_calls(&mock).is_empty(), "no call is made");
    let expected = "the summary prompt is too long for this model's context";
    assert!(r.events.lock().unwrap().iter().any(|e| matches!(
        e,
        RunnerEvent::SummaryUnavailable { message, .. } if message.contains(expected)
    )));
    let path = saved_folder(&r, "s").join("summary.md");
    let stub =
        session_core::folder::Summary::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(stub.body.contains(expected), "{}", stub.body);
    assert!(r.script.learned.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerating_a_prompt_too_long_stub_after_a_larger_context_is_set_finishes() {
    let r = rig().await;
    let text: String = "Keep every detail that was said. "
        .repeat(200)
        .chars()
        .take(summarize::prompts::PROMPT_TEXT_LIMIT)
        .collect();
    use_prompt(
        &r,
        &summarize::Prompt {
            id: "custom-long".into(),
            name: "Long".into(),
            text,
            title_text: String::new(),
        },
    );
    let mock = summaries_from(
        &r,
        http::Reply::Json(200, http::completion("A summary.")),
        Some(2_048),
    )
    .await;
    long_session(&r);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert!(chat_calls(&mock).is_empty(), "no call is made");
    // The same prompt, with a model that has a larger context.
    *r.script.summary.lock().unwrap() = Some(Ok(SummaryEndpoint {
        context_limit: Some(32_000),
        ..endpoint(&mock.base)
    }));
    let path = r.runner.regenerate_summary("s").await.unwrap();
    assert!(!chat_calls(&mock).is_empty(), "the summary calls are made");
    assert_eq!(prompt_name(&path), Some("Long".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resumed_run_with_no_saved_prompt_makes_its_calls_again() {
    let r = rig().await;
    let mock = summaries_from(
        &r,
        http::Reply::Json(200, http::completion("A new summary.")),
        Some(32_000),
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
    // A run from before prompts were saved: a plan and a cached result,
    // and no prompt snapshot.
    let reserve = summarize::tokens::reply_reserve(32_000);
    let plan = serde_json::json!({
        "budget": { "limit": 32_000, "source": "user" },
        "layers": [{ "limit": 32_000, "reserve": reserve }],
    });
    checkpoint::put(
        &r.store,
        "s",
        Stage::Summarize,
        pipeline::summarize::PLAN,
        &plan,
    )
    .unwrap();
    for key in [pipeline::summarize::WHOLE, "l00-00000"] {
        checkpoint::put(&r.store, "s", Stage::Summarize, key, &"An old summary.").unwrap();
    }
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(chat_calls(&mock).len(), 1, "the model is called again");
    let path = saved_folder(&r, "s").join("summary.md");
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("A new summary."), "{body}");
    assert!(!body.contains("An old summary."), "{body}");
    assert!(
        !summarize_keys(&r).iter().any(|k| k == "l00-00000"),
        "the old part note is gone"
    );
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
    assert_eq!(destination_entries(&r).len(), 1);

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
        matches!(&outcome, Outcome::AwaitingDestination { problem } if problem.contains("no storage folder")),
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

/// A title change on this computer: the row takes the title, and the
/// folder is owed the name it gives.
fn set_title(store: &SharedStore, id: &str, title: &str) {
    let guard = store.lock().unwrap();
    guard
        .set_title(id, title, session_core::TitleSource::User)
        .unwrap();
    guard.set_rename_owed(id, true).unwrap();
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
        .filter(|name| name != library::scan::LONGHAND_FOLDER)
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
    drop(s);
    record_resumed_segment(r, id, audio);
}

/// Record one more segment of `audio` into the resumed session `id`, a day
/// after its last one, and stop.
fn record_resumed_segment(r: &Rig, id: &str, audio: &[f32]) {
    let mut s = r.store.lock().unwrap();
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
    assert_eq!(destination_entries(&r).len(), 1);
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
        .filter(|name| name != library::scan::LONGHAND_FOLDER)
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

/// The folder steps of a launch (KTD9): settle the rename intents, then
/// give every saved folder the name its title gives.
fn launch_folders(r: &Rig) {
    folder_rename::resolve_intents(&r.store, &session_save::RealFs).unwrap();
    folder_rename::reconcile_saved(&r.store, &session_save::RealFs).unwrap();
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
    // The rewrite keeps the complete record (KTD2).
    let doc = session_md(&new);
    assert_eq!(doc.title_source, Some(session_core::TitleSource::User));
    assert_eq!(doc.asr_model.as_deref(), Some("m"));
    assert_fingerprints_match(&new);
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
    launch_folders(&r);
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

    launch_folders(&r);
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

    launch_folders(&r);
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

    launch_folders(&r);
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
    launch_folders(&r);
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

    launch_folders(&r);
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

    launch_folders(&r);
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

#[tokio::test(flavor = "multi_thread")]
async fn a_title_change_that_keeps_the_folder_name_keeps_the_recorded_fingerprints() {
    // KTD6: only the publication and the accept of a hand edit set the
    // fingerprints. A rename does not read them from the files.
    let r = rig().await;
    let folder = saved_session(&r, Some("Budget review")).await;
    let before = session_md(&folder);
    std::fs::write(folder.join(TRANSCRIPT_FILE), "Hand edit\n").unwrap();
    std::fs::write(folder.join("summary.md"), "Hand edit\n").unwrap();

    assert_eq!(
        r.runner.rename_session("s", "Budget review?").unwrap(),
        FolderRename::Unchanged
    );
    let doc = session_md(&folder);
    assert_eq!(doc.title.as_deref(), Some("Budget review?"));
    assert_eq!(doc.transcript_fingerprint, before.transcript_fingerprint);
    assert_eq!(doc.summary_fingerprint, before.summary_fingerprint);
    assert_eq!(doc.recognition_artifact, before.recognition_artifact);
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

// ------------------------------------------------------------- transcribe again (U9)

/// Summaries that name their call: each summary differs from the one
/// before. Title calls fail.
async fn numbered_summaries(r: &Rig) -> http::Mock {
    let n = Arc::new(AtomicUsize::new(0));
    let reply = http::Reply::With(Arc::new(move |_| {
        let k = n.fetch_add(1, Ordering::SeqCst);
        (200, http::completion(&format!("Summary number {k}.")))
    }));
    summaries_from(r, reply, None).await
}

/// "s" saved with model "m", with notes and, when given, a user title.
async fn saved_note(r: &Rig, title: Option<&str>) -> std::path::PathBuf {
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    add_notes(&r.store, "s");
    if let Some(title) = title {
        set_title(&r.store, "s", title);
    }
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    saved_folder(r, "s")
}

fn read(folder: &std::path::Path, name: &str) -> String {
    std::fs::read_to_string(folder.join(name)).unwrap()
}

fn marks(r: &Rig) -> session_core::TranscribeAgainMarks {
    r.store.lock().unwrap().transcribe_again_marks("s").unwrap()
}

fn asr_model(r: &Rig) -> Option<String> {
    r.store
        .lock()
        .unwrap()
        .session("s")
        .unwrap()
        .unwrap()
        .asr_model
}

/// The transcription model that the test settings select.
const MODEL: &str = models::DEFAULT_ASR_MODEL;

/// The recognition settings of the tests: the default model and no
/// speaker source.
fn settings() -> RecognitionConfig {
    RecognitionConfig {
        speaker_source: SpeakerSource::Off,
        ..Default::default()
    }
}

/// The readiness that the provider of `r` reports: a model in
/// `Script::missing` is not downloaded.
fn readiness(r: &Rig) -> impl Fn(&ExecutionSpec) -> Readiness + '_ {
    |spec| {
        let missing = r.script.missing.lock().unwrap();
        match spec.artifacts.iter().find(|a| missing.contains(&a.id)) {
            Some(a) => Readiness::Missing {
                artifact: a.id.clone(),
            },
            None => Readiness::Ready,
        }
    }
}

/// The user confirms Transcribe Again on "s" with `config` in Settings.
fn again_with(r: &Rig, config: &RecognitionConfig) -> Result<(), Refusal> {
    r.runner
        .transcribe_again("s", config, &config.asr_model, &readiness(r))
}

/// The user confirms Transcribe Again on "s" with the test settings.
fn confirm_again(r: &Rig) -> Result<(), Refusal> {
    again_with(r, &settings())
}

/// The check before the dialog opens, with `config` in Settings.
fn check_with(r: &Rig, config: &RecognitionConfig) -> Result<ExecutionSpec, Refusal> {
    r.runner.check_transcribe_again("s", config, &readiness(r))
}

/// The snapshot that the queued run of "s" uses.
fn pending_snapshot(r: &Rig) -> ExecutionSpec {
    let value = r
        .store
        .lock()
        .unwrap()
        .result_state("s", "pending-snapshot")
        .unwrap()
        .expect("a pending snapshot");
    serde_json::from_value(value).unwrap()
}

fn set_count(r: &Rig, count: Option<session_core::SpeakerCount>) {
    r.store
        .lock()
        .unwrap()
        .set_speaker_count("s", count)
        .unwrap();
}

fn record_count(r: &Rig) -> Option<session_core::SpeakerCount> {
    r.store
        .lock()
        .unwrap()
        .session("s")
        .unwrap()
        .unwrap()
        .speaker_count
}

#[tokio::test(flavor = "multi_thread")]
async fn transcribe_again_replaces_the_transcript_and_the_summary_and_keeps_the_notes_and_title() {
    // Covers AE7.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let folder = saved_note(&r, Some("Budget review")).await;
    let (transcript, summary, notes) = (
        read(&folder, TRANSCRIPT_FILE),
        read(&folder, "summary.md"),
        read(&folder, "notes.md"),
    );
    let calls = r.script.calls.load(Ordering::SeqCst);

    confirm_again(&r).unwrap();
    assert!(matches!(state(&r.store, "s"), SessionState::Queued { .. }));
    assert_eq!(asr_model(&r).as_deref(), Some("m"), "not before the save");
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);

    assert_eq!(saved_folder(&r, "s"), folder, "the same folder");
    assert_eq!(destination_entries(&r).len(), 1);
    assert_ne!(read(&folder, TRANSCRIPT_FILE), transcript);
    assert_stored_fingerprint(&r, "s", &folder);
    assert_ne!(read(&folder, "summary.md"), summary);
    assert!(summary_is_complete(&folder));
    assert_eq!(read(&folder, "notes.md"), notes);
    assert_eq!(title_of(&r).unwrap().text, "Budget review");
    assert_eq!(asr_model(&r).as_deref(), Some(MODEL));
    assert_eq!(marks(&r), Default::default());
    // A full pass, with the new model only.
    let called = r.script.models_called.lock().unwrap()[calls..].to_vec();
    assert_eq!(called.len(), plan(&r.store, "s").windows.len());
    assert!(called.iter().all(|m| m == MODEL), "{called:?}");
    assert!(!r.root.join("s").exists(), "the working copy is gone");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_legacy_note_is_transcribed_again_in_full_with_no_consent_and_promotes_only_after_save() {
    // R9: the confirmation is the permission to transcribe again.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = saved_note(&r, Some("Kept title")).await;
    let old = read(&home, TRANSCRIPT_FILE);
    assert!(session_md(&home).recognition_artifact.is_none());
    let calls = r.script.calls.load(Ordering::SeqCst);
    confirm_again(&r).unwrap();
    assert!(matches!(state(&r.store, "s"), SessionState::Queued { .. }));
    let snapshot = pending_snapshot(&r);
    assert!(
        r.store
            .lock()
            .unwrap()
            .recognition_snapshot("s")
            .unwrap()
            .is_none()
    );
    assert_eq!(read(&home, TRANSCRIPT_FILE), old);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    assert!(
        r.script.calls.load(Ordering::SeqCst) > calls,
        "a full transcription"
    );
    let doc = session_md(&home);
    assert_eq!(
        doc.recognition_snapshot,
        Some(serde_json::to_value(snapshot).unwrap())
    );
    assert!(doc.recognition_artifact.is_some());
    assert!(doc.recognition_audio.is_some());
    let calls = r.script.calls.load(Ordering::SeqCst);
    let loads = r.script.loads.lock().unwrap().len();
    let summary = read(&home, "summary.md");
    // Covers AE4. Only the speaker source changed, and Settings has
    // "During recording" timing.
    let config = RecognitionConfig {
        speaker_source: SpeakerSource::Pyannote,
        processing_mode: ProcessingMode::DuringRecording,
        ..settings()
    };
    again_with(&r, &config).unwrap();
    let snapshot = pending_snapshot(&r);
    assert_eq!(snapshot.config.processing_mode, ProcessingMode::AfterStop);
    assert_eq!(
        snapshot.config.speaker_processing_mode,
        ProcessingMode::AfterStop
    );
    assert!(!snapshot.enabled_live_transcription);
    assert!(!snapshot.enabled_live_speaker_analysis);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(
        r.script.calls.load(Ordering::SeqCst),
        calls,
        "timed text reuse does not run ASR again"
    );
    assert_eq!(
        r.script.loads.lock().unwrap().len(),
        loads,
        "timed text reuse does not load ASR"
    );
    assert!(read(&home, TRANSCRIPT_FILE).contains("Speaker 1:"));
    assert_ne!(
        read(&home, "summary.md"),
        summary,
        "reuse refreshes the summary"
    );
    let before = session_md(&home).summary_fingerprint;
    r.runner.regenerate_summary("s").await.unwrap();
    let doc = session_md(&home);
    assert!(doc.recognition_artifact.is_some());
    assert_ne!(doc.summary_fingerprint, before);
    assert_eq!(doc.title.as_deref(), Some("Kept title"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_model_that_cannot_use_the_saved_text_runs_a_full_transcription() {
    // Covers AE1.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = saved_note(&r, None).await;
    confirm_again(&r).unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert!(session_md(&home).recognition_artifact.is_some());
    let transcript = read(&home, TRANSCRIPT_FILE);
    let calls = r.script.calls.load(Ordering::SeqCst);

    let other = RecognitionConfig {
        asr_model: "parakeet-tdt-0.6b-v3-q80".into(),
        ..settings()
    };
    again_with(&r, &other).unwrap();
    assert!(matches!(state(&r.store, "s"), SessionState::Queued { .. }));
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let called = r.script.models_called.lock().unwrap()[calls..].to_vec();
    assert!(!called.is_empty(), "a full transcription");
    assert!(called.iter().all(|m| *m == other.asr_model), "{called:?}");
    assert_ne!(read(&home, TRANSCRIPT_FILE), transcript);
    assert_eq!(asr_model(&r), Some(other.asr_model));
}

/// "s" saved, then transcribed again: its folder has a `recognition.jsonl`
/// that `session.md` refers to.
async fn note_with_timed_text(r: &Rig) -> std::path::PathBuf {
    let home = saved_note(r, Some("Kept title")).await;
    confirm_again(r).unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert!(session_md(&home).recognition_artifact.is_some());
    assert!(
        home.join(session_core::recognition::ARTIFACT_FILE)
            .is_file()
    );
    home
}

/// The `recognition.jsonl` of "s" is changed by `change`. The run then
/// reads the file in the folder.
async fn transcribe_again_with_artifact(change: impl FnOnce(&std::path::Path)) {
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = note_with_timed_text(&r).await;
    change(&home.join(session_core::recognition::ARTIFACT_FILE));
    let calls = r.script.calls.load(Ordering::SeqCst);
    // A speaker-only change. The run uses saved timed text that it can use.
    let config = RecognitionConfig {
        speaker_source: SpeakerSource::Pyannote,
        ..settings()
    };
    again_with(&r, &config).unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert!(
        r.script.calls.load(Ordering::SeqCst) > calls,
        "a full transcription"
    );
    assert!(session_md(&home).recognition_artifact.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn transcribe_again_with_a_missing_recognition_artifact_runs_in_full() {
    // KTD8, R11: the file that `session.md` refers to is gone.
    transcribe_again_with_artifact(|path| std::fs::remove_file(path).unwrap()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn transcribe_again_with_a_changed_recognition_artifact_runs_in_full() {
    // KTD8: the file does not have the checksum that `session.md` records.
    transcribe_again_with_artifact(|path| {
        let mut bytes = std::fs::read(path).unwrap();
        bytes.extend_from_slice(b"\n");
        std::fs::write(path, bytes).unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_with_no_recognition_artifact_regenerates_renames_and_saves_notes() {
    // Covers AE4, R12. Audio removal deleted `recognition.jsonl`, and
    // `session.md` still refers to it.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = note_with_timed_text(&r).await;
    std::fs::remove_file(home.join(session_core::recognition::ARTIFACT_FILE)).unwrap();
    let summary = read(&home, "summary.md");

    let path = r.runner.regenerate_summary("s").await.unwrap();
    assert_eq!(path, home.join("summary.md"));
    assert_ne!(read(&home, "summary.md"), summary);

    // The rename also runs with no `recognition.jsonl` in the folder.
    std::fs::remove_file(home.join(session_core::recognition::ARTIFACT_FILE)).ok();
    let renamed = r.runner.rename_session("s", "Budget review").unwrap();
    let new = r.dest.join(name_for(Some("Budget review")));
    assert_eq!(renamed, FolderRename::Moved(new.clone()));
    assert_eq!(document_title(&new).as_deref(), Some("Budget review"));
    assert_fingerprints_match(&new);

    let mut store = r.store.lock().unwrap();
    assert!(pipeline::save::set_notes(&mut store, "s", "- later notes\n").unwrap());
    drop(store);
    assert!(read(&new, "notes.md").contains("later notes"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_whose_audio_is_removed_has_no_recognition_jsonl_and_regenerates_and_renames() {
    // Covers AE4, R11, R12, through the removal step.
    use pipeline::remove_audio::{NoteSync, no_guard, remove_audio};
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = note_with_timed_text(&r).await;
    let artifact = session_core::recognition::ARTIFACT_FILE;
    let written = std::fs::read(home.join("session.md")).unwrap();

    let removal = remove_audio(
        &r.store,
        r.runner.claims(),
        "s",
        no_guard(),
        NoteSync::Synced,
    )
    .unwrap();
    assert!(removal.complete());
    assert!(!home.join(artifact).exists());
    assert_eq!(std::fs::read(home.join("session.md")).unwrap(), written);

    let summary = read(&home, "summary.md");
    let path = r.runner.regenerate_summary("s").await.unwrap();
    assert_eq!(path, home.join("summary.md"));
    assert_ne!(read(&home, "summary.md"), summary);
    assert!(session_md(&home).recognition_artifact.is_none());
    assert!(!home.join(artifact).exists());

    let renamed = r.runner.rename_session("s", "Budget review").unwrap();
    let new = r.dest.join(name_for(Some("Budget review")));
    assert_eq!(renamed, FolderRename::Moved(new.clone()));
    assert_eq!(document_title(&new).as_deref(), Some("Budget review"));
    assert_fingerprints_match(&new);
    assert!(!new.join(artifact).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removal_during_a_regeneration_on_another_computer_brings_no_recognition_jsonl_back() {
    // KTD5: the regeneration never writes `recognition.jsonl`, and its
    // `session.md` drops the reference to a file that is gone.
    let r = rig().await;
    let _first = numbered_summaries(&r).await;
    let home = note_with_timed_text(&r).await;
    let artifact = home.join(session_core::recognition::ARTIFACT_FILE);
    // Another computer removes the audio during the summary call.
    let removed = artifact.clone();
    let reply = http::Reply::With(Arc::new(move |_| {
        let _ = std::fs::remove_file(&removed);
        (200, http::completion("The new summary."))
    }));
    let _second = summaries_from(&r, reply, None).await;

    r.runner.regenerate_summary("s").await.unwrap();

    assert!(read(&home, "summary.md").contains("The new summary."));
    let doc = session_md(&home);
    assert!(doc.recognition_artifact.is_none());
    assert!(doc.recognition_audio.is_none());
    assert!(!artifact.exists());
    assert_fingerprints_match(&home);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rename_after_a_hand_edit_of_the_transcript_writes_session_md() {
    // R9: the edit makes the saved timed text unusable. The write does not
    // fail on it.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = note_with_timed_text(&r).await;
    let before = session_md(&home);
    std::fs::write(home.join(TRANSCRIPT_FILE), "Hand edit\n").unwrap();

    let renamed = r.runner.rename_session("s", "Budget review").unwrap();
    let new = r.dest.join(name_for(Some("Budget review")));
    assert_eq!(renamed, FolderRename::Moved(new.clone()));
    assert_eq!(document_title(&new).as_deref(), Some("Budget review"));
    assert_eq!(read(&new, TRANSCRIPT_FILE), "Hand edit\n");
    // KTD6: the rename keeps the recorded fingerprint.
    assert_eq!(
        session_md(&new).transcript_fingerprint,
        before.transcript_fingerprint
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hand_edit_of_the_transcript_is_summarized_and_transcribe_again_replaces_it() {
    // Covers AE2, R8, R9: after the wait for the sync ends, Regenerate
    // Summary reads the edited transcript, and Transcribe Again runs in full.
    let r = rig().await;
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&bodies);
    let reply = http::Reply::With(Arc::new(move |body: &str| {
        seen.lock().unwrap().push(body.to_string());
        (200, http::completion("A summary."))
    }));
    let _mock = summaries_from(&r, reply, None).await;
    let home = note_with_timed_text(&r).await;
    let edit = "# Transcript\n\n- [00:00:01] Speaker 1: A hand edit of the text.\n";
    std::fs::write(home.join(TRANSCRIPT_FILE), edit).unwrap();
    bodies.lock().unwrap().clear();

    r.runner.regenerate_summary("s").await.unwrap();
    assert!(
        bodies
            .lock()
            .unwrap()
            .iter()
            .any(|body| body.contains("A hand edit of the text.")),
        "the summary call has the edited transcript"
    );
    // The accept of the edit records its fingerprint and drops the timed
    // text.
    assert_eq!(read(&home, TRANSCRIPT_FILE), edit);
    assert_fingerprints_match(&home);
    assert!(session_md(&home).recognition_artifact.is_none());

    let calls = r.script.calls.load(Ordering::SeqCst);
    let config = RecognitionConfig {
        speaker_source: SpeakerSource::Pyannote,
        ..settings()
    };
    again_with(&r, &config).unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert!(
        r.script.calls.load(Ordering::SeqCst) > calls,
        "a full transcription"
    );
    assert_ne!(read(&home, TRANSCRIPT_FILE), edit);
    assert_fingerprints_match(&home);
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerate_summary_makes_a_missing_summary() {
    // R8: a missing `summary.md` counts after the wait, so the action can
    // make it again.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = saved_note(&r, None).await;
    std::fs::remove_file(home.join("summary.md")).unwrap();

    let path = r.runner.regenerate_summary("s").await.unwrap();
    assert_eq!(path, home.join("summary.md"));
    assert!(read(&home, "summary.md").contains("Summary number"));
    assert_fingerprints_match(&home);
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerate_summary_with_no_transcript_refuses() {
    // R8: with no `transcript.md` there is nothing to summarize.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = saved_note(&r, None).await;
    std::fs::remove_file(home.join(TRANSCRIPT_FILE)).unwrap();
    let summary = read(&home, "summary.md");
    let written = std::fs::read(home.join("session.md")).unwrap();

    let error = r.runner.regenerate_summary("s").await.unwrap_err();
    assert_eq!(error.to_string(), "The transcript is missing.");
    assert_eq!(read(&home, "summary.md"), summary);
    assert_eq!(std::fs::read(home.join("session.md")).unwrap(), written);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_run_uses_the_speaker_count_of_the_note_and_not_the_settings_default() {
    // Covers AE2.
    use session_core::SpeakerCount;
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = saved_note(&r, None).await;
    edit_session_md(&home, |doc| {
        doc.speaker_count = Some(SpeakerCount::Exact(3))
    });
    let config = RecognitionConfig {
        speaker_source: SpeakerSource::Pyannote,
        speaker_count: Some(models::recognition::SpeakerCount::Exact { count: 2 }),
        ..settings()
    };
    again_with(&r, &config).unwrap();
    assert_eq!(
        pending_snapshot(&r).config.speaker_count,
        Some(models::recognition::SpeakerCount::Exact { count: 3 })
    );
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(record_count(&r), Some(SpeakerCount::Exact(3)));
    assert_eq!(
        session_md(&home).speaker_count,
        Some(SpeakerCount::Exact(3))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_with_an_automatic_speaker_count_keeps_it_after_the_save() {
    // Covers AE2.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = saved_note(&r, None).await;
    assert_eq!(record_count(&r), None);
    let config = RecognitionConfig {
        speaker_source: SpeakerSource::Pyannote,
        speaker_count: Some(models::recognition::SpeakerCount::Exact { count: 2 }),
        ..settings()
    };
    again_with(&r, &config).unwrap();
    assert_eq!(pending_snapshot(&r).config.speaker_count, None);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(record_count(&r), None);
    assert_eq!(session_md(&home).speaker_count, None);
}

/// "s" stays saved, and no run is queued for it.
fn assert_not_queued(r: &Rig) {
    assert_eq!(state(&r.store, "s"), SessionState::Saved);
    assert_eq!(marks(r).model, None);
    assert!(r.store.lock().unwrap().queue().unwrap().is_empty());
}

fn refusal(message: &str, in_settings: bool) -> Refusal {
    Refusal {
        message: message.into(),
        in_settings,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_check_gives_the_model_of_the_run_and_queues_nothing() {
    let r = rig().await;
    saved_note(&r, None).await;
    let spec = check_with(&r, &settings()).unwrap();
    assert_eq!(spec.config.asr_model, MODEL);
    assert_eq!(
        pipeline::reprocess::model_name(MODEL),
        "Parakeet Unified EN 0.6B"
    );
    assert_not_queued(&r);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_model_that_is_not_downloaded_is_refused_with_a_way_to_settings() {
    // Covers AE3. The second refusal is the confirmation after the model
    // is deleted.
    let r = rig().await;
    saved_note(&r, None).await;
    r.script.missing.lock().unwrap().push(MODEL.into());
    let expected = refusal(
        "Parakeet Unified EN 0.6B is not downloaded. Download it in Settings.",
        true,
    );
    assert_eq!(check_with(&r, &settings()).unwrap_err(), expected);
    assert_not_queued(&r);
    assert_eq!(confirm_again(&r).unwrap_err(), expected);
    assert_not_queued(&r);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_model_that_is_downloading_is_refused_with_no_way_to_settings() {
    let r = rig().await;
    saved_note(&r, None).await;
    let downloading = |spec: &ExecutionSpec| Readiness::Downloading {
        artifact: spec.config.asr_model.clone(),
    };
    let expected = refusal(
        "Parakeet Unified EN 0.6B is still downloading. Try again when it is ready.",
        false,
    );
    assert_eq!(
        r.runner
            .check_transcribe_again("s", &settings(), &downloading)
            .unwrap_err(),
        expected
    );
    assert_eq!(
        r.runner
            .transcribe_again("s", &settings(), MODEL, &downloading)
            .unwrap_err(),
        expected
    );
    assert_not_queued(&r);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_language_that_the_model_does_not_support_is_refused_with_a_way_to_settings() {
    let r = rig().await;
    saved_note(&r, None).await;
    let config = RecognitionConfig {
        language: Some("fr".into()),
        ..settings()
    };
    let expected = refusal(
        "Parakeet Unified EN 0.6B does not support French. Choose another model or language \
         in Settings.",
        true,
    );
    assert_eq!(check_with(&r, &config).unwrap_err(), expected);
    assert_eq!(again_with(&r, &config).unwrap_err(), expected);
    assert_not_queued(&r);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_count_above_the_speaker_limit_is_refused_with_the_count_and_the_limit() {
    use session_core::SpeakerCount;
    let r = rig().await;
    let home = saved_note(&r, None).await;
    edit_session_md(&home, |doc| {
        doc.speaker_count = Some(SpeakerCount::Exact(5))
    });
    set_count(&r, Some(SpeakerCount::Exact(5)));
    let config = RecognitionConfig {
        speaker_source: SpeakerSource::Sortformer,
        ..settings()
    };
    let expected = refusal(
        "This note has 5 speakers. Sortformer supports up to 4. Choose another speaker \
         source in Settings.",
        true,
    );
    assert_eq!(check_with(&r, &config).unwrap_err(), expected);
    assert_eq!(again_with(&r, &config).unwrap_err(), expected);
    assert_not_queued(&r);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_confirmation_after_the_settings_changed_is_refused() {
    let r = rig().await;
    saved_note(&r, None).await;
    let shown = check_with(&r, &settings()).unwrap().config.asr_model;
    // The user selects another model in Settings while the dialog is open.
    let now = RecognitionConfig {
        asr_model: "parakeet-tdt-0.6b-v3-q80".into(),
        ..settings()
    };
    assert_eq!(
        r.runner
            .transcribe_again("s", &now, &shown, &readiness(&r))
            .unwrap_err(),
        refusal(
            "The transcription settings changed. Choose Transcribe Again to use the new \
             settings.",
            false
        )
    );
    assert_not_queued(&r);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_confirmation_queues_nothing_and_says_the_run_is_queued() {
    let r = rig().await;
    saved_note(&r, None).await;
    confirm_again(&r).unwrap();
    let expected = refusal("This note is already being transcribed again.", false);
    assert_eq!(confirm_again(&r).unwrap_err(), expected);
    assert_eq!(check_with(&r, &settings()).unwrap_err(), expected);
    assert_eq!(r.store.lock().unwrap().queue().unwrap(), ["s"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_save_refused_because_another_computer_changed_session_md_keeps_the_previous_result() {
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = saved_note(&r, None).await;
    let (transcript, summary) = (read(&home, TRANSCRIPT_FILE), read(&home, "summary.md"));
    confirm_again(&r).unwrap();
    // Another computer writes `session.md` while the run waits in line.
    edit_session_md(&home, |doc| doc.title = Some("Desktop title".into()));

    // The save compares `session.md` with the value that the confirmation
    // read after the row followed its folder.
    let (_, outcome) = r.runner.run_next().await.unwrap();
    match outcome {
        Outcome::Failed {
            stage: Stage::Save,
            message,
        } => assert!(
            message.contains("the note changed during processing"),
            "{message}"
        ),
        outcome => panic!("{outcome:?}"),
    }
    assert_eq!(read(&home, TRANSCRIPT_FILE), transcript);
    assert_eq!(read(&home, "summary.md"), summary);
    assert_eq!(
        session_md(&home).title.as_deref(),
        Some("Desktop title"),
        "the other computer's change stays"
    );
    assert_eq!(asr_model(&r).as_deref(), Some("m"));
    // The save had started, so the run keeps its staged result for a retry.
    assert!(matches!(
        state(&r.store, "s"),
        SessionState::ProcessingFailed { .. }
    ));
    assert!(r.root.join("s").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_processing_audio_during_summary_stops_save_and_retry_verifies_again() {
    use std::io::{Seek, SeekFrom, Write};
    let mut r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let spec = models::recognition::resolve(&models::recognition::RecognitionConfig {
        speaker_source: models::recognition::SpeakerSource::Off,
        ..Default::default()
    })
    .unwrap();
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some(&spec.config.asr_model),
        &[(16_000, speech())],
    );
    r.store
        .lock()
        .unwrap()
        .set_recognition_snapshot("s", &serde_json::to_value(spec).unwrap())
        .unwrap();
    let root = r.root.clone();
    let sink = r.events.clone();
    let changed = Arc::new(AtomicBool::new(false));
    let change = changed.clone();
    r.runner = Runner::new(Environment {
        store: r.store.clone(),
        working_root: r.root.clone(),
        engines: Box::new(Provider(r.script.clone())),
        suspended: Box::new(|| false),
        events: Box::new(move |event| {
            if matches!(
                event,
                RunnerEvent::StageStarted {
                    stage: Stage::Summarize,
                    ..
                }
            ) && !change.swap(true, Ordering::SeqCst)
            {
                let path = root
                    .join("s")
                    .join(pipeline::proc_file("audio/segment-01.opus"));
                let before = std::fs::metadata(&path).unwrap();
                let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                file.seek(SeekFrom::End(-4)).unwrap();
                file.write_all(&0.12345_f32.to_le_bytes()).unwrap();
                file.set_times(std::fs::FileTimes::new().set_modified(before.modified().unwrap()))
                    .unwrap();
                let after = file.metadata().unwrap();
                assert_eq!(after.len(), before.len());
                assert_eq!(after.modified().unwrap(), before.modified().unwrap());
            }
            sink.lock().unwrap().push(event);
        }),
    });
    assert!(matches!(
        r.runner.run_next().await.unwrap().1,
        Outcome::Failed {
            stage: Stage::Save,
            ..
        }
    ));
    assert!(changed.load(Ordering::SeqCst));
    assert!(
        destination_entries(&r).is_empty(),
        "changed input must not be published"
    );
    let calls = r.script.calls.load(Ordering::SeqCst);
    r.events.lock().unwrap().clear();
    r.runner.retry("s").unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_fingerprints_match(&saved_folder(&r, "s"));
    assert!(
        r.script.calls.load(Ordering::SeqCst) > calls,
        "retry recomputes recognition from verified audio"
    );
    assert!(
        r.events.lock().unwrap().iter().any(|event| matches!(
            event,
            RunnerEvent::StageStarted {
                stage: Stage::Summarize,
                ..
            }
        )),
        "retry refreshes the summary"
    );
    assert!(
        r.events.lock().unwrap().iter().any(|event| matches!(
            event,
            RunnerEvent::StageStarted {
                stage: Stage::Assemble,
                ..
            }
        )),
        "retry rebuilds the changed processing audio"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn transcribe_again_proposes_no_title_and_keeps_the_folder_name() {
    let r = rig().await;
    let title = title_reply("First");
    let mock = summaries_and_titles(&r, a_summary(), &title, None).await;
    let folder = saved_session(&r, None).await;
    assert_eq!(title_of(&r), proposed("First"));

    *title.lock().unwrap() = (200, http::completion("Second"));
    confirm_again(&r).unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(title_calls(&mock).len(), 1, "no title call for the run");
    assert_eq!(title_of(&r), proposed("First"));
    assert_eq!(saved_folder(&r, "s"), folder);
    assert_eq!(destination_entries(&r), [name_for(Some("First"))]);
}

/// The transcript that the stored checkpoints of "s" give.
fn rebuilt_transcript(r: &Rig) -> String {
    let segments = r.store.lock().unwrap().segments("s").unwrap();
    pipeline::join::build(
        &segments,
        &transcribe::results(&r.store, "s").unwrap(),
        &pipeline::diarize::speakers(&r.store, "s").unwrap(),
    )
    .render()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recognizer_failure_during_transcribe_again_keeps_the_previous_transcript() {
    // Covers AE12, and AE5 of the Transcribe Again from Settings plan.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let folder = saved_note(&r, None).await;
    let before: Vec<String> = [TRANSCRIPT_FILE, "summary.md", "notes.md", "session.md"]
        .iter()
        .map(|name| read(&folder, name))
        .collect();
    let turns = rebuilt_transcript(&r);

    confirm_again(&r).unwrap();
    *r.script.fail_on.lock().unwrap() = Some(r.script.calls.load(Ordering::SeqCst));
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::Failed {
                stage: Stage::Transcribe,
                ..
            }
        ),
        "{outcome:?}"
    );

    assert_eq!(state(&r.store, "s"), SessionState::Saved);
    let after: Vec<String> = [TRANSCRIPT_FILE, "summary.md", "notes.md", "session.md"]
        .iter()
        .map(|name| read(&folder, name))
        .collect();
    assert_eq!(after, before);
    assert_eq!(asr_model(&r).as_deref(), Some("m"));
    let marks = marks(&r);
    assert_eq!(marks.model, None);
    assert!(
        marks
            .error
            .as_deref()
            .is_some_and(|e| e.contains("decoder exploded")),
        "{marks:?}"
    );
    assert!(!r.root.join("s").exists(), "the working copy is gone");
    assert!(r.events.lock().unwrap().iter().any(
        |e| matches!(e, RunnerEvent::TranscribeAgainFailed { session_id, .. } if session_id == "s")
    ));
    // The previous checkpoints came back: they rebuild the saved
    // transcript, and a regeneration works from it.
    assert_eq!(rebuilt_transcript(&r), turns);
    r.runner.regenerate_summary("s").await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_offline_summary_endpoint_preserves_the_complete_previous_result() {
    // U9: a replacement includes both documents or changes neither.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let folder = saved_note(&r, None).await;
    let (transcript, summary) = (read(&folder, TRANSCRIPT_FILE), read(&folder, "summary.md"));

    let _offline = summaries_from(&r, http::Reply::Json(500, "{}".into()), None).await;
    confirm_again(&r).unwrap();
    assert!(matches!(
        r.runner.run_next().await.unwrap().1,
        Outcome::Failed {
            stage: Stage::Summarize,
            ..
        }
    ));

    assert_eq!(state(&r.store, "s"), SessionState::Saved);
    assert_eq!(read(&folder, TRANSCRIPT_FILE), transcript);
    assert_eq!(read(&folder, "summary.md"), summary, "the previous summary");
    assert_fingerprints_match(&folder);
    assert_eq!(session_md(&folder).asr_model.as_deref(), Some("m"));
    assert_eq!(asr_model(&r).as_deref(), Some("m"));
    let marks = marks(&r);
    assert!(marks.error.is_some(), "{marks:?}");

    // A regeneration that succeeds updates the summary and the notice goes.
    let _online = numbered_summaries(&r).await;
    r.runner.regenerate_summary("s").await.unwrap();
    assert_eq!(self::marks(&r).summary_error, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn transcribe_again_is_refused_while_a_regeneration_runs_a_capture_is_active_or_audio_is_gone()
 {
    let r = rig().await;
    let folder = saved_note(&r, None).await;
    // The check and the confirmation give the same plain text, never the
    // text of the state machine.
    let refused = |r: &Rig, message: &str| {
        let expected = refusal(message, false);
        assert_eq!(check_with(r, &settings()).unwrap_err(), expected);
        let error = confirm_again(r).unwrap_err();
        assert!(!error.message.contains("is not legal from"), "{error}");
        assert_eq!(error, expected);
        assert_not_queued(r);
    };

    let claim = r.runner.claims().regenerate("s").unwrap();
    refused(
        &r,
        "The summary of this note is being made again. Try again when it is done.",
    );
    drop(claim);

    r.script.capturing.store(true, Ordering::SeqCst);
    refused(&r, "A recording is in progress. Try again after it stops.");
    r.script.capturing.store(false, Ordering::SeqCst);

    std::fs::remove_file(folder.join("audio/segment-01.opus")).unwrap();
    refused(
        &r,
        "The note's folder does not have all of its audio, so it can't be transcribed again.",
    );
}

// Covers KTD2, KTD9 (U2) and R14 (U7): removed audio has its own reason.
#[tokio::test(flavor = "multi_thread")]
async fn transcribe_again_is_refused_when_a_marker_removes_the_audio_that_is_still_present() {
    let r = rig().await;
    let folder = saved_note(&r, None).await;
    std::fs::write(
        folder.join(session_core::folder::removal_marker(
            "audio/segment-01.opus",
        )),
        session_core::folder::REMOVAL_MARKER_TEXT,
    )
    .unwrap();
    assert!(folder.join("audio/segment-01.opus").is_file());
    assert_eq!(
        pipeline::save::audio_state(&r.store.lock().unwrap(), "s").unwrap(),
        session_core::folder::AudioState::Removed
    );
    let expected = refusal(
        "The audio of this note was removed, so it can't be transcribed again.",
        false,
    );
    assert_eq!(check_with(&r, &settings()).unwrap_err(), expected);
    assert_eq!(confirm_again(&r).unwrap_err(), expected);
    assert_not_queued(&r);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transcribe_again_that_waits_for_its_model_lets_other_notes_go_first() {
    // R17, KTD13. The model goes away after the confirmation.
    let r = rig().await;
    saved_note(&r, None).await;
    confirm_again(&r).unwrap();
    r.script.missing.lock().unwrap().push(MODEL.into());
    queue_session(
        &r.store,
        &r.root,
        "other",
        200,
        Some("m"),
        &[(16_000, speech())],
    );

    let (id, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!((id.as_str(), outcome), ("other", Outcome::Finished));
    assert!(matches!(state(&r.store, "s"), SessionState::Queued { .. }));
    assert!(r.runner.run_next().await.is_none());

    r.script.missing.lock().unwrap().clear();
    let (id, outcome) = r.runner.run_next().await.unwrap();
    assert_eq!((id.as_str(), outcome), ("s", Outcome::Finished));
    assert_eq!(asr_model(&r).as_deref(), Some(MODEL));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transcribe_again_stopped_by_a_crash_resumes_from_its_checkpoints() {
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let folder = saved_note(&r, None).await;
    let transcript = read(&folder, TRANSCRIPT_FILE);
    let calls = r.script.calls.load(Ordering::SeqCst);
    confirm_again(&r).unwrap();
    *r.script.cancel_after.lock().unwrap() = Some((calls + 1, Arc::clone(&r.runner), "s".into()));
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Cancelled);
    let windows = plan(&r.store, "s").windows.len();

    // The next launch.
    let relaunched = Runner::new(Environment {
        store: Arc::clone(&r.store),
        working_root: r.root.clone(),
        engines: Box::new(Provider(Arc::clone(&r.script))),
        suspended: Box::new(|| false),
        events: Box::new(|_| {}),
    });
    assert_eq!(
        relaunched
            .reclaim(session_core::jobs::Liveness::OnlyInstance)
            .unwrap(),
        ["s"]
    );
    assert_eq!(relaunched.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(
        r.script.calls.load(Ordering::SeqCst) - calls,
        windows,
        "no window ran twice"
    );
    assert_ne!(read(&folder, TRANSCRIPT_FILE), transcript);
    assert_eq!(asr_model(&r).as_deref(), Some(MODEL));
    assert_eq!(marks(&r), Default::default());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resumed_transcribe_again_that_then_fails_is_a_normal_failure_that_keeps_the_new_audio() {
    let r = rig().await;
    saved_note(&r, None).await;
    confirm_again(&r).unwrap();
    resume_completed(&r, "s", &speech());
    assert_eq!(marks(&r).model, None);
    *r.script.fail_on.lock().unwrap() = Some(r.script.calls.load(Ordering::SeqCst));
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert!(matches!(outcome, Outcome::Failed { .. }), "{outcome:?}");
    assert!(matches!(
        state(&r.store, "s"),
        SessionState::ProcessingFailed { .. }
    ));
    assert!(r.root.join("s").join("audio/segment-02.opus").exists());
    assert_eq!(marks(&r).error, None);
    // The note keeps its model: the run did not save.
    assert_eq!(asr_model(&r).as_deref(), Some("m"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replacement_with_an_unwritable_summary_rolls_forward_on_retry() {
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let folder = saved_note(&r, None).await;
    let transcript = read(&folder, TRANSCRIPT_FILE);
    let summary = read(&folder, "summary.md");
    confirm_again(&r).unwrap();
    // A directory at summary.md stops the publication after the write of
    // `transcript.md`.
    std::fs::remove_file(folder.join("summary.md")).unwrap();
    std::fs::create_dir_all(folder.join("summary.md/blocked")).unwrap();
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::Failed {
                stage: Stage::Save,
                ..
            }
        ),
        "{outcome:?}"
    );
    // The publication rolls forward only (KTD3): the new transcript stays.
    let replaced = read(&folder, TRANSCRIPT_FILE);
    assert_ne!(replaced, transcript);
    assert!(
        r.root
            .join("s")
            .join(session_save::publication::RECORD)
            .exists()
    );
    assert!(
        !r.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, RunnerEvent::TranscribeAgainFailed { .. }))
    );
    assert!(matches!(
        state(&r.store, "s"),
        SessionState::ProcessingFailed { .. }
    ));
    // The run keeps its marker and its checkpoints until the save finishes.
    assert_eq!(marks(&r).model.as_deref(), Some(MODEL));
    assert_eq!(marks(&r).error, None);
    assert_eq!(asr_model(&r).as_deref(), Some("m"));
    assert!(
        r.root.join("s").exists(),
        "the working copy stays for a retry"
    );

    std::fs::remove_dir_all(folder.join("summary.md")).unwrap();
    std::fs::write(folder.join("summary.md"), summary).unwrap();
    let calls = r.script.calls.load(Ordering::SeqCst);
    r.runner.retry("s").unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(
        r.script.calls.load(Ordering::SeqCst),
        calls,
        "no window ran again"
    );
    assert_eq!(state(&r.store, "s"), SessionState::Saved);
    assert_eq!(read(&folder, TRANSCRIPT_FILE), replaced);
    assert_fingerprints_match(&folder);
    assert!(summary_is_complete(&folder));
    assert_eq!(asr_model(&r).as_deref(), Some(MODEL));
    assert_eq!(marks(&r), Default::default());
    assert!(!r.root.join("s").exists(), "the working copy is gone");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_while_transcribe_again_recognizes_a_window_keeps_the_restored_transcription() {
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    saved_note(&r, None).await;
    let before = transcribe::results(&r.store, "s").unwrap();
    let calls = r.script.calls.load(Ordering::SeqCst);
    confirm_again(&r).unwrap();
    // The resume comes while the run's first window is recognized.
    *r.script.resume_after.lock().unwrap() = Some((calls + 1, Arc::clone(&r.store), "s".into()));
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Cancelled);

    assert_eq!(state(&r.store, "s"), SessionState::Capturing);
    assert_eq!(marks(&r).model, None);
    assert_eq!(
        transcribe::results(&r.store, "s").unwrap(),
        before,
        "the window recognized during the resume is not written"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_speaker_analysis_failure_during_transcribe_again_keeps_the_previous_result() {
    // The run has an exact snapshot: its speaker source is part of the
    // result, so a failure of it fails the run (R19).
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let folder = saved_note(&r, None).await;
    let transcript = read(&folder, TRANSCRIPT_FILE);
    let config = RecognitionConfig {
        speaker_source: SpeakerSource::Pyannote,
        ..settings()
    };
    again_with(&r, &config).unwrap();
    r.script.diarize_fails.store(true, Ordering::SeqCst);
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::Failed {
                stage: Stage::Diarize,
                ..
            }
        ),
        "{outcome:?}"
    );

    assert_eq!(state(&r.store, "s"), SessionState::Saved);
    assert_eq!(read(&folder, TRANSCRIPT_FILE), transcript);
    assert_eq!(asr_model(&r).as_deref(), Some("m"));
    assert!(marks(&r).error.is_some(), "{:?}", marks(&r));
    assert!(r.events.lock().unwrap().iter().any(
        |e| matches!(e, RunnerEvent::TranscribeAgainFailed { session_id, .. } if session_id == "s")
    ));
}

// ------------------------------------------- stereo notes and echo removal (U9)

const ECHO_DELAY: usize = 4_000;

/// Three remote sentences from the speech fixture, raised to a normal
/// level, with their names.
fn remote_sentences() -> Vec<(String, Vec<f32>)> {
    let speech = speech();
    let cut = |from: f64, to: f64| -> Vec<f32> {
        speech[(from * 16_000.0) as usize..(to * 16_000.0) as usize]
            .iter()
            .map(|s| s * 10.0)
            .collect()
    };
    vec![
        ("alpha".into(), cut(0.5, 2.5)),
        ("bravo".into(), cut(3.0, 5.0)),
        ("charlie".into(), cut(16.0, 18.0)),
    ]
}

/// A call on the laptop speakers: the system channel holds the remote
/// sentences from `starts` (in seconds), and the microphone hears them 250
/// ms late at 0.4, with `room` added.
fn speaker_call(starts: [f64; 3], room: Option<(f64, f64, f64)>) -> (Vec<f32>, Vec<f32>) {
    let len = 24 * 16_000;
    let mut system = vec![0.0f32; len];
    for ((_, sentence), start) in remote_sentences().iter().zip(starts) {
        let at = (start * 16_000.0) as usize;
        system[at..at + sentence.len()].copy_from_slice(sentence);
    }
    let mut mic = common::delayed(&system, ECHO_DELAY, 0.4);
    if let Some((at, from, to)) = room {
        let speech = speech();
        let room = &speech[(from * 16_000.0) as usize..(to * 16_000.0) as usize];
        let at = (at * 16_000.0) as usize;
        for (m, s) in mic[at..].iter_mut().zip(room) {
            *m += s * 8.0;
        }
    }
    (mic, system)
}

async fn stereo_rig() -> Rig {
    let r = rig().await;
    *r.script.sentences_heard.lock().unwrap() = remote_sentences();
    r
}

fn queue_stereo(r: &Rig, segments: &[common::Recorded]) {
    common::queue_recorded_session(&r.store, &r.root, "s", 100, Some("m"), segments);
}

/// How many times the transcript holds each remote sentence.
fn times_heard(r: &Rig) -> Vec<(String, usize)> {
    let text: String = transcribe::results(&r.store, "s")
        .unwrap()
        .iter()
        .map(|w| format!(" {} ", w.text))
        .collect();
    remote_sentences()
        .into_iter()
        .map(|(name, _)| {
            let n = text.matches(&format!(" {name}")).count();
            (name, n)
        })
        .collect()
}

/// [`times_heard`] in the saved `transcript.md` of "s". A Transcribe Again
/// run keeps its text in the recognition result, not in the window results.
fn times_heard_in_transcript(r: &Rig) -> Vec<(String, usize)> {
    // The test tokens have no space before them, so the rows join them to
    // the call number.
    let text = read(&saved_folder(r, "s"), TRANSCRIPT_FILE);
    remote_sentences()
        .into_iter()
        .map(|(name, _)| {
            let n = text.matches(name.as_str()).count();
            (name, n)
        })
        .collect()
}

fn once_each() -> Vec<(String, usize)> {
    remote_sentences()
        .into_iter()
        .map(|(name, _)| (name, 1))
        .collect()
}

#[test]
fn the_test_recognizer_hears_an_echo_250_ms_late_as_a_second_sentence() {
    // The check of each test below: without echo removal, the plain sum of
    // the channels holds each sentence twice.
    let (mic, system) = speaker_call([2.0, 9.0, 16.0], None);
    let sum = common::add(&mic.iter().map(|s| s * 2.5).collect::<Vec<_>>(), &system);
    let names = heard(&sum, &remote_sentences());
    assert_eq!(
        names,
        ["alpha", "alpha", "bravo", "bravo", "charlie", "charlie"]
    );
    assert_eq!(
        heard(&system, &remote_sentences()),
        ["alpha", "bravo", "charlie"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_on_the_speakers_is_transcribed_with_each_remote_sentence_once() {
    // Covers AE2.
    let r = stereo_rig().await;
    let (mic, system) = speaker_call([2.0, 9.0, 16.0], None);
    queue_stereo(&r, &[common::Recorded::Stereo(16_000, mic, system)]);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let echo = pipeline::echo::saved(&r.store, "s", 0).unwrap().unwrap();
    assert_eq!(echo.spans.len(), 3, "{:?}", echo.spans);
    assert_eq!(times_heard(&r), once_each());
}

#[tokio::test(flavor = "multi_thread")]
async fn live_text_inside_the_echo_spans_gives_way_to_the_cleaned_mix() {
    // Covers AE2 with the live transcript on (U10): the live text holds each
    // remote sentence twice, from the system channel and from its echo.
    let r = stereo_rig().await;
    let starts = [2.0, 9.0, 16.0];
    let (mic, system) = speaker_call(starts, None);
    let duration_ms = mic.len() as u64 / 16;
    queue_stereo(&r, &[common::Recorded::Stereo(16_000, mic, system)]);
    let word = |start_ms: u64, end_ms: u64, text: &str| asr::LiveWord {
        start_ms,
        end_ms,
        text: text.into(),
    };
    // A room word before the call, outside every echo span.
    let mut words = vec![word(500, 800, "hello")];
    for ((name, sentence), start) in remote_sentences().iter().zip(starts) {
        let at = (start * 1_000.0) as u64;
        let len = sentence.len() as u64 / 16;
        words.push(word(at, at + len, name));
        words.push(word(at + 250, at + 250 + len, name));
    }
    let part = asr::LivePart {
        start_ms: 0,
        end_ms: duration_ms,
        words,
    };
    pipeline::live::save_part(&r.store, "s", 0, 0, &part).unwrap();

    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let echo = pipeline::echo::saved(&r.store, "s", 0).unwrap().unwrap();
    assert_eq!(echo.spans.len(), 3, "{:?}", echo.spans);
    assert_eq!(times_heard(&r), once_each());
    let live: Vec<String> = transcribe::results(&r.store, "s")
        .unwrap()
        .into_iter()
        .filter(|w| w.live)
        .map(|w| w.text)
        .collect();
    assert_eq!(live, ["hello"], "the live word outside the spans stays");
    let transcript = std::fs::read_to_string(saved_folder(&r, "s").join(TRANSCRIPT_FILE)).unwrap();
    assert!(transcript.contains("hello"), "{transcript}");
}

#[tokio::test(flavor = "multi_thread")]
async fn room_speech_over_the_call_keeps_each_remote_sentence_once() {
    let r = stereo_rig().await;
    // The room talks over the second sentence.
    let (mic, system) = speaker_call([2.0, 9.0, 16.0], Some((8.5, 20.0, 26.0)));
    queue_stereo(&r, &[common::Recorded::Stereo(16_000, mic, system)]);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let echo = pipeline::echo::saved(&r.store, "s", 0).unwrap().unwrap();
    assert!(
        echo.spans
            .iter()
            .any(|s| s.start_ms <= 9_350 && s.end_ms >= 11_250),
        "{:?}",
        echo.spans
    );
    assert_eq!(times_heard(&r), once_each());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_sentence_that_starts_an_echo_span_is_heard_once() {
    // The first sentence starts 1 s into the note, so its span starts 1.25 s
    // in, and the sentences follow each other closely.
    let r = stereo_rig().await;
    let (mic, system) = speaker_call([1.0, 5.5, 10.0], None);
    queue_stereo(&r, &[common::Recorded::Stereo(16_000, mic, system)]);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let echo = pipeline::echo::saved(&r.store, "s", 0).unwrap().unwrap();
    assert!(
        echo.spans[0].start_ms.abs_diff(1_250) <= 100,
        "{:?}",
        echo.spans
    );
    assert_eq!(times_heard(&r), once_each());
}

/// The note "s" with one stereo call segment, saved.
async fn saved_stereo_note(r: &Rig, segments: &[common::Recorded]) -> std::path::PathBuf {
    queue_stereo(r, segments);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    saved_folder(r, "s")
}

#[tokio::test(flavor = "multi_thread")]
async fn transcribe_again_on_a_stereo_note_finds_the_same_echo_spans_within_100_ms() {
    let r = stereo_rig().await;
    let (mic, system) = speaker_call([2.0, 9.0, 16.0], None);
    saved_stereo_note(&r, &[common::Recorded::Stereo(16_000, mic, system)]).await;
    let first = pipeline::echo::saved(&r.store, "s", 0).unwrap().unwrap();

    confirm_again(&r).unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let again = pipeline::echo::saved(&r.store, "s", 0).unwrap().unwrap();
    assert_eq!(again.spans.len(), first.spans.len(), "{:?}", again.spans);
    for (a, b) in first.spans.iter().zip(&again.spans) {
        assert!(
            a.start_ms.abs_diff(b.start_ms) <= 100 && a.end_ms.abs_diff(b.end_ms) <= 100,
            "{a:?} against {b:?}"
        );
    }
    assert_eq!(
        times_heard_in_transcript(&r),
        once_each(),
        "from the cleaned mix"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn transcribe_again_on_a_note_with_a_mono_and_a_stereo_segment_decodes_each() {
    let r = stereo_rig().await;
    let (mic, system) = speaker_call([2.0, 9.0, 16.0], None);
    saved_stereo_note(
        &r,
        &[
            common::Recorded::Mono(48_000, vec![0.0; 3 * 48_000]),
            common::Recorded::Stereo(16_000, mic, system),
        ],
    )
    .await;
    // `session.md` keeps each segment's channels; the stereo mark follows.
    let doc = session_md(&saved_folder(&r, "s"));
    assert_eq!(doc.segments[0].channels, 1);
    assert_eq!(doc.segments[1].channels, 2);
    assert_eq!(doc.stereo_from(), Some(1));
    assert_eq!(
        doc.stereo_from(),
        r.store.lock().unwrap().stereo_from("s").unwrap()
    );
    confirm_again(&r).unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(pipeline::echo::saved(&r.store, "s", 0).unwrap(), None);
    assert_eq!(
        pipeline::echo::saved(&r.store, "s", 1)
            .unwrap()
            .unwrap()
            .spans
            .len(),
        3
    );
    assert_eq!(times_heard_in_transcript(&r), once_each());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_transcribe_again_restores_the_previous_echo_spans() {
    let r = stereo_rig().await;
    let (mic, system) = speaker_call([2.0, 9.0, 16.0], None);
    saved_stereo_note(&r, &[common::Recorded::Stereo(16_000, mic, system)]).await;
    let before = checkpoint::all(&r.store, "s", Stage::Assemble).unwrap();
    assert!(before.iter().any(|(k, _)| k == &checkpoint::echo_key(0)));

    let calls = r.script.calls.load(Ordering::SeqCst);
    confirm_again(&r).unwrap();
    // Assemble runs again and saves a new echo spans row; then a resume
    // takes the run back during its first window.
    *r.script.resume_after.lock().unwrap() = Some((calls + 1, Arc::clone(&r.store), "s".into()));
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Cancelled);
    assert_eq!(state(&r.store, "s"), SessionState::Capturing);
    assert_eq!(
        checkpoint::all(&r.store, "s", Stage::Assemble).unwrap(),
        before
    );
}

// ---------------------------------------------------- the folder is the authority

use library::reconcile::{Changes, Reconciler};
use pipeline::save::NotesSave;
use session_core::folder::{SessionDoc, Transcript, TranscriptLine};

/// A second computer: its own store, working location and recognizer, and
/// the Destination of `a`, which a sync shares.
async fn other_computer(a: &Rig) -> Rig {
    let b = rig_with(store(), tempfile::tempdir().unwrap(), Arc::default());
    *b.script.destination.lock().unwrap() = Some(a.dest.clone());
    with_summaries(&b.script).await;
    b
}

/// A library scan of `root` applied to the store of `r` (KTD5, KTD6).
fn sync(r: &Rig, root: &std::path::Path) -> Changes {
    r.runner
        .reconcile_library(&[root.to_path_buf()], &mut Reconciler::default())
        .unwrap()
}

/// Change `session.md` in `folder`, as another computer's write does.
fn edit_session_md(folder: &std::path::Path, edit: impl FnOnce(&mut SessionDoc)) {
    let mut doc = session_md(folder);
    edit(&mut doc);
    std::fs::write(folder.join(pipeline::save::SESSION_FILE), doc.render()).unwrap();
}

/// Another computer renames the note: the folder takes the new name, and
/// its `session.md` the user's title. Returns the new folder.
fn rename_on_other_computer(folder: &std::path::Path, title: &str) -> std::path::PathBuf {
    let started = session_md(folder).timeline_segments()[0].wall_start;
    let renamed = folder
        .parent()
        .unwrap()
        .join(session_save::naming::folder_name(started, Some(title)));
    std::fs::rename(folder, &renamed).unwrap();
    edit_session_md(&renamed, |doc| {
        doc.title = Some(title.into());
        doc.title_source = Some(TitleSource::User);
    });
    renamed
}

/// The text of the turns before the first gap of the folder's transcript:
/// what the first recording said.
fn first_recording_text(folder: &std::path::Path) -> String {
    Transcript::parse(&read(folder, TRANSCRIPT_FILE))
        .unwrap()
        .lines
        .into_iter()
        .take_while(|line| !matches!(line, TranscriptLine::Gap { .. }))
        .filter_map(|line| match line {
            TranscriptLine::Turn { text, .. } => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resume the saved note `id` as capture does: the row follows its folder
/// (KTD3), and a note with no recognition results takes the text of its
/// recorded segments from `transcript.md` (KTD12). Then record one more
/// segment and stop.
fn resume_from_folder(r: &Rig, id: &str, audio: &[f32]) {
    {
        let mut s = r.store.lock().unwrap();
        pipeline::follow::follow_folder(&mut s, id, false).unwrap();
        let home = s.session(id).unwrap().unwrap().home_path.unwrap();
        assert_eq!(
            pipeline::save::audio_state(&s, id).unwrap(),
            session_core::folder::AudioState::Present
        );
        s.apply(
            id,
            &session_core::Event::UserResume {
                space_ok: true,
                audio_intact: true,
            },
        )
        .unwrap();
        pipeline::transcribe::seed_from_transcript(&s, id, std::path::Path::new(&home)).unwrap();
    }
    record_resumed_segment(r, id, audio);
}

fn calls(r: &Rig) -> usize {
    r.script.calls.load(Ordering::SeqCst)
}

/// The windows planned for the segment `index` of "s".
fn windows_of(r: &Rig, index: u32) -> usize {
    let plan: WindowPlan = checkpoint::get(
        &r.store,
        "s",
        Stage::Transcribe,
        &checkpoint::plan_key(index),
    )
    .unwrap()
    .unwrap();
    plan.windows.len()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_uses_the_notes_and_keeps_the_title_that_another_computer_wrote() {
    // Covers AE5.
    let a = rig().await;
    saved_note(&a, None).await;
    let b = other_computer(&a).await;
    assert_eq!(sync(&b, &a.dest).changed, ["s"]);

    // The desktop renames the note, and the laptop's scan follows the
    // folder. Then the desktop edits the notes.
    assert!(matches!(
        b.runner.rename_session("s", "Budget review").unwrap(),
        FolderRename::Moved(_)
    ));
    let renamed = saved_folder(&b, "s");
    sync(&a, &a.dest);
    assert_eq!(saved_folder(&a, "s"), renamed);
    assert!(
        pipeline::save::set_notes(&mut b.store.lock().unwrap(), "s", "- the desktop notes\n")
            .unwrap()
    );

    let mock = numbered_summaries(&a).await;
    a.runner.regenerate_summary("s").await.unwrap();
    let calls = chat_calls(&mock);
    assert!(
        calls.iter().all(|c| c.contains("- the desktop notes")),
        "{calls:?}"
    );
    assert_eq!(saved_folder(&a, "s"), renamed);
    assert_eq!(destination_entries(&a), [name_for(Some("Budget review"))]);
    assert_eq!(session_md(&renamed).title.as_deref(), Some("Budget review"));
    assert_eq!(read(&renamed, "notes.md"), "- the desktop notes\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_on_a_computer_with_no_recognition_results_summarizes_transcript_md() {
    let a = rig().await;
    let folder = saved_note(&a, None).await;
    let b = other_computer(&a).await;
    sync(&b, &a.dest);
    assert!(
        checkpoint::all(&b.store, "s", Stage::Transcribe)
            .unwrap()
            .is_empty()
    );

    let mock = numbered_summaries(&b).await;
    b.runner.regenerate_summary("s").await.unwrap();
    let turns: Vec<String> = Transcript::parse(&read(&folder, TRANSCRIPT_FILE))
        .unwrap()
        .lines
        .into_iter()
        .filter_map(|line| match line {
            TranscriptLine::Turn { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert!(!turns.is_empty());
    let calls = chat_calls(&mock);
    assert!(
        turns
            .iter()
            .all(|t| calls.iter().any(|c| c.contains(t.as_str()))),
        "{calls:?}"
    );
    assert!(read(&folder, "summary.md").contains("Summary number"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_resumed_on_another_computer_keeps_its_text_and_transcribes_only_the_new_segment() {
    // Covers AE3.
    let a = rig().await;
    let folder = saved_note(&a, None).await;
    let audio = std::fs::read(folder.join("audio/segment-01.opus")).unwrap();
    let said = first_recording_text(&folder);
    let b = other_computer(&a).await;
    sync(&b, &a.dest);

    resume_from_folder(&b, "s", &speech());
    assert_eq!(b.runner.run_next().await.unwrap().1, Outcome::Finished);

    assert_eq!(saved_folder(&b, "s"), folder);
    assert_eq!(
        std::fs::read(folder.join("audio/segment-01.opus")).unwrap(),
        audio
    );
    assert!(folder.join("audio/segment-02.opus").exists());
    let transcript = Transcript::parse(&read(&folder, TRANSCRIPT_FILE)).unwrap();
    let gaps = transcript
        .lines
        .iter()
        .filter(|l| matches!(l, TranscriptLine::Gap { .. }))
        .count();
    assert_eq!(gaps, 1, "both recordings");
    assert_eq!(first_recording_text(&folder), said);
    assert_eq!(calls(&b), windows_of(&b, 1), "only the new segment");
    assert_fingerprints_match(&folder);

    // The laptop then shows the updated note.
    sync(&a, &a.dest);
    assert_eq!(a.store.lock().unwrap().segments("s").unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn transcribe_again_on_another_computer_decodes_the_folder_audio_and_keeps_notes_and_title() {
    let a = rig().await;
    let folder = saved_note(&a, Some("Budget review")).await;
    let notes = read(&folder, "notes.md");
    let b = other_computer(&a).await;
    sync(&b, &a.dest);

    confirm_again(&b).unwrap();
    assert_eq!(b.runner.run_next().await.unwrap().1, Outcome::Finished);

    assert!(calls(&b) > 0);
    assert!(
        b.script
            .models_called
            .lock()
            .unwrap()
            .iter()
            .all(|m| m == MODEL)
    );
    assert_eq!(saved_folder(&b, "s"), folder);
    assert_eq!(read(&folder, "notes.md"), notes);
    let doc = session_md(&folder);
    assert_eq!(doc.title.as_deref(), Some("Budget review"));
    assert_eq!(doc.asr_model.as_deref(), Some(MODEL));
    assert_fingerprints_match(&folder);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_after_another_computer_transcribed_again_uses_the_snapshot_of_that_run() {
    // A Transcribe Again run saves an exact snapshot. This computer has no
    // recognition generation of that run, so a resume transcribes every
    // segment again with the snapshot of the run.
    let a = rig().await;
    let folder = saved_note(&a, None).await;
    let b = other_computer(&a).await;
    sync(&b, &a.dest);
    *b.script.says.lock().unwrap() = Some((0, "Heard on the desktop.".into()));
    confirm_again(&b).unwrap();
    assert_eq!(b.runner.run_next().await.unwrap().1, Outcome::Finished);
    let desktop = first_recording_text(&folder);
    assert!(desktop.contains("Heard on the desktop."), "{desktop}");
    let snapshot = session_md(&folder).recognition_snapshot;
    assert!(snapshot.is_some());

    let before = a.script.models_called.lock().unwrap().len();
    resume_from_folder(&a, "s", &speech());
    assert_eq!(a.runner.run_next().await.unwrap().1, Outcome::Finished);
    let called = a.script.models_called.lock().unwrap()[before..].to_vec();
    assert!(!called.is_empty());
    assert!(called.iter().all(|m| m == MODEL), "{called:?}");
    assert_eq!(session_md(&folder).recognition_snapshot, snapshot);
    assert_eq!(session_md(&folder).timeline_segments().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notes_edit_from_another_computer_during_transcribe_again_is_kept() {
    let a = rig().await;
    let folder = saved_note(&a, None).await;
    confirm_again(&a).unwrap();
    // The other computer edits the notes while the run waits in line.
    std::fs::write(folder.join("notes.md"), "- the desktop notes\n").unwrap();

    assert_eq!(a.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(read(&folder, "notes.md"), "- the desktop notes\n");
    assert_eq!(
        a.store.lock().unwrap().notes("s").unwrap(),
        "- the desktop notes\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notes_edit_on_this_computer_during_transcribe_again_is_kept() {
    let a = rig().await;
    let folder = saved_note(&a, None).await;
    confirm_again(&a).unwrap();
    // The folder takes the edit at once, so the run's refresh keeps it.
    assert!(pipeline::save::set_notes(&mut a.store.lock().unwrap(), "s", "- my notes\n").unwrap());
    assert_eq!(read(&folder, "notes.md"), "- my notes\n");

    assert_eq!(a.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(read(&folder, "notes.md"), "- my notes\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rename_from_another_computer_during_reprocessing_preserves_the_previous_result() {
    let a = rig().await;
    let folder = saved_note(&a, None).await;
    confirm_again(&a).unwrap();
    let renamed = rename_on_other_computer(&folder, "Budget review");
    // The scan follows the folder of a row in a run (KTD6).
    sync(&a, &a.dest);

    assert!(matches!(
        a.runner.run_next().await.unwrap().1,
        Outcome::Failed {
            stage: Stage::Save,
            ..
        }
    ));
    assert_eq!(
        a.store
            .lock()
            .unwrap()
            .session("s")
            .unwrap()
            .unwrap()
            .home_path
            .as_deref(),
        Some(renamed.to_string_lossy().as_ref())
    );
    assert!(!folder.exists());
    assert_eq!(destination_entries(&a), [name_for(Some("Budget review"))]);
    let doc = session_md(&renamed);
    assert_eq!(doc.title.as_deref(), Some("Budget review"));
    assert_eq!(doc.asr_model.as_deref(), Some("m"));
    assert!(a.root.join("s").exists(), "staged work remains recoverable");
    assert_eq!(
        title_of(&a),
        Some(Title {
            text: "Budget review".into(),
            source: TitleSource::User,
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_launch_reconcile_does_not_rename_a_folder_back_after_a_synced_rename() {
    let a = rig().await;
    let folder = saved_session(&a, Some("Planning")).await;
    let renamed = rename_on_other_computer(&folder, "Budget review");

    // The launch steps in their order (KTD9).
    folder_rename::resolve_intents(&a.store, &session_save::RealFs).unwrap();
    sync(&a, &a.dest);
    folder_rename::reconcile_saved(&a.store, &session_save::RealFs).unwrap();

    assert_eq!(saved_folder(&a, "s"), renamed);
    assert_eq!(destination_entries(&a), [name_for(Some("Budget review"))]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_launch_reconcile_does_not_rename_a_folder_whose_new_session_md_has_not_synced() {
    let a = rig().await;
    let folder = saved_session(&a, Some("Planning")).await;
    // Another computer renamed the note. The folder move synced, but the
    // session.md with the new title did not.
    let renamed = folder
        .parent()
        .unwrap()
        .join(name_for(Some("Budget review")));
    std::fs::rename(&folder, &renamed).unwrap();

    folder_rename::resolve_intents(&a.store, &session_save::RealFs).unwrap();
    sync(&a, &a.dest);
    folder_rename::reconcile_saved(&a.store, &session_save::RealFs).unwrap();

    assert_eq!(saved_folder(&a, "s"), renamed);
    assert_eq!(destination_entries(&a), [name_for(Some("Budget review"))]);
    assert_eq!(document_title(&renamed).as_deref(), Some("Planning"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_save_writes_the_library_marker_in_the_destination() {
    let a = rig().await;
    assert!(!library::scan::marker(&a.dest).exists());
    saved_session(&a, None).await;
    assert!(library::scan::marker(&a.dest).is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notes_save_over_a_file_that_another_computer_changed_writes_a_draft_beside_it() {
    use pipeline::save::{notes_fingerprint, save_notes};
    let a = rig().await;
    let folder = saved_note(&a, None).await;
    let mut s = a.store.lock().unwrap();
    let loaded = notes_fingerprint(&s.notes("s").unwrap());
    std::fs::write(folder.join("notes.md"), "- the desktop notes\n").unwrap();

    let NotesSave::Conflict { draft } =
        save_notes(&mut s, "s", "- my notes\n", Some(&loaded)).unwrap()
    else {
        panic!("no conflict");
    };
    assert_eq!(read(&folder, "notes.md"), "- the desktop notes\n");
    assert_eq!(s.notes("s").unwrap(), "- the desktop notes\n");
    assert_eq!(std::fs::read_to_string(&draft).unwrap(), "- my notes\n");
    assert_eq!(draft.parent(), Some(folder.as_path()));
    let name = draft.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("notes (Longhand conflict "), "{name}");
    assert_eq!(library::conflict::document_copy(name), Some("notes.md"));

    // An edit of the file as it is now saves.
    let current = notes_fingerprint("- the desktop notes\n");
    assert_eq!(
        save_notes(&mut s, "s", "- both notes\n", Some(&current)).unwrap(),
        NotesSave::Saved { written: true }
    );
    assert_eq!(read(&folder, "notes.md"), "- both notes\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notes_save_over_a_file_that_cannot_be_read_writes_a_draft_and_keeps_the_file() {
    use pipeline::save::{notes_fingerprint, save_notes};
    let a = rig().await;
    let folder = saved_note(&a, None).await;
    let mut s = a.store.lock().unwrap();
    let loaded = notes_fingerprint(&s.notes("s").unwrap());
    // A `notes.md` that cannot be read, as a file that does not download.
    std::fs::remove_file(folder.join("notes.md")).unwrap();
    std::fs::create_dir(folder.join("notes.md")).unwrap();

    let NotesSave::Conflict { draft } =
        save_notes(&mut s, "s", "- my notes\n", Some(&loaded)).unwrap()
    else {
        panic!("no conflict");
    };
    assert!(folder.join("notes.md").is_dir());
    assert_eq!(std::fs::read_to_string(&draft).unwrap(), "- my notes\n");
    assert_eq!(draft.parent(), Some(folder.as_path()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_with_a_transcript_that_cannot_be_read_names_the_storage_folder() {
    let a = rig().await;
    let folder = saved_note(&a, None).await;
    let summary = read(&folder, "summary.md");
    std::fs::remove_file(folder.join(TRANSCRIPT_FILE)).unwrap();
    std::fs::create_dir(folder.join(TRANSCRIPT_FILE)).unwrap();

    let error = a.runner.regenerate_summary("s").await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("the storage folder is not available"),
        "{error}"
    );
    assert_eq!(read(&folder, "summary.md"), summary);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rename_whose_folder_move_fails_keeps_the_title_after_a_library_scan() {
    let r = rig().await;
    let old = saved_session(&r, None).await;
    let fs = AlwaysInUse(AtomicUsize::new(0));
    let result =
        folder_rename::rename_session(&r.store, "s", "Budget review", &|_| false, &fs).unwrap();
    assert!(matches!(result, FolderRename::Kept(_)), "{result:?}");
    assert_eq!(document_title(&old).as_deref(), Some("Budget review"));

    sync(&r, &r.dest);
    assert_eq!(
        title_of(&r),
        Some(Title {
            text: "Budget review".into(),
            source: TitleSource::User,
        })
    );
    // The launch reconcile then moves the folder.
    folder_rename::reconcile_saved(&r.store, &session_save::RealFs).unwrap();
    assert_eq!(destination_entries(&r), [name_for(Some("Budget review"))]);
}

#[tokio::test(flavor = "multi_thread")]
async fn regenerating_a_stub_after_another_computer_replaced_the_transcript_makes_every_call_again()
{
    let r = rig().await;
    let (mock, path) = stub_after_part_five(&r).await;
    let folder = path.parent().unwrap().to_path_buf();

    // Another computer replaces `transcript.md`, and its `session.md`
    // records the new fingerprint.
    let mut transcript = Transcript::parse(&read(&folder, TRANSCRIPT_FILE)).unwrap();
    for line in &mut transcript.lines {
        if let TranscriptLine::Turn { text, .. } = line {
            *text = format!("{text} REMOTE-MARKER");
        }
    }
    let rendered = transcript.render();
    std::fs::write(folder.join(TRANSCRIPT_FILE), &rendered).unwrap();
    edit_session_md(&folder, |doc| {
        doc.transcript_fingerprint = Some(session_core::folder::fingerprint(rendered.as_bytes()));
    });
    sync(&r, &r.dest);

    assert_eq!(r.runner.regenerate_summary("s").await.unwrap(), path);
    let calls = chat_calls(&mock);
    // The regeneration starts at part 1 again, from the new text.
    let again = &calls[5..];
    assert!(
        again[0].contains("<transcript part=\"1\">"),
        "{}",
        &again[0][..80]
    );
    assert!(calls[..5].iter().all(|c| !c.contains("REMOTE-MARKER")));
    assert!(again.iter().any(|c| c.contains("REMOTE-MARKER")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_keeps_a_user_title_that_another_computer_wrote_during_its_calls() {
    let a = rig().await;
    let folder = saved_note(&a, None).await;
    let previous_summary = read(&folder, "summary.md");
    let written = Arc::new(AtomicUsize::new(0));
    let at = folder.clone();
    let reply = http::Reply::With(Arc::new(move |_| {
        // Another computer sets the title while the call runs.
        if written.fetch_add(1, Ordering::SeqCst) == 0 {
            edit_session_md(&at, |doc| {
                doc.title = Some("Budget review".into());
                doc.title_source = Some(TitleSource::User);
            });
        }
        (200, http::completion("A new summary."))
    }));
    summaries_from(&a, reply, None).await;

    assert!(
        a.runner.regenerate_summary("s").await.is_err(),
        "metadata used by the summary changed"
    );
    // The check runs before the staging: no staging and no record stay.
    assert!(
        a.store
            .lock()
            .unwrap()
            .result_state("s", "summary-publication")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        title_of(&a),
        Some(Title {
            text: "Budget review".into(),
            source: TitleSource::User,
        })
    );
    let renamed = saved_folder(&a, "s");
    assert_eq!(destination_entries(&a), [name_for(Some("Budget review"))]);
    assert_eq!(session_md(&renamed).title.as_deref(), Some("Budget review"));
    assert_eq!(read(&renamed, "summary.md"), previous_summary);
}

struct ObservedAssertion(Arc<AtomicUsize>);

impl Drop for ObservedAssertion {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn observed_power() -> (power::ProcessingPower, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let held = Arc::new(AtomicUsize::new(0));
    let acquired = Arc::new(AtomicUsize::new(0));
    let live = Arc::clone(&held);
    let starts = Arc::clone(&acquired);
    let owner = power::ProcessingPower::with_factory(move || {
        live.fetch_add(1, Ordering::SeqCst);
        starts.fetch_add(1, Ordering::SeqCst);
        Some(ObservedAssertion(Arc::clone(&live)))
    })
    .unwrap();
    (owner, held, acquired)
}

#[tokio::test(flavor = "multi_thread")]
async fn processing_power_covers_stage_preparation_and_start() {
    let r = rig().await;
    let (power, held, _) = observed_power();
    let active = Arc::clone(&held);
    let runner = Runner::with_power(
        Environment {
            store: Arc::clone(&r.store),
            working_root: r.root.clone(),
            engines: Box::new(Provider(Arc::clone(&r.script))),
            suspended: Box::new(|| false),
            events: Box::new(move |event| {
                if matches!(event, RunnerEvent::StageStarted { .. }) {
                    assert_eq!(
                        active.load(Ordering::SeqCst),
                        1,
                        "protection starts before stage preparation and notification"
                    );
                }
            }),
        },
        power,
    );
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    assert_eq!(runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(held.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn processing_power_releases_during_capture_and_after_summary_regeneration() {
    let r = rig().await;
    let (power, held, acquired) = observed_power();
    let script = Arc::clone(&r.script);
    let capturing = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&capturing);
    let runner = Runner::with_power(
        Environment {
            store: Arc::clone(&r.store),
            working_root: r.root.clone(),
            engines: Box::new(Provider(Arc::clone(&script))),
            suspended: Box::new(move || flag.load(Ordering::SeqCst)),
            events: Box::new(|_| {}),
        },
        power,
    );
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    assert!(runner.run_next().await.is_none());
    assert_eq!(
        acquired.load(Ordering::SeqCst),
        0,
        "queued work holds no assertion"
    );
    capturing.store(false, Ordering::SeqCst);
    *script.capture_after.lock().unwrap() = Some((1, Arc::clone(&capturing)));
    let job = Arc::clone(&runner);
    let task = tokio::spawn(async move { job.run_next().await });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !capturing.load(Ordering::SeqCst) || held.load(Ordering::SeqCst) != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let before = acquired.load(Ordering::SeqCst);
    assert!(before > 0);
    capturing.store(false, Ordering::SeqCst);
    assert_eq!(task.await.unwrap().unwrap().1, Outcome::Finished);
    assert!(
        acquired.load(Ordering::SeqCst) > before,
        "resumption acquires protection"
    );
    assert_eq!(held.load(Ordering::SeqCst), 0);
    let before = acquired.load(Ordering::SeqCst);
    let active = Arc::clone(&held);
    let mock = summaries_from(
        &r,
        http::Reply::With(Arc::new(move |_| {
            assert_eq!(
                active.load(Ordering::SeqCst),
                1,
                "summary request holds protection"
            );
            (200, http::completion("A replacement summary."))
        })),
        None,
    )
    .await;
    runner.regenerate_summary("s").await.unwrap();
    assert!(!chat_calls(&mock).is_empty());
    assert!(acquired.load(Ordering::SeqCst) > before);
    assert_eq!(held.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn processing_power_releases_after_failure_and_cancellation() {
    for cancel in [false, true] {
        let r = rig().await;
        let (power, held, acquired) = observed_power();
        let runner = Runner::with_power(
            Environment {
                store: Arc::clone(&r.store),
                working_root: r.root.clone(),
                engines: Box::new(Provider(Arc::clone(&r.script))),
                suspended: Box::new(|| false),
                events: Box::new(|_| {}),
            },
            power,
        );
        queue_session(
            &r.store,
            &r.root,
            "s",
            100,
            Some("m"),
            &[(16_000, conversation())],
        );
        if cancel {
            *r.script.cancel_after.lock().unwrap() = Some((1, Arc::clone(&runner), "s".into()));
        } else {
            *r.script.fail_on.lock().unwrap() = Some(0);
        }
        let outcome = runner.run_next().await.unwrap().1;
        if cancel {
            assert_eq!(outcome, Outcome::Cancelled);
        } else {
            assert!(matches!(
                outcome,
                Outcome::Failed {
                    stage: Stage::Transcribe,
                    ..
                }
            ));
        }
        assert!(acquired.load(Ordering::SeqCst) > 0);
        assert_eq!(held.load(Ordering::SeqCst), 0);
    }
}

/// A regeneration of "s" in `home` that wrote its record and stopped
/// after `stop` writes, as a crash does: 0 before `summary.md`, 1 after
/// `summary.md`, 2 after `session.md`. Returns the local staging.
fn stopped_summary(r: &Rig, home: &std::path::Path, stop: usize) -> std::path::PathBuf {
    use session_save::publication::{Publication, WriteSet};
    let local = r.root.join(".longhand-summary-stopped");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::write(local.join("summary.md"), "replacement summary").unwrap();
    let mut doc = session_md(home);
    doc.summary_fingerprint = Some(session_core::folder::fingerprint(b"replacement summary"));
    let publication = Publication::new(home, doc, WriteSet::Summary).unwrap();
    publication.record(&local).unwrap();
    r.store
        .lock()
        .unwrap()
        .set_result_state(
            "s",
            "summary-publication",
            &serde_json::to_value(&local).unwrap(),
        )
        .unwrap();
    if stop > 0 {
        let mut written = 0;
        let stopped = publication.publish_with(home, &local, &mut |_| {
            written += 1;
            if written == stop {
                Err(std::io::Error::other("crash"))
            } else {
                Ok(())
            }
        });
        assert!(stopped.is_err());
    }
    local
}

fn summary_publication(r: &Rig) -> Option<serde_json::Value> {
    r.store
        .lock()
        .unwrap()
        .result_state("s", "summary-publication")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_stopped_at_each_boundary_rolls_forward_at_the_next_launch() {
    for stop in 0..=2 {
        let r = rig().await;
        let home = saved_note(&r, None).await;
        let local = stopped_summary(&r, &home, stop);
        let (summary, session) = (read(&home, "summary.md"), read(&home, "session.md"));
        // An instance that can still be alive may own the publication.
        r.runner
            .reclaim(session_core::jobs::Liveness::Unknown {
                now: chrono::Utc::now().fixed_offset(),
            })
            .unwrap();
        assert_eq!(read(&home, "summary.md"), summary, "stop {stop}");
        assert_eq!(read(&home, "session.md"), session, "stop {stop}");
        assert!(local.join(session_save::publication::RECORD).exists());

        r.runner
            .reclaim(session_core::jobs::Liveness::OnlyInstance)
            .unwrap();
        assert_eq!(
            read(&home, "summary.md"),
            "replacement summary",
            "stop {stop}"
        );
        assert_fingerprints_match(&home);
        assert!(!local.exists(), "stop {stop}: the staging is gone");
        assert!(summary_publication(&r).is_none(), "stop {stop}");
        assert_eq!(state(&r.store, "s"), SessionState::Saved);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_first_finishes_the_publication_that_an_earlier_one_left() {
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = saved_note(&r, None).await;
    let local = stopped_summary(&r, &home, 1);
    r.runner.regenerate_summary("s").await.unwrap();
    let folder = saved_folder(&r, "s");
    assert!(read(&folder, "summary.md").contains("Summary number"));
    assert_fingerprints_match(&folder);
    assert!(!local.exists());
    assert!(summary_publication(&r).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_recovery_after_another_result_drops_the_staging() {
    // KTD4: the last change wins.
    let r = rig().await;
    let home = saved_note(&r, None).await;
    let local = stopped_summary(&r, &home, 1);
    edit_session_md(&home, |doc| {
        doc.transcript_fingerprint = Some(session_core::folder::fingerprint(b"other"));
    });
    let other = read(&home, "session.md");
    r.runner
        .reclaim(session_core::jobs::Liveness::OnlyInstance)
        .unwrap();
    assert_eq!(read(&home, "session.md"), other);
    assert!(!local.exists());
    assert!(summary_publication(&r).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn publication_recovery_error_does_not_prevent_stale_job_reclaim() {
    let r = rig().await;
    let home = saved_note(&r, None).await;
    let local = stopped_summary(&r, &home, 0);
    std::fs::write(
        local.join(session_save::publication::RECORD),
        "invalid json",
    )
    .unwrap();
    queue_session(
        &r.store,
        &r.root,
        "other",
        200,
        Some("m"),
        &[(16_000, conversation())],
    );
    r.store
        .lock()
        .unwrap()
        .dequeue_session("other", "dead-owner", chrono::Utc::now().fixed_offset())
        .unwrap();
    assert_eq!(
        r.runner
            .reclaim(session_core::jobs::Liveness::OnlyInstance)
            .unwrap(),
        vec!["other"]
    );
    assert!(local.join(session_save::publication::RECORD).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn publication_notes_change_keeps_previous_result_and_retry_summarizes_current_notes() {
    let r = rig().await;
    let home = saved_note(&r, Some("Kept title")).await;
    let transcript = read(&home, TRANSCRIPT_FILE);
    let summary = read(&home, "summary.md");
    let store = Arc::clone(&r.store);
    let changed = AtomicBool::new(false);
    let mock = summaries_from(
        &r,
        http::Reply::With(Arc::new(move |_| {
            if !changed.swap(true, Ordering::SeqCst) {
                pipeline::save::set_notes(
                    &mut store.lock().unwrap(),
                    "s",
                    "Updated notes for retry",
                )
                .unwrap();
            }
            (200, http::completion("A replacement summary."))
        })),
        None,
    )
    .await;
    confirm_again(&r).unwrap();
    assert!(matches!(
        r.runner.run_next().await.unwrap().1,
        Outcome::Failed {
            stage: Stage::Save,
            ..
        }
    ));
    // R5: the check runs before the record, so nothing was written.
    assert_eq!(read(&home, TRANSCRIPT_FILE), transcript);
    assert_eq!(read(&home, "summary.md"), summary);
    assert!(
        !r.root
            .join("s")
            .join(session_save::publication::RECORD)
            .exists()
    );
    let calls = r.script.calls.load(Ordering::SeqCst);
    r.runner.retry("s").unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert_eq!(r.script.calls.load(Ordering::SeqCst), calls);
    assert!(
        mock.seen
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .body
            .contains("Updated notes for retry")
    );
    assert_eq!(read(&home, "notes.md"), "Updated notes for retry");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_writes_only_summary_md_and_session_md() {
    // KTD5: the other files keep their inodes, so nothing rewrote them.
    use std::os::unix::fs::MetadataExt;
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let home = note_with_timed_text(&r).await;
    let inode = |name: &str| std::fs::metadata(home.join(name)).unwrap().ino();
    let kept = [
        TRANSCRIPT_FILE,
        session_core::recognition::ARTIFACT_FILE,
        "notes.md",
    ];
    let before: Vec<u64> = kept.iter().map(|name| inode(name)).collect();
    let written = [inode("summary.md"), inode("session.md")];
    r.runner.regenerate_summary("s").await.unwrap();
    assert_eq!(saved_folder(&r, "s"), home);
    assert_eq!(
        kept.iter().map(|name| inode(name)).collect::<Vec<_>>(),
        before
    );
    assert_ne!(inode("summary.md"), written[0]);
    assert_ne!(inode("session.md"), written[1]);
    assert!(session_md(&home).recognition_artifact.is_some());
    assert_fingerprints_match(&home);
}

/// At the first check after the summary call: a cancel, or a `notes.md`
/// edit in the note folder.
struct AfterSummaryCall {
    called: Arc<AtomicBool>,
    home: std::path::PathBuf,
    cancel: bool,
    done: AtomicBool,
}

impl pipeline::Control for AfterSummaryCall {
    fn proceed(&self) -> Result<(), pipeline::StageError> {
        if self.called.load(Ordering::SeqCst) && !self.done.swap(true, Ordering::SeqCst) {
            if self.cancel {
                return Err(pipeline::StageError::Cancelled);
            }
            std::fs::write(self.home.join("notes.md"), "Notes edited during the call\n").unwrap();
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_regeneration_stopped_before_its_record_changes_nothing_and_keeps_the_notes() {
    for cancel in [true, false] {
        let r = rig().await;
        let home = saved_note(&r, Some("Kept title")).await;
        let (summary, session) = (read(&home, "summary.md"), read(&home, "session.md"));
        let called = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&called);
        summaries_from(
            &r,
            http::Reply::With(Arc::new(move |_| {
                seen.store(true, Ordering::SeqCst);
                (200, http::completion("Replacement summary."))
            })),
            None,
        )
        .await;
        let control = AfterSummaryCall {
            called,
            home: home.clone(),
            cancel,
            done: AtomicBool::new(false),
        };
        let store = Arc::clone(&r.store);
        let root = r.root.clone();
        let endpoint = r
            .script
            .summary
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        let regenerated = tokio::task::spawn_blocking(move || {
            pipeline::save::regenerate_summary(
                &store,
                "s",
                &root,
                endpoint,
                summarize::prompts::built_in(summarize::prompts::TOPIC_SUMMARY_ID).unwrap(),
                &|_| {},
                &control,
                &mut |_, _| {},
            )
        })
        .await
        .unwrap();
        let staging: Vec<_> = std::fs::read_dir(&r.root)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".longhand-summary-")
            })
            .collect();
        assert!(staging.is_empty(), "cancel {cancel}: {staging:?}");
        assert!(summary_publication(&r).is_none());
        if cancel {
            // R3: a run that stops before its record changes no file.
            assert!(regenerated.is_err());
            assert_eq!(read(&home, "summary.md"), summary);
            assert_eq!(read(&home, "session.md"), session);
            r.runner.regenerate_summary("s").await.unwrap();
            assert!(read(&home, "summary.md").contains("Replacement summary."));
        } else {
            // R5: the publication does not write `notes.md`.
            regenerated.unwrap();
            assert!(read(&home, "summary.md").contains("Replacement summary."));
            assert_eq!(read(&home, "notes.md"), "Notes edited during the call\n");
        }
    }
}

/// What changes the note between a stopped publication and its rerun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Between {
    Nothing,
    Notes,
    Title,
    OtherResult,
}

/// Covers AE3. A Transcribe Again with another model stops after it wrote
/// `transcript.md` and before it wrote `summary.md`, as a crash there
/// does: the write of `summary.md` fails. `between` then changes the note,
/// and the retry reruns the publication. Returns the rig, the folder, the
/// previous `summary.md`, and the outcome of the rerun.
async fn rerun_after_the_transcript(
    between: Between,
) -> (Rig, std::path::PathBuf, String, Outcome) {
    let r = rig().await;
    let home = saved_note(&r, Some("Kept title")).await;
    let (transcript, summary) = (read(&home, TRANSCRIPT_FILE), read(&home, "summary.md"));
    let session = read(&home, "session.md");
    let at = home.clone();
    let blocked = AtomicBool::new(false);
    summaries_from(
        &r,
        http::Reply::With(Arc::new(move |_| {
            // A folder with the name of `summary.md` makes its write fail.
            if !blocked.swap(true, Ordering::SeqCst) {
                std::fs::remove_file(at.join("summary.md")).unwrap();
                std::fs::create_dir(at.join("summary.md")).unwrap();
            }
            (200, http::completion("The summary after the stop."))
        })),
        None,
    )
    .await;
    let other = RecognitionConfig {
        asr_model: "parakeet-tdt-0.6b-v3-q80".into(),
        ..settings()
    };
    again_with(&r, &other).unwrap();
    assert!(
        matches!(
            r.runner.run_next().await.unwrap().1,
            Outcome::Failed {
                stage: Stage::Save,
                ..
            }
        ),
        "the write of summary.md fails"
    );
    // The publication rolls forward only: the new transcript stays.
    assert_ne!(read(&home, TRANSCRIPT_FILE), transcript);
    assert_eq!(read(&home, "session.md"), session);
    std::fs::remove_dir(home.join("summary.md")).unwrap();
    std::fs::write(home.join("summary.md"), &summary).unwrap();

    match between {
        Between::Nothing => {}
        Between::Notes => std::fs::write(
            home.join("notes.md"),
            session_core::folder::render_notes("Notes from another computer"),
        )
        .unwrap(),
        Between::Title => edit_session_md(&home, |doc| {
            doc.title = Some("Renamed elsewhere".into());
            doc.title_source = Some(TitleSource::User);
        }),
        Between::OtherResult => edit_session_md(&home, |doc| {
            doc.transcript_fingerprint = Some(session_core::folder::fingerprint(b"other"));
        }),
    }
    r.runner.retry("s").unwrap();
    let outcome = r.runner.run_next().await.unwrap().1;
    (r, home, summary, outcome)
}

/// The rerun wrote the whole staged result: the folder is complete, with
/// no result copies, and the working copy is gone.
fn assert_published(r: &Rig, previous_summary: &str) -> std::path::PathBuf {
    let folder = saved_folder(r, "s");
    let summary = read(&folder, "summary.md");
    assert_ne!(summary, previous_summary);
    assert!(summary.contains("The summary after the stop."), "{summary}");
    assert_fingerprints_match(&folder);
    let hidden: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(hidden.is_empty(), "{hidden:?}");
    assert!(!r.root.join("s").exists(), "the working copy is gone");
    folder
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_after_the_transcript_write_finishes_from_the_staged_result() {
    let (r, _, summary, outcome) = rerun_after_the_transcript(Between::Nothing).await;
    assert_eq!(outcome, Outcome::Finished);
    assert_published(&r, &summary);
    assert_eq!(asr_model(&r).as_deref(), Some("parakeet-tdt-0.6b-v3-q80"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_after_a_notes_edit_still_finishes_and_keeps_the_notes() {
    // KTD3: the notes check runs once, before the first write.
    let (r, _, summary, outcome) = rerun_after_the_transcript(Between::Notes).await;
    assert_eq!(outcome, Outcome::Finished);
    let folder = assert_published(&r, &summary);
    assert!(read(&folder, "notes.md").contains("Notes from another computer"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_after_a_title_change_finishes_and_keeps_the_title() {
    // KTD2: a title is not a result field.
    let (r, _, summary, outcome) = rerun_after_the_transcript(Between::Title).await;
    assert_eq!(outcome, Outcome::Finished);
    let folder = assert_published(&r, &summary);
    assert_eq!(
        session_md(&folder).title.as_deref(),
        Some("Renamed elsewhere")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rerun_after_another_result_drops_this_result_and_writes_nothing() {
    // KTD4: the last change wins.
    let (r, home, summary, outcome) = rerun_after_the_transcript(Between::OtherResult).await;
    let other = read(&home, "session.md");
    assert!(
        matches!(
            &outcome,
            Outcome::Failed { stage: Stage::Save, message }
                if message == "This note changed on another computer. Longhand kept that version."
        ),
        "{outcome:?}"
    );
    assert_eq!(state(&r.store, "s"), SessionState::Saved);
    assert_eq!(read(&home, "session.md"), other);
    assert_eq!(read(&home, "summary.md"), summary);
    assert!(!r.root.join("s").exists(), "the staging is gone");
}

/// A note that records while the storage folder changes saves to the new
/// folder, and the old folder gets nothing (U7).
#[tokio::test(flavor = "multi_thread")]
async fn a_note_that_records_during_a_change_of_folder_saves_to_the_new_folder() {
    let r = rig().await;
    r.script.capturing.store(true, Ordering::SeqCst);
    queue_session(
        &r.store,
        &r.root,
        "s",
        100,
        Some("m"),
        &[(16_000, conversation())],
    );
    let new = tempfile::tempdir().unwrap();
    *r.script.destination.lock().unwrap() = Some(new.path().to_path_buf());
    r.script.capturing.store(false, Ordering::SeqCst);
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    let folder = saved_folder(&r, "s");
    assert_eq!(folder.parent(), Some(new.path()));
    assert!(
        destination_entries(&r).is_empty(),
        "{:?}",
        destination_entries(&r)
    );
}

// ---------------------------------------- old audio copy (U5 of the audio removal plan)

/// Resume the saved note `id` as capture does (KTD8 of the audio removal
/// plan): list its old segments and copy their audio into the working
/// location. The caller then records one more segment.
fn resume_saved_with_copy(r: &Rig, id: &str) {
    let segments = {
        let mut s = r.store.lock().unwrap();
        let segments = s.segments(id).unwrap();
        s.apply(
            id,
            &session_core::Event::UserResume {
                space_ok: true,
                audio_intact: true,
            },
        )
        .unwrap();
        segments
    };
    let working = r.root.join(id);
    pipeline::old_audio::begin(&working, &segments).unwrap();
    pipeline::old_audio::copy_for(&r.store, id, &working).unwrap();
}

/// Another computer removes the audio of `audio` from `folder`: the removal
/// marker first, then the delete (KTD2).
fn remove_on_other_computer(folder: &std::path::Path, audio: &str) {
    write_marker(folder, audio);
    std::fs::remove_file(folder.join(audio)).unwrap();
}

fn write_marker(folder: &std::path::Path, audio: &str) {
    std::fs::write(
        folder.join(session_core::folder::removal_marker(audio)),
        session_core::folder::REMOVAL_MARKER_TEXT,
    )
    .unwrap();
}

fn audio_state_of(r: &Rig, id: &str) -> session_core::folder::AudioState {
    pipeline::save::audio_state(&r.store.lock().unwrap(), id).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resumed_note_whose_old_audio_goes_during_recording_saves_only_its_new_audio() {
    // Covers AE9. The note has a recognition artifact, so the saved-prefix
    // check runs.
    let r = rig().await;
    let _mock = numbered_summaries(&r).await;
    let folder = saved_note(&r, None).await;
    confirm_again(&r).unwrap();
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);
    assert!(session_md(&folder).recognition_artifact.is_some());
    let turns = |folder: &std::path::Path| -> Vec<String> {
        read(folder, TRANSCRIPT_FILE)
            .lines()
            .filter(|line| line.starts_with("- ["))
            .map(str::to_string)
            .collect()
    };
    let old_turns = turns(&folder);
    assert!(!old_turns.is_empty());

    resume_saved_with_copy(&r, "s");
    remove_on_other_computer(&folder, "audio/segment-01.opus");
    let before = calls(&r);
    record_resumed_segment(&r, "s", &speech());
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);

    assert_eq!(saved_folder(&r, "s"), folder);
    assert!(
        !folder.join("audio/segment-01.opus").exists(),
        "old audio came back"
    );
    assert!(
        folder.join("audio/segment-02.opus").is_file(),
        "the new audio"
    );
    assert_eq!(
        audio_state_of(&r, "s"),
        session_core::folder::AudioState::Removed
    );
    assert!(!r.root.join("s").exists(), "the working copy is gone");
    assert_eq!(session_md(&folder).timeline_segments().len(), 2);
    let new_turns = turns(&folder);
    assert_eq!(
        new_turns[..old_turns.len()],
        old_turns[..],
        "the saved text"
    );
    assert!(new_turns.len() > old_turns.len(), "the new text");
    // The saved prefix read the copy: only the new audio was recognized.
    assert_eq!(calls(&r) - before, windows_of(&r, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_with_the_old_audio_copy_and_no_removal_saves_all_old_and_new_audio() {
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
    let files = ["audio/segment-01.opus", "audio/segment-02.opus"];
    let old: Vec<Vec<u8>> = files
        .iter()
        .map(|f| std::fs::read(folder.join(f)).unwrap())
        .collect();

    resume_saved_with_copy(&r, "s");
    for (file, bytes) in files.iter().zip(&old) {
        assert_eq!(&std::fs::read(r.root.join("s").join(file)).unwrap(), bytes);
    }
    record_resumed_segment(&r, "s", &speech());
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);

    for (file, bytes) in files.iter().zip(&old) {
        assert_eq!(&std::fs::read(folder.join(file)).unwrap(), bytes, "{file}");
    }
    assert!(folder.join("audio/segment-03.opus").is_file());
    assert_eq!(
        audio_state_of(&r, "s"),
        session_core::folder::AudioState::Present
    );
    assert!(!r.root.join("s").exists(), "the working copy is gone");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_old_file_deleted_before_its_marker_syncs_is_not_put_back_by_the_save() {
    let r = rig().await;
    let folder = saved_note(&r, None).await;
    resume_saved_with_copy(&r, "s");
    // The sync delivers the delete of the other computer first.
    std::fs::remove_file(folder.join("audio/segment-01.opus")).unwrap();
    record_resumed_segment(&r, "s", &speech());
    assert_eq!(r.runner.run_next().await.unwrap().1, Outcome::Finished);

    assert!(
        !folder.join("audio/segment-01.opus").exists(),
        "old audio came back"
    );
    assert!(folder.join("audio/segment-02.opus").is_file());
    // The marker comes after the save.
    write_marker(&folder, "audio/segment-01.opus");
    assert_eq!(
        audio_state_of(&r, "s"),
        session_core::folder::AudioState::Removed
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_resumed_note_deletes_the_copy_of_its_marked_old_audio_only() {
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
    resume_saved_with_copy(&r, "s");
    record_resumed_segment(&r, "s", &speech());
    remove_on_other_computer(&folder, "audio/segment-01.opus");
    *r.script.fail_on.lock().unwrap() = Some(calls(&r));
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert!(matches!(outcome, Outcome::Failed { .. }), "{outcome:?}");
    assert!(matches!(
        state(&r.store, "s"),
        SessionState::ProcessingFailed { .. }
    ));
    let working = r.root.join("s");
    assert!(
        !working.join("audio/segment-01.opus").exists(),
        "the marked copy stayed"
    );
    assert!(
        working.join("audio/segment-02.opus").is_file(),
        "the unmarked copy went"
    );
    assert!(
        working.join("audio/segment-03.opus").is_file(),
        "the new audio went"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sync_marker_for_a_failed_resumed_note_deletes_its_old_audio_copy() {
    // The library reconcile does not report a note that failed processing
    // as changed, so a removal that a sync delivers after the failure goes
    // through the list of all notes that failed processing (KTD8).
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
    resume_saved_with_copy(&r, "s");
    record_resumed_segment(&r, "s", &speech());
    *r.script.fail_on.lock().unwrap() = Some(calls(&r));
    let (_, outcome) = r.runner.run_next().await.unwrap();
    assert!(matches!(outcome, Outcome::Failed { .. }), "{outcome:?}");
    let working = r.root.join("s");
    assert!(working.join("audio/segment-01.opus").is_file());
    // A sync delivers the removal from another computer after the failure.
    remove_on_other_computer(&folder, "audio/segment-01.opus");
    r.runner.drop_removed_old_audio_of_failed();
    assert!(
        !working.join("audio/segment-01.opus").exists(),
        "the marked copy stayed"
    );
    assert!(
        working.join("audio/segment-02.opus").is_file(),
        "the unmarked copy went"
    );
    assert!(
        working.join("audio/segment-03.opus").is_file(),
        "the new audio went"
    );
}
