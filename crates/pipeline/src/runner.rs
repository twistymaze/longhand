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
use crate::progress::{SpeakerMeter, SpeakerProgress, SpeakerWork};
use crate::reprocess::{self, Guards, Readiness, Refusal};
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
    /// before uses its saved one (KTD4). The first automatic summary of a
    /// note that recorded its prompt at the start uses that prompt instead
    /// (KTD15). Summarize Again always uses this prompt.
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
    /// The speaker stage's progress over the whole note (KTD1). A new
    /// attempt replaces the progress of an earlier one.
    SpeakerProgress {
        session_id: String,
        progress: crate::progress::SpeakerProgress,
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
        // While the Move holds the note for its final check, the
        // regeneration waits (KTD9).
        let _claim = loop {
            if let Some(claim) = self.claims.regenerate(session_id) {
                break claim;
            }
            if !self.claims.is_held(session_id) {
                return Err(StageError::Failed(
                    "This note is being deleted, so its summary cannot be made again.".into(),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        };
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

    /// Check if the user's Transcribe Again can start on a saved note now
    /// (KTD2), and give the execution of the run: `settings` with After
    /// Stop timing and the speaker count of the note. This is read-only.
    /// `ready` tells if the artifacts of the run are on disk.
    pub fn check_transcribe_again(
        &self,
        session_id: &str,
        settings: &models::recognition::RecognitionConfig,
        ready: &dyn Fn(&models::recognition::ExecutionSpec) -> Readiness,
    ) -> Result<models::recognition::ExecutionSpec, Refusal> {
        let guards = self.guards(session_id);
        reprocess::check(&lock(&self.env.store), session_id, guards, settings, ready)
    }

    /// The user's Transcribe Again on a saved note (R15, KTD14): queue it
    /// for a run with the execution that [`Runner::check_transcribe_again`]
    /// gives. Every check runs again after the row follows its folder.
    /// Refused when the settings no longer select `model_id`, the model that
    /// the user confirmed.
    pub fn transcribe_again(
        &self,
        session_id: &str,
        settings: &models::recognition::RecognitionConfig,
        model_id: &str,
        ready: &dyn Fn(&models::recognition::ExecutionSpec) -> Readiness,
    ) -> Result<(), Refusal> {
        let guards = self.guards(session_id);
        let spec = reprocess::queue(
            &mut lock(&self.env.store),
            session_id,
            guards,
            settings,
            model_id,
            ready,
        )?;
        log::info!(
            "{session_id} is queued to be transcribed again with '{}'",
            spec.config.asr_model
        );
        self.wake();
        Ok(())
    }

    /// The state outside the store that refuses a Transcribe Again run.
    /// Call this before the store lock. `suspended` locks the capture state,
    /// and the capture manager locks its state and then the store.
    fn guards(&self, session_id: &str) -> Guards {
        Guards {
            regenerating: self.claims.is_regenerating_or_deleting(session_id),
            capture_active: (self.env.suspended)(),
        }
    }

    /// True while a stage or a summary regeneration runs. A session that
    /// waits in the queue does not count. This reads only the permit, not
    /// the store, so a caller can hold any lock.
    pub fn is_working(&self) -> bool {
        self.permit.available_permits() == 0
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
        // The rename waits while the Move holds the note (KTD9).
        let mut store = crate::move_notes::lock_unheld(&self.env.store, &self.claims, session_id);
        folder_rename::rename_locked(
            &mut store,
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
        let mut attempt = crate::recognition::durable::Attempt::default();
        for stage in stages {
            if saving && stage != Stage::Save {
                continue;
            }
            let result = self
                .run_stage(id, model, stage, cancel.clone(), &mut attempt)
                .await;
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
                                    .join(session_save::publication::RECORD)
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
                    self.drop_removed_old_audio(id);
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

    /// For a resumed note that failed processing: delete the copy of each
    /// old segment whose home file has a removal marker (KTD8 of the audio
    /// removal plan). Call it again when a sync delivers a removal marker
    /// for a note that failed processing.
    pub fn drop_removed_old_audio(&self, id: &str) {
        let home = {
            let store = lock(&self.env.store);
            match store.session(id) {
                Ok(Some(record))
                    if matches!(record.state, SessionState::ProcessingFailed { .. }) =>
                {
                    record.home_path
                }
                _ => None,
            }
        };
        let Some(home) = home else {
            return;
        };
        self.drop_removed_old_audio_at(id, &home);
    }

    /// Do `drop_removed_old_audio` for each note that failed processing.
    /// Call it after each library reconcile: the reconcile does not report
    /// a note that failed processing as changed, so a sync that delivers
    /// a removal marker for it is not visible in the changes.
    pub fn drop_removed_old_audio_of_failed(&self) {
        let failed: Vec<(String, String)> = {
            let store = lock(&self.env.store);
            match store.sessions_in("processing_failed") {
                Ok(records) => records
                    .into_iter()
                    .filter_map(|record| record.home_path.map(|home| (record.id, home)))
                    .collect(),
                Err(e) => {
                    log::warn!("could not list the notes that failed processing: {e}");
                    return;
                }
            }
        };
        for (id, home) in failed {
            self.drop_removed_old_audio_at(&id, &home);
        }
    }

    fn drop_removed_old_audio_at(&self, id: &str, home: &str) {
        let working = self.env.working_root.join(id);
        match crate::old_audio::drop_removed(&working, std::path::Path::new(&home)) {
            Ok(dropped) if !dropped.is_empty() => {
                log::info!(
                    "deleted the old audio copy of {dropped:?} of {id}: its audio was removed"
                );
            }
            Ok(_) => {}
            Err(e) => log::warn!("could not delete the removed old audio copy of {id}: {e}"),
        }
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
        attempt: &mut crate::recognition::durable::Attempt,
    ) -> Result<(), StageError> {
        // A new attempt does not show the progress of an earlier attempt.
        if stage == Stage::Diarize {
            lock(&self.env.store).delete_checkpoint(id, stage, crate::progress::SPEAKER_KEY)?;
        }
        let _permit = self
            .permit
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| StageError::Failed(e.to_string()))?;
        let control = StageControl {
            cancel,
            env: Arc::clone(&self.env),
            session_id: id.to_string(),
            power: ActivePower::new(self.power.clone()),
        };
        // Preparation reads and hashes audio before the stage starts.
        control.power.resume();
        let segments = lock(&self.env.store).segments(id)?;
        let mut prepared = None;
        if matches!(
            stage,
            Stage::Diarize
                | Stage::Transcribe
                | Stage::Join
                | Stage::Verify
                | Stage::Summarize
                | Stage::Save
        ) && let Some(spec) = crate::recognition::snapshot(&self.env.store, id)?
            && (stage != Stage::Save || self.env.working_root.join(id).exists())
        {
            attempt.prepare(
                &self.env.store,
                id,
                &self.env.working_root.join(id),
                &segments,
                &spec,
                stage == Stage::Save,
            )?;
            prepared = Some(spec);
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
        let result = tokio::task::spawn_blocking(move || {
            control.proceed()?;
            execute(
                &env,
                &engine,
                &id_owned,
                &model,
                stage,
                &segments,
                prepared.as_ref(),
                &control,
            )
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

#[allow(clippy::too_many_arguments)]
fn execute(
    env: &Environment,
    engine: &Mutex<Option<Box<dyn Recognizer>>>,
    id: &str,
    model: &str,
    stage: Stage,
    segments: &[session_core::Segment],
    prepared: Option<&models::recognition::ExecutionSpec>,
    control: &dyn Control,
) -> Result<(), StageError> {
    let dir = env.working_root.join(id);
    let mut report_speakers = speaker_sink(env, id);
    let mut speakers = SpeakerMeter::new(&mut report_speakers);
    if stage == Stage::Diarize {
        // The stage is active before it can measure (KTD3).
        speakers.unmeasured();
    }
    if matches!(
        stage,
        Stage::Diarize | Stage::Transcribe | Stage::Join | Stage::Verify
    ) && let Some(spec) = prepared
    {
        return crate::recognition::execute_prepared(
            &env.store,
            id,
            &dir,
            segments,
            spec,
            env.engines.as_ref(),
            stage,
            control,
            &mut speakers,
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
            speakers.plan(crate::diarize::units(segments), 0);
            crate::diarize::run(
                &env.store,
                id,
                &dir,
                segments,
                count,
                analyzer.as_mut(),
                control,
                &mut speakers,
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
            let prompt = first_summary_prompt(&env.store, id)?
                .unwrap_or_else(|| env.engines.summary_prompt());
            summarize::run(
                &env.store,
                id,
                &dir,
                segments,
                endpoint,
                prompt,
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

/// Save and send the speaker progress of a new attempt. The first report
/// replaces the progress of an earlier attempt in the store (KTD4).
fn speaker_sink<'a>(env: &'a Environment, id: &'a str) -> impl FnMut(SpeakerWork) + 'a {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let attempt = format!(
        "{}-{}",
        now().timestamp_nanos_opt().unwrap_or_default(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    move |work| {
        let progress = SpeakerProgress {
            attempt: attempt.clone(),
            work,
        };
        if let Err(e) = checkpoint::put(
            &env.store,
            id,
            Stage::Diarize,
            crate::progress::SPEAKER_KEY,
            &progress,
        ) {
            log::warn!("could not save the speaker progress of {id}: {e}");
        }
        (env.events)(RunnerEvent::SpeakerProgress {
            session_id: id.to_string(),
            progress,
        });
    }
}

fn endpoint(env: &Environment) -> Result<SummaryEndpoint, StageError> {
    env.engines.summary_endpoint().map_err(|p| match p {
        EndpointProblem::Locked(message) => StageError::Blocked(message),
        EndpointProblem::Unconfigured(message) => StageError::Failed(message),
    })
}

/// The prompt of the note's first automatic summary: the active prompt
/// when the note started (KTD15). `None` for a note with no snapshot, and
/// for a Transcribe Again run, which the user starts later, as Summarize
/// Again. Those use the active prompt.
fn first_summary_prompt(
    store: &SharedStore,
    id: &str,
) -> Result<Option<summarize::Prompt>, StageError> {
    let store = lock(store);
    if store.transcribe_again_marks(id)?.model.is_some() {
        return Ok(None);
    }
    Ok(store.prompt_snapshot(id)?.map(|p| summarize::Prompt {
        id: p.id,
        name: p.name,
        text: p.text,
    }))
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

    struct UnusedEngines;

    impl EngineProvider for UnusedEngines {
        fn ready(&self, _: &str) -> bool {
            unreachable!()
        }
        fn load(&self, _: &str) -> Result<Box<dyn Recognizer>, String> {
            unreachable!()
        }
        fn default_model(&self) -> String {
            unreachable!()
        }
        fn analyzer(&self) -> Result<Box<dyn SpeakerAnalyzer>, DiarizeError> {
            unreachable!()
        }
        fn summary_endpoint(&self) -> Result<SummaryEndpoint, EndpointProblem> {
            unreachable!()
        }
        fn summary_prompt(&self) -> summarize::Prompt {
            unreachable!()
        }
        fn learn_limit(&self, _: &str, _: &str, _: u32) {
            unreachable!()
        }
        fn destination(&self) -> Option<PathBuf> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn a_stage_waiting_for_the_shared_permit_acquires_no_power() {
        use std::future::Future;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::task::{Context, Poll, Waker};

        let acquired = Arc::new(AtomicUsize::new(0));
        let starts = Arc::clone(&acquired);
        let power = power::ProcessingPower::with_factory(move || {
            starts.fetch_add(1, Ordering::SeqCst);
            Some(())
        })
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let runner = Runner::with_power(
            Environment {
                store: Arc::new(Mutex::new(session_core::Store::open_in_memory().unwrap())),
                working_root: root.path().into(),
                engines: Box::new(UnusedEngines),
                suspended: Box::new(|| false),
                events: Box::new(|_| panic!("a waiting stage must not start")),
            },
            power,
        );
        // Another job owns the permit, including while it is suspended.
        let _other_job = runner.permit.clone().acquire_owned().await.unwrap();
        let mut attempt = crate::recognition::durable::Attempt::default();
        let stage = runner.run_stage(
            "waiting",
            "model",
            Stage::Assemble,
            CancellationToken::new(),
            &mut attempt,
        );
        let mut stage = std::pin::pin!(stage);
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(stage.as_mut().poll(&mut context), Poll::Pending));
        assert_eq!(
            acquired.load(Ordering::SeqCst),
            0,
            "waiting for another job must not acquire processing power"
        );
    }

    #[tokio::test]
    async fn the_runner_works_only_while_a_job_holds_the_permit() {
        let root = tempfile::tempdir().unwrap();
        let runner = Runner::new(Environment {
            store: Arc::new(Mutex::new(session_core::Store::open_in_memory().unwrap())),
            working_root: root.path().into(),
            engines: Box::new(UnusedEngines),
            suspended: Box::new(|| false),
            events: Box::new(|_| {}),
        });
        assert!(!runner.is_working(), "a runner with no job is not working");
        let job = runner.permit.clone().acquire_owned().await.unwrap();
        assert!(
            runner.is_working(),
            "a runner with a stage or a regeneration is working"
        );
        drop(job);
        assert!(!runner.is_working());
    }

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
