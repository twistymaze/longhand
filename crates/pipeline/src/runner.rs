//! The stage machine (R20c, KTD20). One runner per process polls the FIFO,
//! claims the head session, and runs its stages in order on the blocking
//! pool, one stage of one session at a time. Each stage checkpoints as it
//! goes and consults a [`Control`] at every checkpoint boundary, which is
//! where cancellation (a resume took the session back, or the app is
//! quitting) and suspension (capture is active) take effect.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use asr::Recognizer;
use chrono::{DateTime, FixedOffset, Local};
use session_core::jobs::Claims;
use session_core::{Event, SessionState, Stage};
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::diarize::Speakers;
use crate::folder_rename::{self, FolderRename};
use crate::live::{EngineAccess, EngineSlot};
use crate::save;
use crate::summarize::{self, SummaryEndpoint};
use crate::transcribe::{self, TurnCuts};
use crate::{Control, SharedStore, StageError, assemble, checkpoint, join, lock, verify};
use diarize::{DiarizeError, SpeakerAnalyzer};
use library::reconcile::{Changes, Reconciler, Target};

/// The stages, in order.
pub const STAGES: &[Stage] = &[
    Stage::Assemble,
    Stage::Diarize,
    Stage::Transcribe,
    Stage::Join,
    Stage::Verify,
    Stage::Summarize,
    Stage::Save,
];

const POLL: Duration = Duration::from_secs(2);
const HEARTBEAT: Duration = Duration::from_secs(10);
const SUSPENDED_POLL: Duration = Duration::from_millis(250);

/// Where recognition engines come from.
pub trait EngineProvider: Send + Sync {
    /// Resolve the exact artifact selected in the note's immutable snapshot.
    fn artifact_path(
        &self,
        artifact: &models::recognition::ArtifactSpec,
    ) -> Result<PathBuf, String> {
        Err(format!("exact artifact is unavailable: {}", artifact.id))
    }
    fn load_exact(
        &self,
        _spec: &models::recognition::ExecutionSpec,
    ) -> Result<Box<dyn Recognizer>, String> {
        Err("exact transcription runtime is unavailable".into())
    }
    fn analyzer_exact(
        &self,
        _spec: &models::recognition::ExecutionSpec,
    ) -> Result<Box<dyn SpeakerAnalyzer>, DiarizeError> {
        Err(DiarizeError::MissingWeights(
            "exact speaker artifacts are unavailable".into(),
        ))
    }
    fn nemo_helper(&self) -> Result<PathBuf, String> {
        Err("packaged NeMo helper is unavailable".into())
    }
    fn nemo_gpu(&self) -> i32 {
        -1
    }

    /// A cheap readiness check. Exact loaders still validate artifact bytes.
    fn ready_exact(&self, spec: &models::recognition::ExecutionSpec) -> bool {
        spec.artifacts.iter().all(|a| self.artifact_path(a).is_ok())
    }
    /// Whether everything processing needs for `model_id` is on disk.
    fn ready(&self, model_id: &str) -> bool;
    fn load(&self, model_id: &str) -> Result<Box<dyn Recognizer>, String>;
    /// The model for a session that recorded none.
    fn default_model(&self) -> String;
    /// The speaker-separation stack, loaded for each diarize stage and
    /// dropped when it ends, so its memory is free before transcription.
    fn analyzer(&self) -> Result<Box<dyn SpeakerAnalyzer>, DiarizeError>;
    /// The summarization endpoint, read fresh for each summarize stage so a
    /// settings change applies to the next session to reach it.
    fn summary_endpoint(&self) -> Result<SummaryEndpoint, EndpointProblem>;
    /// A snapshot of the active summary prompt, read fresh for each
    /// summarize stage like the endpoint. A run that saved a snapshot
    /// before uses its saved one (KTD4).
    fn summary_prompt(&self) -> summarize::Prompt;
    /// Remember that `model` at `base_url` takes no more than `limit` tokens.
    fn learn_limit(&self, base_url: &str, model: &str, limit: u32);
    /// The destination folder chosen now. It is read when the session is saved,
    /// so a change during recording saves the session in the last one (AE7).
    fn destination(&self) -> Option<PathBuf>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointProblem {
    /// No endpoint is set, or its settings are unusable.
    Unconfigured(String),
    /// The secret store is locked; the user can unlock it (R20b).
    Locked(String),
}

/// What the runner needs from the application.
pub struct Environment {
    pub store: SharedStore,
    /// The directory holding one working folder per session.
    pub working_root: PathBuf,
    pub engines: Box<dyn EngineProvider>,
    /// True while inference must wait: capture is active (KTD20).
    pub suspended: Box<dyn Fn() -> bool + Send + Sync>,
    pub events: Box<dyn Fn(RunnerEvent) + Send + Sync>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerEvent {
    StageStarted {
        session_id: String,
        stage: Stage,
    },
    /// Per window of the transcribe stage.
    Progress {
        session_id: String,
        stage: Stage,
        segment: u32,
        done: usize,
        total: usize,
    },
    StageFailed {
        session_id: String,
        stage: Stage,
        message: String,
    },
    Cancelled {
        session_id: String,
    },
    /// Diarization failed; the transcript goes ahead without speaker marks.
    SpeakersUnavailable {
        session_id: String,
        message: String,
    },
    /// The credential store is locked; the session waits for the user.
    Blocked {
        session_id: String,
        message: String,
    },
    /// A model is missing; the session keeps its place in the queue.
    Waiting {
        session_id: String,
        what: String,
    },
    /// Summarization failed; the session is saved with a stub summary (KD13).
    SummaryUnavailable {
        session_id: String,
        message: String,
    },
    /// The destination cannot take the session; it waits, whole, in the
    /// working location (R22b).
    AwaitingDestination {
        session_id: String,
        problem: String,
    },
    /// In the destination under its final name. `copied` when the volumes
    /// differed and the session was copied rather than moved in one step.
    Saved {
        session_id: String,
        path: PathBuf,
        copied: bool,
    },
    /// A Transcribe Again run failed before its save. The session is saved
    /// with its previous transcript and summary (R19).
    TranscribeAgainFailed {
        session_id: String,
        stage: Stage,
        message: String,
    },
    /// Every stage has run.
    Finished {
        session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Finished,
    Failed { stage: Stage, message: String },
    Cancelled,
    Waiting,
    Blocked,
    AwaitingDestination { problem: String },
}

pub struct Runner {
    env: Arc<Environment>,
    owner: String,
    wake: Notify,
    current: Mutex<Option<(String, CancellationToken)>>,
    shutdown: CancellationToken,
    /// The one loaded engine, kept across sessions (KTD6) and shared with
    /// the live worker (KTD9).
    engine: EngineSlot,
    /// One stage of one session at a time, whoever asks (R20c).
    permit: Arc<Semaphore>,
    /// Summary regenerations and deletes that run on a session. They
    /// exclude each other, and a folder rename waits for both (R8, KTD8).
    claims: Claims,
    power: power::ProcessingPower,
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

struct StageControl {
    cancel: CancellationToken,
    env: Arc<Environment>,
    session_id: String,
    power: ActivePower,
}

impl StageControl {
    /// A resume moves the session out of processing however it arrives;
    /// work on it stops at the next boundary even without a `cancel`.
    fn taken_back(&self) -> bool {
        !matches!(
            lock(&self.env.store).session(&self.session_id),
            Ok(Some(record)) if matches!(record.state, SessionState::Processing { .. })
        )
    }
}

impl Control for StageControl {
    fn proceed(&self) -> Result<(), StageError> {
        loop {
            if self.cancel.is_cancelled() || self.taken_back() {
                return Err(StageError::Cancelled);
            }
            if !(self.env.suspended)() {
                self.power.resume();
                return Ok(());
            }
            self.power.suspend();
            std::thread::sleep(SUSPENDED_POLL);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled() || self.taken_back()
    }
}

impl Runner {
    pub fn new(env: Environment) -> Arc<Self> {
        Self::with_power(env, power::ProcessingPower::default())
    }

    /// Use the supplied owner for processing assertions.
    pub fn with_power(env: Environment, power: power::ProcessingPower) -> Arc<Self> {
        let owner = format!(
            "{}-{}",
            std::process::id(),
            now().timestamp_nanos_opt().unwrap_or_default()
        );
        Arc::new(Self {
            env: Arc::new(env),
            owner,
            wake: Notify::new(),
            current: Mutex::new(None),
            shutdown: CancellationToken::new(),
            engine: Arc::new(Mutex::new(None)),
            permit: Arc::new(Semaphore::new(1)),
            claims: Claims::default(),
            power,
        })
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Launch sweep: return sessions an earlier process was working on to
    /// the queue, checkpoints intact.
    pub fn reclaim(
        &self,
        liveness: session_core::jobs::Liveness,
    ) -> Result<Vec<String>, StageError> {
        // Summary regeneration has no job heartbeat. An uncertain instance
        // lock cannot prove that its publication owner has stopped.
        if matches!(liveness, session_core::jobs::Liveness::OnlyInstance)
            && let Err(error) = save::recover_summary_publications(&self.env.store)
        {
            log::warn!("could not recover summary publications: {error}");
        }
        Ok(lock(&self.env.store).reclaim_at_launch(&self.owner, liveness)?)
    }

    /// Something changed that may let work start: a session was queued,
    /// capture ended, a model finished downloading.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Stop work on `session_id` at its next checkpoint (a resume took it
    /// back). Completed checkpoints stand.
    pub fn cancel(&self, session_id: &str) {
        if let Some((id, token)) = self
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            && id == session_id
        {
            token.cancel();
        }
    }

    /// The user's Retry on a failed session: put it back in line, keeping
    /// its place and its checkpoints. A failed verification first forgets
    /// the windows that missed speech, so only those are transcribed again.
    pub fn retry(&self, session_id: &str) -> Result<(), StageError> {
        let failed = lock(&self.env.store).session(session_id)?.map(|r| r.state);
        if let Some(SessionState::ProcessingFailed { stage, .. }) = failed
            && stage == Stage::Verify
        {
            if crate::recognition::snapshot(&self.env.store, session_id)?.is_some() {
                crate::recognition::prepare_retry(&self.env.store, session_id)?;
            } else {
                verify::prepare_retry(&self.env.store, session_id)?;
            }
        }
        {
            let mut store = lock(&self.env.store);
            store.apply(session_id, &Event::Retry)?;
            store.apply(session_id, &Event::ClaimLost)?;
        }
        self.wake();
        Ok(())
    }

    /// The secret store is available again: a blocked session goes back to
    /// its place in line and resumes at its checkpoints (R20b).
    pub fn unblock(&self, session_id: &str) -> Result<(), StageError> {
        let blocked = matches!(
            lock(&self.env.store).session(session_id)?.map(|r| r.state),
            Some(SessionState::Blocked { .. })
        );
        if blocked {
            lock(&self.env.store).apply(session_id, &Event::ClaimLost)?;
            self.wake();
        }
        Ok(())
    }

    /// The destination changed or came back: every session waiting for one
    /// goes back in line (R22b).
    pub fn retry_awaiting(&self) -> Result<Vec<String>, StageError> {
        let waiting = lock(&self.env.store).sessions_in("awaiting_destination")?;
        let ids: Vec<String> = waiting.into_iter().map(|r| r.id).collect();
        for id in &ids {
            self.retry(id)?;
        }
        Ok(ids)
    }

    /// Summarize a saved session again, replacing its `summary.md` (R19,
    /// AE17). Takes its turn with the stages, one at a time (R20c).
    pub async fn regenerate_summary(&self, session_id: &str) -> Result<PathBuf, StageError> {
        let _claim = self.claims.regenerate(session_id).ok_or_else(|| {
            StageError::Failed(
                "This note is being deleted, so its summary cannot be made again.".into(),
            )
        })?;
        let _permit = self
            .permit
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| StageError::Failed(e.to_string()))?;
        let env = Arc::clone(&self.env);
        let cancel = self.shutdown.child_token();
        let id = session_id.to_string();
        let power = self.power.clone();
        tokio::task::spawn_blocking(move || {
            let control = Unattended {
                cancel,
                env: Arc::clone(&env),
                power: ActivePower::new(power),
            };
            control.proceed()?;
            let endpoint = endpoint(&env)?;
            let (base_url, model_id) = (endpoint.base_url.clone(), endpoint.model.clone());
            save::regenerate_summary(
                &env.store,
                &id,
                &env.working_root,
                endpoint,
                env.engines.summary_prompt(),
                &|limit| env.engines.learn_limit(&base_url, &model_id, limit),
                &control,
                &mut |done, total| {
                    (env.events)(RunnerEvent::Progress {
                        session_id: id.clone(),
                        stage: Stage::Summarize,
                        segment: 0,
                        done,
                        total,
                    })
                },
            )
        })
        .await
        .map_err(|e| StageError::Failed(format!("the summary stopped unexpectedly: {e}")))?
    }

    /// The user's Transcribe Again on a saved session (R15, KTD14): queue it
    /// for a full recognition pass with `model`, the model selected now.
    /// Refused while the session's summary is regenerated or the session is
    /// deleted, while a capture is active, and when the session's folder
    /// does not hold every segment's audio.
    pub fn transcribe_again(&self, session_id: &str, model: &str) -> Result<(), StageError> {
        let regenerating = self.claims.busy(session_id);
        let capture_active = (self.env.suspended)();
        {
            let mut store = lock(&self.env.store);
            // The row follows its folder first (KTD3): the run keeps the
            // notes and the title that another computer wrote, and decodes
            // the segments that the folder holds.
            if !regenerating {
                crate::follow::follow_folder(&mut store, session_id, false)?;
            }
            let audio_intact = save::home_intact(&store, session_id)?;
            let refusal = if regenerating {
                Some("The summary of this note is being made again. Try again when it is done.")
            } else if capture_active {
                Some("A recording is in progress. Try again after it stops.")
            } else if !audio_intact {
                Some(
                    "The note's folder does not have all of its audio, so it can't be transcribed again.",
                )
            } else {
                None
            };
            if let Some(refusal) = refusal {
                return Err(StageError::Failed(refusal.into()));
            }
            store.transcribe_again(
                session_id,
                model,
                &Event::UserTranscribeAgain {
                    at: now(),
                    audio_intact,
                    capture_active,
                    regenerating,
                },
            )?;
            if let Some(home) = store.session(session_id)?.and_then(|r| r.home_path) {
                let hash = session_core::folder::fingerprint(&std::fs::read(
                    std::path::Path::new(&home).join("session.md"),
                )?);
                store.set_result_state(
                    session_id,
                    "replacement-base",
                    &serde_json::to_value((home, hash))?,
                )?;
            }
        }
        log::info!("{session_id} is queued to be transcribed again with '{model}'");
        self.wake();
        Ok(())
    }

    pub fn preview_reprocess(
        &self,
        session_id: &str,
        config: &models::recognition::RecognitionConfig,
    ) -> Result<crate::reprocess::Preview, StageError> {
        let store = lock(&self.env.store);
        let home = store
            .session(session_id)?
            .and_then(|r| r.home_path)
            .ok_or_else(|| StageError::Failed("the note has no saved folder".into()))?;
        crate::reprocess::preview(std::path::Path::new(&home), config)
    }

    pub fn reprocess(
        &self,
        session_id: &str,
        preview: &crate::reprocess::Preview,
        allow_transcription: bool,
    ) -> Result<(), StageError> {
        let mut store = lock(&self.env.store);
        let record = store
            .session(session_id)?
            .ok_or_else(|| StageError::Failed("the note is absent".into()))?;
        let home = record
            .home_path
            .ok_or_else(|| StageError::Failed("the note has no saved folder".into()))?;
        let fresh =
            crate::reprocess::preview(std::path::Path::new(&home), &preview.execution.config)?;
        if fresh.manifest != preview.manifest || fresh.execution != preview.execution {
            return Err(StageError::Failed(
                "the note or model configuration changed; review the replacement again".into(),
            ));
        }
        if fresh.requires_transcription && !allow_transcription {
            return Err(StageError::Failed(fresh.explanation));
        }
        let capture_active = (self.env.suspended)();
        let regenerating = self.claims.busy(session_id);
        let audio_intact = save::home_intact(&store, session_id)?;
        store.begin_reprocess(
            session_id,
            &fresh.execution.config.asr_model,
            &Event::UserTranscribeAgain {
                at: now(),
                audio_intact,
                capture_active,
                regenerating,
            },
            Some(&serde_json::to_value(&fresh.execution)?),
            Some(&serde_json::to_value((home, fresh.manifest))?),
        )?;
        drop(store);
        self.wake();
        Ok(())
    }

    /// True while `session_id` has its summary regenerated or waits to.
    pub fn is_regenerating(&self, session_id: &str) -> bool {
        self.claims.is_regenerating(session_id)
    }

    /// The claims that a delete takes (R8): see
    /// [`session_core::jobs::delete_session`].
    pub fn claims(&self) -> &Claims {
        &self.claims
    }

    /// The user's rename (R9): the title changes at once, and a saved, idle
    /// session's folder takes the new name now. A busy session's folder
    /// takes it when the work ends.
    pub fn rename_session(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<FolderRename, StageError> {
        folder_rename::rename_session(
            &self.env.store,
            session_id,
            title,
            &|id| self.claims.busy(id),
            &session_save::RealFs,
        )
    }

    /// At launch, before the library scan: settle the folder renames that a
    /// crash left (KTD8, KTD9).
    pub fn resolve_rename_intents(&self) -> Result<Vec<String>, StageError> {
        folder_rename::resolve_intents(&self.env.store, &session_save::RealFs)
    }

    /// Scan every note folder in `roots` and apply the scan to the store
    /// (KTD5, KTD6): notes from other computers are adopted, and known
    /// rows follow their folders. The scan runs without the store lock. A
    /// row that a regeneration or a delete claims is not refreshed or
    /// forgotten. `reconciler` keeps the pending deletes between scans.
    pub fn reconcile_library(
        &self,
        roots: &[PathBuf],
        reconciler: &mut Reconciler,
    ) -> Result<Changes, StageError> {
        Ok(
            reconciler.run(&self.env.store, &Target::Full(roots.to_vec()), &|id| {
                self.claims.busy(id)
            })?,
        )
    }

    /// At launch, after the library reconcile and before [`Runner::run`]:
    /// give every saved folder the name its title gives (KTD8, KTD9).
    pub fn reconcile_folders(&self) -> Result<(), StageError> {
        folder_rename::reconcile_saved(&self.env.store, &session_save::RealFs)
    }

    /// Stop at the next window and free the recognition engine. Called as
    /// the process exits: ggml's Metal backend asserts in its static
    /// destructor if a model is still loaded then, and the abort reads as a
    /// crash on every quit after a session was transcribed. Handy unloads on
    /// exit for the same reason. Waits for a window in progress to end.
    /// Stop the live workers first ([`crate::live::Workers::shutdown`]): a
    /// worker holds the engine slot while it streams.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
        if let Some((_, token)) = self
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            token.cancel();
        }
        self.engine.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    /// The live worker's way to the engine slot (KTD9). After
    /// [`Runner::shutdown`], it loads nothing.
    pub fn live_workers(self: &Arc<Self>) -> crate::live::Workers {
        let runner = Arc::downgrade(self);
        crate::live::Workers::new(self.env.store.clone(), self.engine_access()).with_recognition(
            Arc::new(move |store, config, cancel, end, sink| {
                let runner = runner.upgrade().ok_or(StageError::Cancelled)?;
                crate::live::recognize(
                    store,
                    config,
                    runner.env.engines.as_ref(),
                    cancel,
                    end,
                    sink,
                )
            }),
        )
    }

    pub fn engine_access(&self) -> EngineAccess {
        let env = Arc::clone(&self.env);
        let exact = Arc::clone(&self.env);
        EngineAccess::new(
            Arc::clone(&self.engine),
            Arc::new(move |model| env.engines.load(model)),
            self.shutdown.clone(),
        )
        .with_exact_loader(Arc::new(move |spec| exact.engines.load_exact(spec)))
    }

    /// Free the loaded engine when it holds `model`, so that the model file
    /// can be removed: Windows cannot remove an open file (KTD12). Returns
    /// whether the engine held `model`.
    pub fn unload_engine(&self, model: &str) -> bool {
        unload_if(
            &self.engine,
            model,
            <dyn Recognizer as Recognizer>::model_id,
        )
    }

    pub async fn run(self: Arc<Self>) {
        loop {
            // A session waiting on a model is back at the head of the queue;
            // claiming it again at once would spin until the model download finishes.
            while let Some((_, outcome)) = self.run_next().await
                && outcome != Outcome::Waiting
            {}
            tokio::select! {
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(POLL) => {}
                _ = self.shutdown.cancelled() => return,
            }
        }
    }

    /// Claim the first queued session whose models are on disk and run its
    /// stages. A session that waits for its model keeps its place, and the
    /// sessions after it go first (KTD13). `None` when nothing can start:
    /// the queue is empty, capture is active, or no queued session has its
    /// models.
    pub async fn run_next(&self) -> Option<(String, Outcome)> {
        if self.shutdown.is_cancelled() || (self.env.suspended)() {
            return None;
        }
        let (id, model) = {
            let mut store = lock(&self.env.store);
            let (next, model) = store.queue().ok()?.into_iter().find_map(|id| {
                let record = store.session(&id).ok()??;
                // A Transcribe Again run uses the model it recorded (KTD14).
                let again = store.transcribe_again_marks(&id).ok()?.model;
                let model = again
                    .or(record.asr_model)
                    .unwrap_or_else(|| self.env.engines.default_model());
                let ready = if let Some(value) = store
                    .result_state(&id, "pending-snapshot")
                    .ok()?
                    .or(record.recognition_snapshot)
                {
                    match serde_json::from_value::<models::recognition::ExecutionSpec>(value) {
                        Ok(spec) => self.env.engines.ready_exact(&spec),
                        // Claim invalid snapshots so their precise failure is visible.
                        Err(_) => true,
                    }
                } else {
                    self.env.engines.ready(&model)
                };
                ready.then_some((id, model))
            })?;
            let id = match store.dequeue_session(&next, &self.owner, now()) {
                Ok(Some(id)) => id,
                Ok(None) => return None,
                Err(e) => {
                    log::error!("could not claim {next}: {e}");
                    return None;
                }
            };
            (id, model)
        };
        let cancel = CancellationToken::new();
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((id.clone(), cancel.clone()));
        let outcome = self.run_session(&id, &model, cancel).await;
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Some((id, outcome))
    }

    async fn run_session(&self, id: &str, model: &str, cancel: CancellationToken) -> Outcome {
        let emit = |event| (self.env.events)(event);
        // Once saving has begun, the working directory may be gone: only
        // the save itself is left to finish.
        let saving = save::begun(&lock(&self.env.store), id).unwrap_or(false);
        let again = lock(&self.env.store)
            .transcribe_again_marks(id)
            .is_ok_and(|marks| marks.model.is_some());
        let snapshot = crate::recognition::snapshot(&self.env.store, id);
        let exact = matches!(&snapshot, Ok(Some(_)));
        let stages = match snapshot {
            Ok(Some(spec)) => match crate::recognition::TaskGraph::resolve(&spec) {
                Ok(graph) => graph.stages,
                Err(error) => {
                    return Outcome::Failed {
                        stage: Stage::Transcribe,
                        message: error.to_string(),
                    };
                }
            },
            Ok(None) => STAGES.to_vec(),
            Err(error) => {
                let message = error.to_string();
                let _ = lock(&self.env.store).apply(id, &Event::StageFailed(Stage::Transcribe));
                return Outcome::Failed {
                    stage: Stage::Transcribe,
                    message,
                };
            }
        };
        for stage in stages {
            if saving && stage != Stage::Save {
                continue;
            }
            let result = self.run_stage(id, model, stage, cancel.clone()).await;
            match result {
                Ok(()) => {}
                // A failed summary costs the summary, not the session: it
                // is saved with a stub naming the failure (KD13, R22).
                Err(StageError::Failed(message)) if stage == Stage::Summarize && !again => {
                    log::warn!("summarization failed for {id}; saving without it: {message}");
                    if let Err(e) = save::record_summary_failure(&self.env.store, id, &message) {
                        log::error!("could not record the summary failure of {id}: {e}");
                    }
                    emit(RunnerEvent::SummaryUnavailable {
                        session_id: id.to_string(),
                        message,
                    });
                }
                // A Transcribe Again run that fails keeps the saved files
                // (R19). Speaker separation failing costs the speaker
                // marks only, as for any run. A save that has replaced a
                // document in the folder fails as for any run: a retry
                // writes the folder again and then finishes the run.
                Err(StageError::Failed(message) | StageError::NoDestination(message))
                    if again
                        && (stage != Stage::Diarize || exact)
                        && !(stage == Stage::Save
                            && (save::home_changed(&lock(&self.env.store), id)
                                .unwrap_or(true)
                                || self
                                    .env
                                    .working_root
                                    .join(id)
                                    .join(session_save::result::JOURNAL)
                                    .exists())) =>
                {
                    return self.fail_transcribe_again(id, stage, message);
                }
                Err(StageError::NoDestination(problem)) => {
                    log::info!("{id} waits for its destination: {problem}");
                    if let Err(e) =
                        lock(&self.env.store).apply(id, &Event::SaveFailed(problem.clone()))
                    {
                        log::warn!("could not mark {id} awaiting its destination: {e}");
                    }
                    emit(RunnerEvent::AwaitingDestination {
                        session_id: id.to_string(),
                        problem: problem.clone(),
                    });
                    return Outcome::AwaitingDestination { problem };
                }
                // Speaker separation failing costs the speaker marks, not the
                // transcript (R20a).
                Err(StageError::Failed(message)) if stage == Stage::Diarize && !exact => {
                    log::warn!(
                        "diarization failed for {id}; continuing without speakers: {message}"
                    );
                    let segments = lock(&self.env.store).segments(id).map(|s| s.len());
                    let recorded = segments.map_err(StageError::from).and_then(|n| {
                        crate::diarize::record_failure(&self.env.store, id, n, &message)
                    });
                    if let Err(e) = recorded {
                        log::error!("could not record the diarization failure of {id}: {e}");
                    }
                    emit(RunnerEvent::SpeakersUnavailable {
                        session_id: id.to_string(),
                        message,
                    });
                }
                Err(StageError::Waiting(what)) => {
                    log::info!("{id} waits for {what}");
                    // Back to its place in line; it runs once the models are in.
                    if let Err(e) = lock(&self.env.store).apply(id, &Event::ClaimLost) {
                        log::warn!("could not return {id} to the queue: {e}");
                    }
                    emit(RunnerEvent::Waiting {
                        session_id: id.to_string(),
                        what: what.clone(),
                    });
                    return Outcome::Waiting;
                }
                Err(StageError::Blocked(message)) => {
                    log::info!("{id} is blocked on the secret store: {message}");
                    if let Err(e) =
                        crate::progress::record_error(&self.env.store, id, stage, &message)
                    {
                        log::warn!("could not record why {id} is blocked: {e}");
                    }
                    if let Err(e) = lock(&self.env.store).apply(id, &Event::CredentialUnavailable) {
                        log::warn!("could not mark {id} blocked: {e}");
                    }
                    emit(RunnerEvent::Blocked {
                        session_id: id.to_string(),
                        message,
                    });
                    return Outcome::Blocked;
                }
                Err(StageError::Cancelled) => {
                    log::info!("processing of {id} stopped at a checkpoint");
                    emit(RunnerEvent::Cancelled {
                        session_id: id.to_string(),
                    });
                    return Outcome::Cancelled;
                }
                Err(StageError::Failed(message)) => {
                    log::error!("{stage:?} failed for {id}: {message}");
                    if let Err(e) =
                        crate::progress::record_error(&self.env.store, id, stage, &message)
                    {
                        log::warn!("could not record the failure of {id}: {e}");
                    }
                    // A resume may have taken the session in the meantime;
                    // then there is nothing to mark.
                    if let Err(e) = lock(&self.env.store).apply(id, &Event::StageFailed(stage)) {
                        log::warn!("could not record the failure of {id}: {e}");
                    }
                    emit(RunnerEvent::StageFailed {
                        session_id: id.to_string(),
                        stage,
                        message: message.clone(),
                    });
                    return Outcome::Failed { stage, message };
                }
            }
        }
        log::info!("processing of {id} finished");
        emit(RunnerEvent::Finished {
            session_id: id.to_string(),
        });
        Outcome::Finished
    }

    /// Return a failed Transcribe Again run to Saved with its previous
    /// checkpoints, record the error for the note page, and remove the
    /// working directory that the run made. When a resume took the session
    /// first, the resume owns the working directory and nothing changes.
    fn fail_transcribe_again(&self, id: &str, stage: Stage, message: String) -> Outcome {
        log::error!("transcribing {id} again failed at {stage:?}: {message}");
        match lock(&self.env.store).fail_transcribe_again(id, &message) {
            Ok(_) => {
                let dir = self.env.working_root.join(id);
                if dir.exists()
                    && let Err(e) = std::fs::remove_dir_all(&dir)
                {
                    log::warn!("could not remove {}: {e}", dir.display());
                }
            }
            Err(e) => log::warn!("could not return {id} to its saved state: {e}"),
        }
        (self.env.events)(RunnerEvent::TranscribeAgainFailed {
            session_id: id.to_string(),
            stage,
            message: message.clone(),
        });
        Outcome::Failed { stage, message }
    }

    async fn run_stage(
        &self,
        id: &str,
        model: &str,
        stage: Stage,
        cancel: CancellationToken,
    ) -> Result<(), StageError> {
        let segments = lock(&self.env.store).segments(id)?;
        if (checkpoint::is_complete(&self.env.store, id, stage, segments.len())?
            || matches!(stage, Stage::Summarize | Stage::Save))
            && matches!(
                stage,
                Stage::Diarize
                    | Stage::Transcribe
                    | Stage::Join
                    | Stage::Verify
                    | Stage::Summarize
                    | Stage::Save
            )
            && let Some(spec) = crate::recognition::snapshot(&self.env.store, id)?
            && self.env.working_root.join(id).exists()
        {
            crate::recognition::durable::prepare(
                &self.env.store,
                id,
                &self.env.working_root.join(id),
                &segments,
                &spec,
            )?;
        }
        if checkpoint::is_complete(&self.env.store, id, stage, segments.len())? {
            let exact = crate::recognition::snapshot(&self.env.store, id)?.is_some();
            let route_complete = match stage {
                Stage::Assemble if exact => crate::assemble::provenance_valid(
                    &self.env.store,
                    id,
                    &self.env.working_root.join(id),
                    &segments,
                )?,
                Stage::Transcribe if exact => {
                    crate::recognition::saved_result(&self.env.store, id)?.is_some()
                }
                Stage::Diarize if exact => checkpoint::get::<serde_json::Value>(
                    &self.env.store,
                    id,
                    stage,
                    crate::recognition::SPEAKERS_KEY,
                )?
                .is_some(),
                _ => true,
            };
            if route_complete {
                return Ok(());
            }
        }
        let _permit = self
            .permit
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| StageError::Failed(e.to_string()))?;
        lock(&self.env.store).set_stage(id, stage)?;
        (self.env.events)(RunnerEvent::StageStarted {
            session_id: id.to_string(),
            stage,
        });

        let heartbeat = {
            let store = Arc::clone(&self.env.store);
            let (id, owner) = (id.to_string(), self.owner.clone());
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(HEARTBEAT).await;
                    if let Err(e) = lock(&store).heartbeat(&id, &owner, now()) {
                        log::warn!("heartbeat for {id} failed: {e}");
                    }
                }
            })
        };

        let env = Arc::clone(&self.env);
        let engine = Arc::clone(&self.engine);
        let (id_owned, model) = (id.to_string(), model.to_string());
        let power = self.power.clone();
        let result = tokio::task::spawn_blocking(move || {
            let control = StageControl {
                cancel,
                env: Arc::clone(&env),
                session_id: id_owned.clone(),
                power: ActivePower::new(power),
            };
            control.proceed()?;
            execute(&env, &engine, &id_owned, &model, stage, &segments, &control)
        })
        .await;
        heartbeat.abort();
        match result {
            Ok(result) => result,
            Err(e) => Err(StageError::Failed(format!(
                "the stage stopped unexpectedly: {e}"
            ))),
        }
    }
}

fn execute(
    env: &Environment,
    engine: &Mutex<Option<Box<dyn Recognizer>>>,
    id: &str,
    model: &str,
    stage: Stage,
    segments: &[session_core::Segment],
    control: &dyn Control,
) -> Result<(), StageError> {
    let dir = env.working_root.join(id);
    if matches!(
        stage,
        Stage::Diarize | Stage::Transcribe | Stage::Join | Stage::Verify
    ) && let Some(spec) = crate::recognition::snapshot(&env.store, id)?
    {
        return crate::recognition::execute(
            &env.store,
            id,
            &dir,
            segments,
            &spec,
            env.engines.as_ref(),
            stage,
            control,
            &mut |p| {
                (env.events)(RunnerEvent::Progress {
                    session_id: id.into(),
                    stage,
                    segment: p.segment,
                    done: p.windows_done,
                    total: p.windows_total,
                });
            },
        );
    }
    match stage {
        Stage::Assemble => {
            let home = lock(&env.store)
                .session(id)?
                .and_then(|record| record.home_path)
                .map(std::path::PathBuf::from);
            assemble::run(&env.store, id, &dir, segments, home.as_deref(), control).map(|_| ())
        }
        Stage::Diarize => {
            let mut analyzer = env.engines.analyzer().map_err(|e| match e {
                DiarizeError::MissingWeights(path) => StageError::Waiting(path),
                other => StageError::Failed(other.to_string()),
            })?;
            let count = lock(&env.store)
                .session(id)?
                .and_then(|record| record.speaker_count);
            let mut last = None;
            crate::diarize::run(
                &env.store,
                id,
                &dir,
                segments,
                count,
                analyzer.as_mut(),
                control,
                &mut |segment, fraction| {
                    let percent = (fraction * 100.0) as usize;
                    if last != Some((segment, percent)) {
                        last = Some((segment, percent));
                        (env.events)(RunnerEvent::Progress {
                            session_id: id.to_string(),
                            stage,
                            segment,
                            done: percent,
                            total: 100,
                        });
                    }
                },
            )
        }
        Stage::Transcribe => {
            let cuts: TurnCuts = match crate::diarize::speakers(&env.store, id)? {
                Speakers::Turns(turns) => segments
                    .iter()
                    .map(|s| (s.index, diarize::turn_cuts(&turns, s.index)))
                    .collect(),
                Speakers::Unavailable => TurnCuts::new(),
            };
            // The stage locks the slot for each window only, so a live
            // worker can take the slot while the stage waits out a
            // suspension (KTD9).
            let slot = transcribe::Slot {
                slot: engine,
                model,
                load: &|model| env.engines.load(model),
            };
            transcribe::run(
                &env.store,
                id,
                &dir,
                segments,
                &cuts,
                &slot,
                control,
                &mut |p| {
                    (env.events)(RunnerEvent::Progress {
                        session_id: id.to_string(),
                        stage,
                        segment: p.segment,
                        done: p.windows_done,
                        total: p.windows_total,
                    })
                },
            )
        }
        Stage::Join => join::run(&env.store, id, &dir, segments).map(|_| ()),
        Stage::Verify => verify::run(&env.store, id, &dir, segments),
        Stage::Summarize => {
            let endpoint = endpoint(env)?;
            let (base_url, model_id) = (endpoint.base_url.clone(), endpoint.model.clone());
            summarize::run(
                &env.store,
                id,
                &dir,
                segments,
                endpoint,
                env.engines.summary_prompt(),
                &|limit| env.engines.learn_limit(&base_url, &model_id, limit),
                control,
                &mut |done, total| {
                    (env.events)(RunnerEvent::Progress {
                        session_id: id.to_string(),
                        stage,
                        segment: 0,
                        done,
                        total,
                    })
                },
            )
        }
        Stage::Save => {
            let destination = env.engines.destination();
            let saved = save::run(
                &env.store,
                id,
                &dir,
                segments,
                destination.as_deref(),
                &session_save::RealFs,
                control,
            )?;
            log::info!(
                "{id} saved at {}{}",
                saved.path.display(),
                if saved.copied {
                    " (copied across volumes)"
                } else {
                    ""
                }
            );
            (env.events)(RunnerEvent::Saved {
                session_id: id.to_string(),
                path: saved.path,
                copied: saved.copied,
            });
            Ok(())
        }
    }
}

fn endpoint(env: &Environment) -> Result<SummaryEndpoint, StageError> {
    env.engines.summary_endpoint().map_err(|p| match p {
        EndpointProblem::Locked(message) => StageError::Blocked(message),
        EndpointProblem::Unconfigured(message) => StageError::Failed(message),
    })
}

/// Empty `slot` when it holds `model`. A slot that a stage holds now stays
/// as it is: the stage runs the model of a note in processing, and delete
/// refuses that model, so the stage loads another model into the slot.
fn unload_if<E: ?Sized>(
    slot: &Mutex<Option<Box<E>>>,
    model: &str,
    model_id: impl Fn(&E) -> &str,
) -> bool {
    let mut slot = match slot.try_lock() {
        Ok(slot) => slot,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return false,
    };
    if slot.as_deref().map(model_id) == Some(model) {
        *slot = None;
        true
    } else {
        false
    }
}

/// For work on a saved session: stops on shutdown and waits out capture,
/// with no processing state to be taken back from.
struct Unattended {
    cancel: CancellationToken,
    env: Arc<Environment>,
    power: ActivePower,
}

impl Control for Unattended {
    fn proceed(&self) -> Result<(), StageError> {
        loop {
            if self.cancel.is_cancelled() {
                return Err(StageError::Cancelled);
            }
            if !(self.env.suspended)() {
                self.power.resume();
                return Ok(());
            }
            self.power.suspend();
            std::thread::sleep(SUSPENDED_POLL);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// A blocking job releases its lease at a suspension boundary. Dropping
/// the control also releases it on failure, cancellation, or unwind.
struct ActivePower {
    owner: power::ProcessingPower,
    lease: Mutex<Option<power::ProcessingLease>>,
}

impl ActivePower {
    fn new(owner: power::ProcessingPower) -> Self {
        Self {
            owner,
            lease: Mutex::new(None),
        }
    }

    fn resume(&self) {
        let mut lease = self.lease.lock().unwrap_or_else(|error| error.into_inner());
        if lease.is_none() {
            *lease = Some(self.owner.acquire());
        }
    }

    fn suspend(&self) {
        self.lease
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(model: &str) -> &str {
        model
    }

    fn slot(model: Option<&str>) -> Mutex<Option<Box<str>>> {
        Mutex::new(model.map(Box::from))
    }

    #[test]
    fn unload_frees_the_engine_that_holds_the_model() {
        let engine = slot(Some("whisper"));
        assert!(unload_if(&engine, "whisper", id));
        assert_eq!(*engine.lock().unwrap(), None);
    }

    #[test]
    fn unload_keeps_an_engine_that_holds_another_model() {
        let engine = slot(Some("parakeet"));
        assert!(!unload_if(&engine, "whisper", id));
        assert_eq!(engine.lock().unwrap().as_deref(), Some("parakeet"));
        assert!(!unload_if(&slot(None), "whisper", id));
    }

    #[test]
    fn unload_does_not_wait_for_a_stage_that_holds_the_engine() {
        let engine = slot(Some("whisper"));
        let _stage = engine.lock().unwrap();
        assert!(!unload_if(&engine, "whisper", id));
    }
}
