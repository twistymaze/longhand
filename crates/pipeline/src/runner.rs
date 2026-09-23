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
use crate::transcribe::{self, TurnCuts};
use crate::{Control, SharedStore, StageError, assemble, checkpoint, join, lock};
use diarize::{DiarizeError, SpeakerAnalyzer};

/// The stages this build runs, in order. Later units add verification,
/// the summary and landing after the join.
pub const STAGES: &[Stage] = &[
    Stage::Assemble,
    Stage::Diarize,
    Stage::Transcribe,
    Stage::Join,
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
    /// A model is missing; the session keeps its place in the queue.
    Waiting {
        session_id: String,
        what: String,
    },
    /// Every stage of this build has run.
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
    pub fn reclaim(&self) -> Result<Vec<String>, StageError> {
        Ok(lock(&self.env.store).reclaim_at_launch(&self.owner)?)
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
        for &stage in STAGES {
            let result = self.run_stage(id, model, stage, cancel.clone()).await;
            match result {
                Ok(()) => {}
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
                Err(StageError::Cancelled) => {
                    log::info!("processing of {id} stopped at a checkpoint");
                    emit(RunnerEvent::Cancelled {
                        session_id: id.to_string(),
                    });
                    return Outcome::Cancelled;
                }
                Err(StageError::Failed(message)) => {
                    log::error!("{stage:?} failed for {id}: {message}");
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
        Stage::Assemble => assemble::run(&env.store, id, &dir, segments, control).map(|_| ()),
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
        other => Err(StageError::Failed(format!(
            "{other:?} is not in this build"
        ))),
    }
}
