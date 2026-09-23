//! The runner end to end over real chunk files, with a scripted recognizer.

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use asr::{AsrError, AsrSegment, Recognizer, WindowText};
use common::{queue_session, speech, store};
use pipeline::runner::Outcome;
use pipeline::transcribe::{self, WindowPlan};
use pipeline::{EngineProvider, Environment, Runner, RunnerEvent, SharedStore, checkpoint};
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
        let ms = pcm.len() as u64 / 16;
        Ok(WindowText {
            text: format!("call {n}"),
            segments: vec![AsrSegment {
                start_ms: 0,
                end_ms: ms,
                text: format!("call {n}"),
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
}

struct Rig {
    runner: Arc<Runner>,
    store: SharedStore,
    script: Arc<Script>,
    events: Arc<Mutex<Vec<RunnerEvent>>>,
    _root: tempfile::TempDir,
    root: std::path::PathBuf,
}

fn rig_with(store: SharedStore, root: tempfile::TempDir, script: Arc<Script>) -> Rig {
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
    }
}

fn rig() -> Rig {
    rig_with(store(), tempfile::tempdir().unwrap(), Arc::default())
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
    let r = rig();
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
            ("earlier".into(), Stage::Transcribe),
            ("later".into(), Stage::Assemble),
            ("later".into(), Stage::Transcribe),
        ]
    );
    // One engine load serves both sessions.
    assert_eq!(*r.script.loads.lock().unwrap(), vec!["m".to_string()]);
    let results = transcribe::results(&r.store, "earlier").unwrap();
    assert_eq!(results.len(), plan(&r.store, "earlier").windows.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_mid_segment_stops_at_the_next_window_and_keeps_what_is_done() {
    let r = rig();
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
    let r = rig();
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
    let r = rig();
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
    let runner = Runner::new(Environment {
        store: Arc::clone(&store),
        working_root: root.path().to_path_buf(),
        engines: Box::new(Provider(Arc::default())),
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
    let r = rig();
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
