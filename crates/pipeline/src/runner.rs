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
use session_core::{Event, SessionState, Stage};
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::diarize::Speakers;
use crate::save;
use crate::summarize::{self, SummaryEndpoint};
use crate::transcribe::{self, TurnCuts};
use crate::{Control, SharedStore, StageError, assemble, checkpoint, join, lock, verify};
use diarize::{DiarizeError, SpeakerAnalyzer};

/// The stages, in order.
pub const STAGES: &[Stage] = &[
    Stage::Assemble,
    Stage::Diarize,
    Stage::Transcribe,
    Stage::Join,
    Stage::Verify,
    Stage::Summarize,
    Stage::Land,
];

const POLL: Duration = Duration::from_secs(2);
const HEARTBEAT: Duration = Duration::from_secs(10);
const SUSPENDED_POLL: Duration = Duration::from_millis(250);

/// Where recognition engines come from.
pub trait EngineProvider: Send + Sync {
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
    /// Remember that `model` at `base_url` takes no more than `limit` tokens.
    fn learn_limit(&self, base_url: &str, model: &str, limit: u32);
    /// The destination folder chosen now, read when the session lands, so
    /// a destination changed during recording lands it in the last (AE7).
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
    /// Summarization failed; the session lands with a stub summary (KD13).
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
    /// The one loaded engine, kept across sessions (KTD6).
    engine: Arc<Mutex<Option<Box<dyn Recognizer>>>>,
    /// One stage of one session at a time, whoever asks (R20c).
    permit: Arc<Semaphore>,
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

struct StageControl {
    cancel: CancellationToken,
    env: Arc<Environment>,
    session_id: String,
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
                return Ok(());
            }
            std::thread::sleep(SUSPENDED_POLL);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled() || self.taken_back()
    }
}

impl Runner {
    pub fn new(env: Environment) -> Arc<Self> {
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
            verify::prepare_retry(&self.env.store, session_id)?;
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

    /// Summarize a landed session again, replacing its `summary.md` (R19,
    /// AE17). Takes its turn with the stages, one at a time (R20c).
    pub async fn regenerate_summary(&self, session_id: &str) -> Result<PathBuf, StageError> {
        let _permit = self
            .permit
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| StageError::Failed(e.to_string()))?;
        let env = Arc::clone(&self.env);
        let cancel = self.shutdown.child_token();
        let id = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let endpoint = endpoint(&env)?;
            let (base_url, model_id) = (endpoint.base_url.clone(), endpoint.model.clone());
            let control = Unattended {
                cancel,
                env: Arc::clone(&env),
            };
            save::regenerate_summary(
                &env.store,
                &id,
                endpoint,
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

    /// Stop at the next window and free the recognition engine. Called as
    /// the process exits: ggml's Metal backend asserts in its static
    /// destructor if a model is still loaded then, and the abort reads as a
    /// crash on every quit after a session was transcribed. Handy unloads on
    /// exit for the same reason. Waits for a window in progress to end.
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

    pub async fn run(self: Arc<Self>) {
        loop {
            // A session waiting on a model is back at the head of the queue;
            // claiming it again at once would spin until the model lands.
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

    /// Claim the head of the queue and run its stages. `None` when nothing
    /// can start: the queue is empty, capture is active, or the head
    /// session's models are not on disk yet (it keeps its place).
    pub async fn run_next(&self) -> Option<(String, Outcome)> {
        if self.shutdown.is_cancelled() || (self.env.suspended)() {
            return None;
        }
        let (id, model) = {
            let mut store = lock(&self.env.store);
            let head = store.queue().ok()?.into_iter().next()?;
            let record = store.session(&head).ok()??;
            let model = record
                .asr_model
                .unwrap_or_else(|| self.env.engines.default_model());
            if !self.env.engines.ready(&model) {
                return None;
            }
            let id = match store.dequeue(&self.owner, now()) {
                Ok(Some(id)) => id,
                Ok(None) => return None,
                Err(e) => {
                    log::error!("could not claim {head}: {e}");
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
        // Once landing has begun, the working directory may be gone: only
        // landing itself is left to finish.
        let saving = save::begun(&lock(&self.env.store), id).unwrap_or(false);
        for &stage in STAGES {
            if saving && stage != Stage::Land {
                continue;
            }
            let result = self.run_stage(id, model, stage, cancel.clone()).await;
            match result {
                Ok(()) => {}
                // A failed summary costs the summary, not the session: it
                // lands with a stub naming the failure (KD13, R22).
                Err(StageError::Failed(message)) if stage == Stage::Summarize => {
                    log::warn!("summarization failed for {id}; landing without it: {message}");
                    if let Err(e) = save::record_summary_failure(&self.env.store, id, &message) {
                        log::error!("could not record the summary failure of {id}: {e}");
                    }
                    emit(RunnerEvent::SummaryUnavailable {
                        session_id: id.to_string(),
                        message,
                    });
                }
                Err(StageError::NoDestination(problem)) => {
                    log::info!("{id} waits for its destination: {problem}");
                    if let Err(e) =
                        lock(&self.env.store).apply(id, &Event::LandingFailed(problem.clone()))
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
                Err(StageError::Failed(message)) if stage == Stage::Diarize => {
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

    async fn run_stage(
        &self,
        id: &str,
        model: &str,
        stage: Stage,
        cancel: CancellationToken,
    ) -> Result<(), StageError> {
        let segments = lock(&self.env.store).segments(id)?;
        if checkpoint::is_complete(&self.env.store, id, stage, segments.len())? {
            return Ok(());
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
        let result = tokio::task::spawn_blocking(move || {
            let control = StageControl {
                cancel,
                env: Arc::clone(&env),
                session_id: id_owned.clone(),
            };
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
    match stage {
        Stage::Assemble => {
            let home = lock(&env.store)
                .session(id)?
                .and_then(|record| record.landed_path)
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
            let mut slot = engine.lock().unwrap_or_else(|e| e.into_inner());
            // Checked under the engine's lock, so a shutdown that freed the
            // engine is never followed by loading it again.
            control.proceed()?;
            // A queued session keeps the model selected when it stopped (R20c).
            if slot.as_ref().map(|e| e.model_id()) != Some(model) {
                *slot = None;
                *slot = Some(env.engines.load(model).map_err(StageError::Failed)?);
            }
            let recognizer = slot.as_mut().expect("loaded above");
            transcribe::run(
                &env.store,
                id,
                &dir,
                segments,
                &cuts,
                recognizer.as_mut(),
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
        Stage::Land => {
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
                "{id} landed at {}{}",
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

/// For work on a landed session: stops on shutdown and waits out capture,
/// with no processing state to be taken back from.
struct Unattended {
    cancel: CancellationToken,
    env: Arc<Environment>,
}

impl Control for Unattended {
    fn proceed(&self) -> Result<(), StageError> {
        loop {
            if self.cancel.is_cancelled() {
                return Err(StageError::Cancelled);
            }
            if !(self.env.suspended)() {
                return Ok(());
            }
            std::thread::sleep(SUSPENDED_POLL);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}
