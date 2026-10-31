//! The stage machine (R20c, KTD20). One runner per process polls the FIFO,
//! claims the head session, and runs its stages in order on the blocking
//! pool. Each stage checkpoints as it goes and consults a [`Control`] at
//! every checkpoint boundary, which is where cancellation (a resume took
//! the session back, or the app is quitting) and suspension (capture is
//! active) take effect.
//!
//! Two queues share the work (KTD1). The local permit lets one audio stage
//! of one note run at a time, and the audio stages pause during a
//! recording. The summary queue runs one job at a time: the Summarize and
//! Save stages of a note, a regeneration or a title job. It does not pause
//! during a recording. Each note has its own task from dequeue to the end
//! of its Save, so the next note can start its audio stages while a note
//! waits for its summary (KTD3).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use asr::Recognizer;
use chrono::{DateTime, FixedOffset, Local};
use session_core::jobs::{Claims, RegenerateRefusal};
use session_core::{Event, SessionState, Stage};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, TryAcquireError, oneshot};
use tokio_util::sync::CancellationToken;

use crate::diarize::Speakers;
use crate::folder_rename::{self, FolderRename};
use crate::live::{EngineAccess, EngineSlot};
use crate::progress::{SpeakerMeter, SpeakerProgress, SpeakerWork};
use crate::reprocess::{self, Guards, Readiness, Refusal};
use crate::save;
use crate::summarize::{self, SummaryEndpoint};
use crate::summary_queue::{Class, SummaryQueue, Turn};
use crate::transcribe::{self, TurnCuts};
use crate::type_move::{self, TypeChange};
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

/// The stages that run in the summary queue (KTD1). The stages before
/// them are the audio stages, which use the local permit.
const SUMMARY_STAGES: [Stage; 2] = [Stage::Summarize, Stage::Save];

/// The stage that the last Retry of a failed note continued from.
const RETRIED_STAGE: &str = "retried-stage";

/// The message of a Regenerate Summary that a regeneration of the same
/// note refuses (R9).
const REGENERATING: &str = "This note's summary is already being made again.";

/// The stages whose repeated failure a start-over can clear: each stage
/// that derives the transcript from the recorded audio.
const START_OVER_STAGES: [Stage; 4] = [
    Stage::Transcribe,
    Stage::Diarize,
    Stage::Join,
    Stage::Verify,
];

/// True when a Retry continued from the checkpoints and the same stage
/// failed again. The next Retry then starts over from the recorded audio,
/// so that a checkpoint the run cannot use does not stop the note forever.
pub fn retry_starts_over(store: &SharedStore, note_id: &str) -> Result<bool, StageError> {
    let store = lock(store);
    let Some(SessionState::ProcessingFailed { stage, .. }) =
        store.session(note_id)?.map(|r| r.state)
    else {
        return Ok(false);
    };
    Ok(START_OVER_STAGES.contains(&stage)
        && store.result_state(note_id, RETRIED_STAGE)? == Some(serde_json::to_value(stage)?))
}

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
    /// A snapshot of the summary prompt of the note type `note_type`, or of
    /// "No type" for `None`, read fresh for each summarize stage like the
    /// endpoint (R3, KTD3). A type with no prompt of its own gives the
    /// default prompt. A run that saved a snapshot before uses its saved
    /// one while its inputs stay the same (KTD4).
    fn summary_prompt(&self, note_type: Option<&str>) -> summarize::Prompt;
    /// The Background text that the next summary uses, read fresh when a
    /// summary starts like the prompt (KTD6). It is the cached text when
    /// the Background file cannot be used (KTD3), and empty when the user
    /// has none.
    fn background(&self) -> String;
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
        note_id: String,
        stage: Stage,
    },
    /// What the note waits for changed (KTD10). `Some` when the note
    /// starts to wait: for the local permit, for its turn in the summary
    /// queue (a new note or a regeneration), for a model, or for a
    /// recording to end. `None` when the wait ends and the stage runs
    /// again. A `StageStarted` event also ends a wait. [`Runner::waits`]
    /// gives the current wait of each note.
    Waits {
        note_id: String,
        wait: Option<Wait>,
    },
    /// Per window of the transcribe stage, and per call of the summarize
    /// stage. `summary_step` is the step of a summary in parts, and `None`
    /// for the transcribe stage.
    Progress {
        note_id: String,
        stage: Stage,
        segment: u32,
        summary_step: Option<summarize::SummaryStep>,
        done: usize,
        total: usize,
    },
    /// The speaker stage's progress over the whole note (KTD1). A new
    /// attempt replaces the progress of an earlier one.
    SpeakerProgress {
        note_id: String,
        progress: crate::progress::SpeakerProgress,
    },
    StageFailed {
        note_id: String,
        stage: Stage,
        message: String,
    },
    Cancelled {
        note_id: String,
    },
    /// Diarization failed; the transcript goes ahead without speaker marks.
    SpeakersUnavailable {
        note_id: String,
        message: String,
    },
    /// The credential store is locked; the session waits for the user.
    Blocked {
        note_id: String,
        message: String,
    },
    /// A model is missing; the session keeps its place in the queue.
    Waiting {
        note_id: String,
        what: String,
    },
    /// Summarization failed; the session is saved with a stub summary (KD13).
    SummaryUnavailable {
        note_id: String,
        message: String,
    },
    /// The destination cannot take the session; it waits, whole, in the
    /// working location (R22b).
    AwaitingDestination {
        note_id: String,
        problem: String,
    },
    /// In the destination under its final name. `copied` when the volumes
    /// differed and the session was copied rather than moved in one step.
    Saved {
        note_id: String,
        path: PathBuf,
        copied: bool,
        /// The Summarize stage made the summary (see
        /// [`save::summary_made`]). The save clears the stage's rows, so
        /// this is the record of it.
        summary: bool,
    },
    /// A Transcribe Again run failed before its save. The session is saved
    /// with its previous transcript and summary (R19).
    TranscribeAgainFailed {
        note_id: String,
        stage: Stage,
        message: String,
    },
    /// Every stage has run.
    Finished {
        note_id: String,
    },
    /// The title job gave the saved note a proposed title (KTD13). Its
    /// folder has the new name, or takes it when the work on it ends.
    TitleProposed {
        note_id: String,
        title: String,
    },
}

/// What a note waits for before its next stage can run (KTD10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// A recording is active, and the next stage of the note is an audio
    /// stage. A queued note also waits: no note is claimed during a
    /// recording.
    Recording,
    /// Another note has the local permit or is in its audio stages.
    AnotherNote,
    /// The note waits for its turn in the summary queue: a new note after
    /// its audio stages, or a regeneration of a saved note.
    SummaryTurn,
    /// A model that the note needs is not on disk.
    Model,
}

/// The waits of the notes at one time (KTD10). Get it with
/// [`Runner::waits`] before the store lock, and give it the state and the
/// stage of each note from the store.
#[derive(Debug, Clone, Default)]
pub struct Waits {
    recording: bool,
    marks: HashMap<String, Wait>,
    no_model: HashSet<String>,
    audio: HashSet<String>,
}

impl Waits {
    /// What the note `id` waits for, if anything. `stage` is the stage in
    /// the job row of the note.
    pub fn of(&self, id: &str, state: &SessionState, stage: Option<Stage>) -> Option<Wait> {
        let mark = self.marks.get(id).copied();
        if mark == Some(Wait::SummaryTurn) {
            return mark;
        }
        match state {
            SessionState::Processing { .. } => {
                let audio_stage = stage.is_none_or(|s| !SUMMARY_STAGES.contains(&s));
                if self.recording && audio_stage {
                    Some(Wait::Recording)
                } else {
                    mark
                }
            }
            SessionState::Queued { .. } => {
                if self.recording {
                    Some(Wait::Recording)
                } else if self.no_model.contains(id) {
                    Some(Wait::Model)
                } else if self.audio.iter().any(|other| other != id) {
                    Some(Wait::AnotherNote)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// What a title job did (KTD13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleJob {
    /// The note has this proposed title.
    Proposed(String),
    /// The note keeps its title: the title turn failed, was cut off or
    /// gave an empty title, no endpoint is set, or the user set a title.
    /// The request is deleted (R14).
    Kept,
    /// The job sent no title. A delete, a Move or a regeneration of the
    /// note deleted the request, or the request stays for later: the note
    /// is not saved, the app quits, or the secret store is locked.
    Skipped,
}

/// Whether a title job can send its request now.
enum TitleDue {
    Yes,
    /// Leave the request as it is: the note is gone, is not saved, or has
    /// no request.
    No,
    /// Delete the request: a delete, a Move or a regeneration works on the
    /// note.
    Dropped,
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
    /// The cancel token of each note that has a task, from dequeue to the
    /// end of its Save (KTD4). Each token is a child of `shutdown`.
    current: Mutex<HashMap<String, CancellationToken>>,
    shutdown: CancellationToken,
    /// The one loaded engine, kept across sessions (KTD6) and shared with
    /// the live worker (KTD9).
    engine: EngineSlot,
    /// The local permit: one audio stage of one note at a time, whoever
    /// asks (R20c).
    permit: Arc<Semaphore>,
    /// The summary queue: one job at a time that sends requests to the
    /// summary endpoint and writes a note folder (KTD1, KTD2).
    summary: SummaryQueue,
    /// The notes that wait now in the runner (KTD10): for the local permit
    /// (`AnotherNote`), or for their turn in the summary queue
    /// (`SummaryTurn`, a new note after its audio stages or a
    /// regeneration). A note leaves the map when its wait ends.
    marks: Mutex<HashMap<String, Wait>>,
    /// The queued notes that the last claims skipped because a model is
    /// not on disk, and the notes whose stage gave `Waiting`.
    no_model: Mutex<HashSet<String>>,
    /// The claimed notes that have not left their audio stages. The queued
    /// notes wait for them (KTD3).
    audio: Mutex<HashSet<String>>,
    /// True while a task runs the queued title jobs.
    titles_running: AtomicBool,
    /// Summary regenerations and deletes that run on a session. They
    /// exclude each other, and a folder rename waits for both (R8, KTD8).
    claims: Claims,
    power: power::ProcessingPower,
    /// The saved notes whose title job waits for its turn (KTD13).
    titles: Mutex<VecDeque<String>>,
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

/// Lock a map of the runner. A panic in another holder does not stop the
/// runner.
fn held<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Tell the app the wait of `note_id` (KTD10). `None` ends the wait.
fn tell_wait(env: &Environment, note_id: &str, wait: Option<Wait>) {
    (env.events)(RunnerEvent::Waits {
        note_id: note_id.to_string(),
        wait,
    });
}

/// The control of a stage of a note. It stops on cancel, on quit, and when
/// the note is no longer `Processing`. An audio stage also waits while
/// capture is active (KTD20). A stage in the summary queue does not wait,
/// and does not read the capture state (KTD6).
struct StageControl {
    cancel: CancellationToken,
    env: Arc<Environment>,
    note_id: String,
    power: ActivePower,
    /// True for an audio stage: the stage waits while capture is active.
    pause: bool,
}

impl StageControl {
    /// A resume moves the session out of processing however it arrives;
    /// work on it stops at the next boundary even without a `cancel`.
    fn taken_back(&self) -> bool {
        !matches!(
            lock(&self.env.store).session(&self.note_id),
            Ok(Some(record)) if matches!(record.state, SessionState::Processing { .. })
        )
    }
}

impl Control for StageControl {
    fn proceed(&self) -> Result<(), StageError> {
        // The app shows the pause and its end (KTD10).
        let tell = |wait| tell_wait(&self.env, &self.note_id, wait);
        let mut paused = false;
        loop {
            if self.cancel.is_cancelled() || self.taken_back() {
                return Err(StageError::Cancelled);
            }
            // `taken_back` released the store lock, so `suspended` can
            // lock the capture state.
            if !self.pause || !(self.env.suspended)() {
                self.power.resume();
                if paused {
                    tell(None);
                }
                return Ok(());
            }
            if !paused {
                paused = true;
                tell(Some(Wait::Recording));
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
            current: Mutex::default(),
            shutdown: CancellationToken::new(),
            engine: Arc::new(Mutex::new(None)),
            permit: Arc::new(Semaphore::new(1)),
            summary: SummaryQueue::default(),
            marks: Mutex::default(),
            no_model: Mutex::default(),
            audio: Mutex::default(),
            titles_running: AtomicBool::new(false),
            claims: Claims::default(),
            power,
            titles: Mutex::default(),
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

    /// Stop work on `note_id` at its next checkpoint (a resume took it
    /// back), also while it waits for the summary queue. Completed
    /// checkpoints stand.
    pub fn cancel(&self, note_id: &str) {
        if let Some(token) = held(&self.current).get(note_id) {
            token.cancel();
        }
    }

    /// The user's Retry on a failed session: put it back in line, keeping
    /// its place and its checkpoints. A failed verification first forgets
    /// the windows that missed speech, so only those are transcribed again.
    /// With `start_over`, the recognition work is removed first and the run
    /// starts again from the recorded audio (see [`retry_starts_over`]).
    pub fn retry(&self, note_id: &str, start_over: bool) -> Result<(), StageError> {
        let failed = lock(&self.env.store).session(note_id)?.map(|r| r.state);
        if let Some(SessionState::ProcessingFailed { stage, .. }) = failed
            && START_OVER_STAGES.contains(&stage)
        {
            let mut store = lock(&self.env.store);
            if start_over {
                log::info!(
                    "{note_id} starts over from the recorded audio after {stage:?} failed again"
                );
                store.start_over(note_id)?;
                store.clear_result_state(note_id, RETRIED_STAGE)?;
            } else {
                store.set_result_state(note_id, RETRIED_STAGE, &serde_json::to_value(stage)?)?;
            }
        }
        if !start_over
            && let Some(SessionState::ProcessingFailed { stage, .. }) = failed
            && stage == Stage::Verify
        {
            if crate::recognition::snapshot(&self.env.store, note_id)?.is_some() {
                crate::recognition::prepare_retry(&self.env.store, note_id)?;
            } else {
                verify::prepare_retry(&self.env.store, note_id)?;
            }
        }
        {
            let mut store = lock(&self.env.store);
            store.apply(note_id, &Event::Retry)?;
            store.apply(note_id, &Event::ClaimLost)?;
        }
        self.wake();
        Ok(())
    }

    /// The secret store is available again: a blocked session goes back to
    /// its place in line and resumes at its checkpoints (R20b).
    pub fn unblock(&self, note_id: &str) -> Result<(), StageError> {
        let blocked = matches!(
            lock(&self.env.store).session(note_id)?.map(|r| r.state),
            Some(SessionState::Blocked { .. })
        );
        if blocked {
            lock(&self.env.store).apply(note_id, &Event::ClaimLost)?;
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
            self.retry(id, false)?;
        }
        Ok(ids)
    }

    /// Summarize a saved session again, replacing its `summary.md` (R19,
    /// AE17). Takes its turn in the summary queue (KTD1). The title job of
    /// the new summary is queued after the regeneration releases its claim
    /// and its turn (R16).
    pub async fn regenerate_summary(&self, note_id: &str) -> Result<PathBuf, StageError> {
        let started = Instant::now();
        let mut waited = Duration::ZERO;
        let regenerated = self.regenerate_claimed(note_id, &mut waited).await;
        // One log line for each regeneration (R14).
        let took = started.elapsed().saturating_sub(waited);
        let (level, line) = regeneration_end(note_id, waited, took, &regenerated);
        log::log!(level, "{line}");
        if regenerated.is_ok() {
            self.queue_title_if_pending(note_id);
        }
        regenerated
    }

    /// The regeneration. `waited` gets the time that the regeneration
    /// waited for its turn in the summary queue.
    async fn regenerate_claimed(
        &self,
        note_id: &str,
        waited: &mut Duration,
    ) -> Result<PathBuf, StageError> {
        // While the Move holds the note for its final check, the
        // regeneration waits (KTD9). A second regeneration of the note is
        // refused at once (R9, KTD8).
        let _claim = loop {
            match self.claims.regenerate(note_id) {
                Ok(claim) => break claim,
                Err(RegenerateRefusal::Regenerating) => {
                    return Err(StageError::Failed(REGENERATING.into()));
                }
                Err(RegenerateRefusal::Deleting) => {
                    return Err(StageError::Failed(
                        "This note is being deleted, so its summary cannot be made again.".into(),
                    ));
                }
                Err(RegenerateRefusal::Held) => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        };
        // A note that is not saved is refused before the wait for a turn
        // (R10). `save::regenerate_summary` checks again after the wait.
        save::require_saved(&lock(&self.env.store), note_id)?;
        // The regeneration writes the note folder, so it takes a turn in
        // the summary queue (KTD1). A quit stops the wait.
        let cancel = self.shutdown.child_token();
        let _turn = {
            let _waits = WaitMark::new(self, note_id, Wait::SummaryTurn);
            let started = Instant::now();
            let turn = tokio::select! {
                turn = self.summary.acquire(Class::Regeneration) => Ok(turn),
                _ = cancel.cancelled() => Err(StageError::Cancelled),
            };
            *waited = started.elapsed();
            turn?
        };
        let queued = *waited;
        let env = Arc::clone(&self.env);
        let id = note_id.to_string();
        let power = self.power.clone();
        tokio::task::spawn_blocking(move || {
            let control = Unattended {
                cancel,
                power: ActivePower::new(power),
            };
            control.proceed()?;
            // The regeneration has its turn (KTD10).
            (env.events)(RunnerEvent::StageStarted {
                note_id: id.clone(),
                stage: Stage::Summarize,
            });
            let endpoint = endpoint(&env)?;
            // Regenerate uses the prompt of the note type after the row
            // follows its folder (R3, KTD3), and the current Background
            // (R10, KTD6).
            let resolve = |note_type: Option<&str>| summarize::Prompt {
                background: env.engines.background(),
                ..env.engines.summary_prompt(note_type)
            };
            save::regenerate_summary(
                &env.store,
                &id,
                &env.working_root,
                endpoint,
                &resolve,
                &control,
                queued,
                &mut |step, done, total| {
                    (env.events)(RunnerEvent::Progress {
                        note_id: id.clone(),
                        stage: Stage::Summarize,
                        segment: 0,
                        summary_step: Some(step),
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
        note_id: &str,
        settings: &models::recognition::RecognitionConfig,
        ready: &dyn Fn(&models::recognition::ExecutionSpec) -> Readiness,
    ) -> Result<models::recognition::ExecutionSpec, Refusal> {
        let guards = self.guards(note_id);
        reprocess::check(&lock(&self.env.store), note_id, guards, settings, ready)
    }

    /// The user's Transcribe Again on a saved note (R15, KTD14): queue it
    /// for a run with the execution that [`Runner::check_transcribe_again`]
    /// gives. Every check runs again after the row follows its folder.
    /// Refused when the settings no longer select `model_id`, the model that
    /// the user confirmed.
    pub fn transcribe_again(
        &self,
        note_id: &str,
        settings: &models::recognition::RecognitionConfig,
        model_id: &str,
        ready: &dyn Fn(&models::recognition::ExecutionSpec) -> Readiness,
    ) -> Result<(), Refusal> {
        let guards = self.guards(note_id);
        let spec = reprocess::queue(
            &mut lock(&self.env.store),
            note_id,
            guards,
            settings,
            model_id,
            ready,
        )?;
        log::info!(
            "{note_id} is queued to be transcribed again with '{}'",
            spec.config.asr_model
        );
        self.wake();
        Ok(())
    }

    /// The state outside the store that refuses a Transcribe Again run.
    /// Call this before the store lock. `suspended` locks the capture state,
    /// and the capture manager locks its state and then the store.
    fn guards(&self, note_id: &str) -> Guards {
        Guards {
            regenerating: self.claims.is_regenerating_or_deleting(note_id),
            capture_active: (self.env.suspended)(),
        }
    }

    /// True while a job runs or waits in either queue, or a note has its
    /// task (KTD11). A session that waits in the durable queue does not
    /// count. This reads only the permit, the summary queue and the map of
    /// note tasks, not the store, so a caller can hold any lock.
    pub fn is_working(&self) -> bool {
        self.permit.available_permits() == 0
            || self.summary.is_active()
            || !held(&self.current).is_empty()
    }

    /// True while the runner works on `note_id` now: a stage of the note
    /// runs, or waits for its turn in the summary queue. A note that waits
    /// in the durable queue does not count.
    pub fn runs(&self, note_id: &str) -> bool {
        held(&self.current).contains_key(note_id)
    }

    /// True while `note_id` waits for its turn in the summary queue:
    /// after its audio stages, or for a regeneration of its summary
    /// (KTD10). A title job that waits does not count.
    pub fn waits_to_summarize(&self, note_id: &str) -> bool {
        held(&self.marks).get(note_id) == Some(&Wait::SummaryTurn)
    }

    /// The waits of the notes now (KTD10). This reads the capture state,
    /// so call it before the store lock: the capture manager locks its
    /// state and then the store.
    pub fn waits(&self) -> Waits {
        let recording = (self.env.suspended)();
        Waits {
            recording,
            marks: held(&self.marks).clone(),
            no_model: held(&self.no_model).clone(),
            audio: held(&self.audio).clone(),
        }
    }

    /// Mark that `note_id` waits for `wait`, and tell the app.
    fn mark(&self, note_id: &str, wait: Wait) {
        held(&self.marks).insert(note_id.to_string(), wait);
        tell_wait(&self.env, note_id, Some(wait));
    }

    /// Remove the wait mark of `note_id`. Returns whether it had one.
    fn unmark(&self, note_id: &str) -> bool {
        held(&self.marks).remove(note_id).is_some()
    }

    /// True while `note_id` has its summary regenerated or waits to.
    pub fn is_regenerating(&self, note_id: &str) -> bool {
        self.claims.is_regenerating(note_id)
    }

    /// The claims that a delete takes (R8): see
    /// [`session_core::jobs::delete_session`].
    pub fn claims(&self) -> &Claims {
        &self.claims
    }

    /// Queue the title job of `note_id` when it has a pending title
    /// request (KTD13), and wake the runner.
    fn queue_title_if_pending(&self, note_id: &str) {
        match has_title_request(&self.env.store, note_id) {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                log::warn!("could not read the title request of {note_id}: {e}");
                return;
            }
        }
        self.queue_title(note_id);
    }

    fn queue_title(&self, note_id: &str) {
        {
            let mut titles = self.titles.lock().unwrap_or_else(|e| e.into_inner());
            if !titles.iter().any(|id| id == note_id) {
                titles.push_back(note_id.to_string());
            }
        }
        self.wake();
    }

    /// At launch: queue a title job for each saved session that still has
    /// a pending title request (R15, KTD13). Returns their ids.
    pub fn queue_pending_titles(&self) -> Result<Vec<String>, StageError> {
        let saved = lock(&self.env.store).sessions_in(SessionState::Saved.name())?;
        let mut queued = Vec::new();
        for record in saved {
            match has_title_request(&self.env.store, &record.id) {
                Ok(true) => {
                    self.queue_title(&record.id);
                    queued.push(record.id);
                }
                Ok(false) => {}
                // One unreadable request does not stop the other notes.
                Err(e) => log::warn!("could not read the title request of {}: {e}", record.id),
            }
        }
        Ok(queued)
    }

    /// Run the queued title jobs, one at a time, each with a turn in the
    /// summary queue (KTD13). A recording does not stop them (R5, KTD6).
    /// Nothing starts after [`Runner::shutdown`]. Returns what each job
    /// did.
    pub async fn run_titles(&self) -> Vec<(String, TitleJob)> {
        let mut done = Vec::new();
        while !self.shutdown.is_cancelled() {
            let next = held(&self.titles).pop_front();
            let Some(id) = next else {
                break;
            };
            let job = self.title_job(&id).await;
            done.push((id, job));
        }
        done
    }

    /// Start a task that runs the queued title jobs, when no such task
    /// runs. A title job waits for its turn in the summary queue, so the
    /// processing loop does not wait for it.
    fn spawn_titles(self: &Arc<Self>) {
        if held(&self.titles).is_empty() || self.titles_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let runner = Arc::clone(self);
        tokio::spawn(async move {
            runner.run_titles().await;
            runner.titles_running.store(false, Ordering::SeqCst);
        });
    }

    /// The title job of a saved note (KTD13): send its pending title
    /// request, propose the cleaned reply and give the folder its name.
    /// The job holds no claim while it waits or while the title turn runs,
    /// so a delete or a Move of the note is not refused. A delete, a Move
    /// or a regeneration stops the job and deletes the request instead.
    async fn title_job(&self, note_id: &str) -> TitleJob {
        if let Some(job) = title_gate(&lock(&self.env.store), &self.claims, note_id) {
            return job;
        }
        // The title job can rename the note folder, so it takes a turn in
        // the summary queue (KTD1). A quit stops the wait, and the request
        // stays for the next launch.
        let cancel = self.shutdown.child_token();
        let started = Instant::now();
        let _turn = tokio::select! {
            turn = self.summary.acquire(Class::Title) => turn,
            _ = cancel.cancelled() => return TitleJob::Skipped,
        };
        let queued = started.elapsed();
        let env = Arc::clone(&self.env);
        let claims = self.claims.clone();
        let id = note_id.to_string();
        let control = TitleControl {
            cancel,
            env: Arc::clone(&self.env),
            claims: self.claims.clone(),
            note_id: id.clone(),
            power: ActivePower::new(self.power.clone()),
        };
        let job =
            tokio::task::spawn_blocking(move || send_title(&env, &claims, &id, &control, queued))
                .await
                .unwrap_or_else(|e| {
                    log::error!("the title job of {note_id} stopped unexpectedly: {e}");
                    TitleJob::Skipped
                });
        if let TitleJob::Proposed(title) = &job {
            (self.env.events)(RunnerEvent::TitleProposed {
                note_id: note_id.to_string(),
                title: title.clone(),
            });
        }
        job
    }

    /// The user's rename (R9): the title changes at once, and a saved, idle
    /// session's folder takes the new name now. A busy session's folder
    /// takes it when the work ends.
    pub fn rename_session(&self, note_id: &str, title: &str) -> Result<FolderRename, StageError> {
        // The rename waits while the Move holds the note (KTD9).
        let mut store = crate::move_notes::lock_unheld(&self.env.store, &self.claims, note_id);
        folder_rename::rename_locked(
            &mut store,
            note_id,
            title,
            &|id| self.claims.busy(id),
            &session_save::RealFs,
        )
    }

    /// The user changes the type of a note (R5): the app shows the new type
    /// at once. The folder of a note that no computer works on moves now,
    /// and any other folder moves later (KTD5, KTD6). `own_runs` are this
    /// computer's runs. The caller checks that `to` is a type of the
    /// effective list.
    pub fn change_note_type(
        &self,
        note_id: &str,
        to: Option<&str>,
        own_runs: &[String],
    ) -> Result<TypeChange, StageError> {
        // The change waits while the Move holds the note (KTD9).
        let mut store = crate::move_notes::lock_unheld(&self.env.store, &self.claims, note_id);
        type_move::change_type_locked(
            &mut store,
            note_id,
            to,
            false,
            &self.type_context(own_runs, &session_save::RealFs),
        )
    }

    /// Run the type moves that this computer owes, and take over the moves
    /// that waited for 7 days (KTD5, KTD13). Returns the ids of the notes
    /// that changed.
    pub fn run_type_moves(&self, own_runs: &[String]) -> Result<Vec<String>, StageError> {
        type_move::run_owed(
            &self.env.store,
            &self.type_context(own_runs, &session_save::RealFs),
        )
    }

    /// One pass of the per-note part of a type rename, merge or delete in
    /// the storage folder `root` (KTD7).
    pub fn apply_type_edit(
        &self,
        root: &std::path::Path,
        edit_id: i64,
        edit: &session_core::BulkEdit,
        notes: &[String],
        own_runs: &[String],
    ) -> Result<type_move::EditPass, StageError> {
        type_move::apply_recorded(
            &self.env.store,
            root,
            edit_id,
            edit,
            notes,
            &self.type_context(own_runs, &session_save::RealFs),
        )
    }

    /// The user adds and removes tags of a note (R10): see
    /// [`crate::tags::set_note_tags`]. Returns the tags of the note.
    pub fn set_note_tags(
        &self,
        note_id: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<Vec<String>, StageError> {
        crate::tags::set_note_tags(&self.env.store, &self.claims, note_id, add, remove)
    }

    /// One pass of a tag rename or delete over the recorded notes `notes`
    /// (KTD11).
    pub fn apply_tag_edit(
        &self,
        edit: &session_core::BulkEdit,
        notes: &[String],
    ) -> Result<crate::tags::TagPass, StageError> {
        crate::tags::apply_recorded(&self.env.store, &self.claims, edit, notes)
    }

    fn type_context<'a>(
        &'a self,
        own_runs: &'a [String],
        fs: &'a dyn session_save::Fs,
    ) -> type_move::Context<'a> {
        type_move::Context {
            own_runs,
            claims: &self.claims,
            now: now(),
            fs,
        }
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
        // The token of each note is a child of this token, so the work on
        // each note stops, also a note that waits for the summary queue.
        self.shutdown.cancel();
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

    /// The processing loop. Each dequeued note runs in its own task. The
    /// loop waits only until the note leaves its audio stages, so only one
    /// note is in the audio stages at a time, in stop order (KTD3). The
    /// title jobs start when they are queued, also while a note is in its
    /// audio stages (KTD13).
    pub async fn run(self: Arc<Self>) {
        if let Err(e) = self.queue_pending_titles() {
            log::warn!("could not queue the pending title jobs: {e}");
        }
        loop {
            while let Some((id, model, cancel)) = self.claim_next() {
                let (audio_done, mut audio) = oneshot::channel();
                let runner = Arc::clone(&self);
                tokio::spawn(async move {
                    runner
                        .run_claimed(&id, &model, cancel, Some(audio_done))
                        .await;
                });
                // A title job queued while the note is in its audio stages
                // starts at once, so it does not wait for those stages
                // (KTD13). The loop keeps each wake that it takes here.
                let mut woken = false;
                let ended = loop {
                    tokio::select! {
                        ended = &mut audio => break ended,
                        _ = self.wake.notified() => {
                            woken = true;
                            self.spawn_titles();
                        }
                    }
                };
                if woken {
                    self.wake();
                }
                // A note that waits for a model is back at the head of
                // the queue. To claim it again at once would repeat until
                // the model download finishes.
                let waits = ended.is_ok_and(|ended| ended == Some(Outcome::Waiting));
                self.spawn_titles();
                if waits {
                    break;
                }
            }
            self.spawn_titles();
            tokio::select! {
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(POLL) => {}
                _ = self.shutdown.cancelled() => return,
            }
        }
    }

    /// Claim the first queued session whose models are on disk and run all
    /// its stages, the summary queue included. A session that waits for
    /// its model keeps its place, and the sessions after it go first
    /// (KTD13). `None` when nothing can start: the queue is empty, capture
    /// is active, or no queued session has its models.
    pub async fn run_next(&self) -> Option<(String, Outcome)> {
        let (id, model, cancel) = self.claim_next()?;
        let outcome = self.run_claimed(&id, &model, cancel, None).await;
        Some((id, outcome))
    }

    /// Claim the first queued session whose models are on disk, and give
    /// it a cancel token in `current`. Returns its id, its model and its
    /// token. `None` when nothing can start.
    fn claim_next(&self) -> Option<(String, String, CancellationToken)> {
        if self.shutdown.is_cancelled() || (self.env.suspended)() {
            return None;
        }
        // Each queued note that this pass reads, and whether its models are
        // on disk.
        let mut scanned: Vec<(String, bool)> = Vec::new();
        let claimed = self.claim_ready(&mut scanned);
        // The model marks change after the store lock, so that the app
        // can read the store when it gets the event.
        let mut missing = Vec::new();
        {
            let mut no_model = held(&self.no_model);
            for (id, ready) in scanned {
                if ready {
                    no_model.remove(&id);
                } else if no_model.insert(id.clone()) {
                    missing.push(id);
                }
            }
        }
        for id in missing {
            tell_wait(&self.env, &id, Some(Wait::Model));
        }
        let (id, model) = claimed?;
        // A quit stops the work on every note (KTD4).
        let cancel = self.shutdown.child_token();
        held(&self.current).insert(id.clone(), cancel.clone());
        Some((id, model, cancel))
    }

    /// Dequeue the first queued session whose models are on disk. Each
    /// session that it reads goes in `scanned` with its readiness.
    fn claim_ready(&self, scanned: &mut Vec<(String, bool)>) -> Option<(String, String)> {
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
            scanned.push((id.clone(), ready));
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
        // The queued notes wait for this note until it leaves its
        // audio stages.
        held(&self.audio).insert(id.clone());
        Some((id, model))
    }

    /// The task of a claimed note, from dequeue to the end of its Save
    /// (KTD3, KTD5). `audio_done` gets `None` when the note leaves its
    /// audio stages for the summary queue, or the outcome when the work on
    /// the note ends before that.
    async fn run_claimed(
        &self,
        id: &str,
        model: &str,
        cancel: CancellationToken,
        audio_done: Option<oneshot::Sender<Option<Outcome>>>,
    ) -> Outcome {
        let mut audio_done = audio_done;
        let outcome = {
            let _task = NoteTask::start(self, id);
            self.run_session(id, model, cancel, &mut audio_done).await
        };
        // The task has ended before the loop can claim the note again.
        if let Some(done) = audio_done.take() {
            let _ = done.send(Some(outcome.clone()));
        }
        outcome
    }

    /// Wait for a turn of the class "new note" in the summary queue
    /// (KTD10). While the note waits, `jobs.stage` is `stage`, so a restart
    /// shows the right stage. A cancel or a quit stops the wait, and the
    /// note leaves the queue. Returns the turn and the time waited.
    async fn summary_turn(
        &self,
        id: &str,
        stage: Stage,
        cancel: &CancellationToken,
    ) -> Result<(Turn, Duration), StageError> {
        // The mark comes before the stage, so that no read sees the
        // summarize stage with no wait.
        let mark = WaitMark::new(self, id, Wait::SummaryTurn);
        lock(&self.env.store).set_stage(id, stage)?;
        log::info!("{id} waits for its turn in the summary queue");
        let started = Instant::now();
        let turn = tokio::select! {
            turn = self.summary.acquire(Class::NewNote) => Ok(turn),
            _ = cancel.cancelled() => Err(StageError::Cancelled),
        };
        drop(mark);
        let waited = started.elapsed();
        if turn.is_ok() {
            log::info!(
                "{id} has its turn in the summary queue after {} ms",
                waited.as_millis()
            );
        }
        Ok((turn?, waited))
    }

    async fn run_session(
        &self,
        id: &str,
        model: &str,
        cancel: CancellationToken,
        audio_done: &mut Option<oneshot::Sender<Option<Outcome>>>,
    ) -> Outcome {
        let emit = |event| (self.env.events)(event);
        // Once saving has begun, the working directory may be gone: only
        // the save itself is left to finish.
        let saving = save::begun(&lock(&self.env.store), id).unwrap_or(false);
        // A failed store write can leave closed chunks with no segment row.
        // They are registered first, so that assemble reads every segment
        // (KTD3 of the capture recovery plan). When this fails, the save
        // stage keeps the working folder.
        if !saving {
            let dir = self.env.working_root.join(id);
            if let Err(e) = crate::recover::register_chunks(&self.env.store, id, &dir, None) {
                log::error!("could not register the audio of {id}: {e}");
            }
        }
        let again = lock(&self.env.store)
            .transcribe_again_marks(id)
            .is_ok_and(|marks| marks.model.is_some());
        let snapshot =
            crate::recognition::snapshot(&self.env.store, id).and_then(|spec| match spec {
                Some(spec) => crate::recognition::TaskGraph::resolve(&spec)
                    .map(|graph| Some(graph.stages))
                    .map_err(|e| StageError::Failed(e.to_string())),
                None => Ok(None),
            });
        let exact = matches!(&snapshot, Ok(Some(_)));
        let stages = match snapshot {
            Ok(Some(stages)) => stages,
            Ok(None) => STAGES.to_vec(),
            Err(error) => {
                return self.fail_run(id, Stage::Transcribe, error.to_string(), again);
            }
        };
        let mut attempt = crate::recognition::durable::Attempt::default();
        // The turn in the summary queue, and the time the note waited for
        // it. The note keeps the turn for Summarize and Save (KTD1).
        let mut turn: Option<(Turn, Duration)> = None;
        for stage in stages {
            if saving && stage != Stage::Save {
                continue;
            }
            let mut result = Ok(());
            if turn.is_none() && SUMMARY_STAGES.contains(&stage) {
                // The audio stages have ended: the next note can start its
                // audio stages (KTD3). A note whose Save has begun comes
                // here first and takes no local permit. The queued notes
                // no longer wait for this note.
                held(&self.audio).remove(id);
                if let Some(done) = audio_done.take() {
                    let _ = done.send(None);
                }
                match self.summary_turn(id, stage, &cancel).await {
                    Ok(got) => turn = Some(got),
                    Err(e) => result = Err(e),
                }
            }
            let queued = turn.as_ref().map_or(Duration::ZERO, |(_, waited)| *waited);
            let result = match result {
                Ok(()) => {
                    self.run_stage(id, model, stage, cancel.clone(), &mut attempt, queued)
                        .await
                }
                Err(e) => Err(e),
            };
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
                        note_id: id.to_string(),
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
                        && !(stage == Stage::Save && self.publication_begun(id)) =>
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
                        note_id: id.to_string(),
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
                        note_id: id.to_string(),
                        message,
                    });
                }
                Err(StageError::Waiting(what)) => {
                    log::info!("{id} waits for {what}");
                    // Back to its place in line; it runs once the models are in.
                    held(&self.no_model).insert(id.to_string());
                    if let Err(e) = lock(&self.env.store).apply(id, &Event::ClaimLost) {
                        log::warn!("could not return {id} to the queue: {e}");
                    }
                    emit(RunnerEvent::Waiting {
                        note_id: id.to_string(),
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
                        note_id: id.to_string(),
                        message,
                    });
                    return Outcome::Blocked;
                }
                Err(StageError::Cancelled) => {
                    log::info!("processing of {id} stopped at a checkpoint");
                    emit(RunnerEvent::Cancelled {
                        note_id: id.to_string(),
                    });
                    return Outcome::Cancelled;
                }
                // The Transcribe Again arm above took the other runs.
                Err(StageError::Failed(message)) => {
                    return self.fail_run(id, stage, message, false);
                }
            }
        }
        log::info!("processing of {id} finished");
        if let Err(e) = lock(&self.env.store).clear_result_state(id, RETRIED_STAGE) {
            log::warn!("could not clear the retried stage of {id}: {e}");
        }
        emit(RunnerEvent::Finished {
            note_id: id.to_string(),
        });
        // The note is saved with its summary; its title turn comes after
        // (R16).
        self.queue_title_if_pending(id);
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

    /// Whether the save of `id` has begun to replace the documents in its
    /// folder: then the run must finish, and a failure keeps its record for
    /// a retry.
    fn publication_begun(&self, id: &str) -> bool {
        save::home_changed(&lock(&self.env.store), id).unwrap_or(true)
            || self
                .env
                .working_root
                .join(id)
                .join(session_save::publication::RECORD)
                .exists()
    }

    /// End the run of `id` that failed at `stage`. A Transcribe Again run
    /// (`again`) whose publication has not begun returns the note to
    /// Saved with the error (R19). Any other run records the error, and
    /// the note fails processing.
    fn fail_run(&self, id: &str, stage: Stage, message: String, again: bool) -> Outcome {
        if again && !self.publication_begun(id) {
            return self.fail_transcribe_again(id, stage, message);
        }
        log::error!("{stage:?} failed for {id}: {message}");
        if let Err(e) = crate::progress::record_error(&self.env.store, id, stage, &message) {
            log::warn!("could not record the failure of {id}: {e}");
        }
        // A resume may have taken the session in the meantime; then there
        // is nothing to mark.
        if let Err(e) = lock(&self.env.store).apply(id, &Event::StageFailed(stage)) {
            log::warn!("could not record the failure of {id}: {e}");
        }
        self.drop_removed_old_audio(id);
        (self.env.events)(RunnerEvent::StageFailed {
            note_id: id.to_string(),
            stage,
            message: message.clone(),
        });
        Outcome::Failed { stage, message }
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
            note_id: id.to_string(),
            stage,
            message: message.clone(),
        });
        Outcome::Failed { stage, message }
    }

    /// The local permit for an audio stage of `id`. While another note has
    /// the permit, `id` is marked as waiting for another note (KTD10).
    async fn local_permit(&self, id: &str) -> Result<OwnedSemaphorePermit, StageError> {
        match self.permit.clone().try_acquire_owned() {
            Ok(permit) => return Ok(permit),
            Err(TryAcquireError::Closed) => {
                return Err(StageError::Failed("the local permit is closed".into()));
            }
            Err(TryAcquireError::NoPermits) => {}
        }
        let mark = WaitMark::new(self, id, Wait::AnotherNote);
        let permit = self
            .permit
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| StageError::Failed(e.to_string()))?;
        drop(mark);
        tell_wait(&self.env, id, None);
        Ok(permit)
    }

    async fn run_stage(
        &self,
        id: &str,
        model: &str,
        stage: Stage,
        cancel: CancellationToken,
        attempt: &mut crate::recognition::durable::Attempt,
        queued: Duration,
    ) -> Result<(), StageError> {
        // A new attempt does not show the progress of an earlier attempt.
        if stage == Stage::Diarize {
            lock(&self.env.store).delete_checkpoint(id, stage, crate::progress::SPEAKER_KEY)?;
        }
        // An audio stage waits for the local permit. A stage in the summary
        // queue already has its turn (KTD1).
        let audio = !SUMMARY_STAGES.contains(&stage);
        let _permit = if audio {
            Some(self.local_permit(id).await?)
        } else {
            None
        };
        let control = StageControl {
            cancel,
            env: Arc::clone(&self.env),
            note_id: id.to_string(),
            power: ActivePower::new(self.power.clone()),
            pause: audio,
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
            note_id: id.to_string(),
            stage,
        });

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
                queued,
            )
        })
        .await;
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
    queued: Duration,
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
                    note_id: id.into(),
                    stage,
                    segment: p.segment,
                    summary_step: None,
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
                        note_id: id.to_string(),
                        stage,
                        segment: p.segment,
                        summary_step: None,
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
            // The type that the note shows when the step starts chooses
            // the prompt (R3, KTD3). A pending type counts.
            let record = summarize::note_record(&env.store, id)?;
            let note_type = type_move::shown_type(&record).0;
            let mut prompt = env.engines.summary_prompt(note_type.as_deref());
            // The summary uses the Background of the time that it starts
            // (KTD6). A resumed summary uses the Background of its saved
            // PROMPT checkpoint, which `summarize::produce` reads first.
            prompt.background = env.engines.background();
            summarize::run(
                &env.store,
                id,
                &dir,
                segments,
                endpoint,
                prompt,
                control,
                queued,
                &mut |step, done, total| {
                    (env.events)(RunnerEvent::Progress {
                        note_id: id.to_string(),
                        stage,
                        segment: 0,
                        summary_step: Some(step),
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
                note_id: id.to_string(),
                path: saved.path,
                copied: saved.copied,
                summary: saved.summary,
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
            note_id: id.to_string(),
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

fn has_title_request(store: &SharedStore, note_id: &str) -> Result<bool, StageError> {
    Ok(checkpoint::get::<serde_json::Value>(
        store,
        note_id,
        Stage::Summarize,
        summarize::TITLE_REQUEST,
    )?
    .is_some())
}

/// Whether the title job of `note_id` can send its request now. The
/// caller holds the store lock.
fn title_due(store: &session_core::Store, claims: &Claims, note_id: &str) -> TitleDue {
    if claims.busy(note_id) {
        return TitleDue::Dropped;
    }
    let record = match store.session(note_id) {
        Ok(Some(record)) => record,
        Ok(None) => return TitleDue::No,
        Err(e) => {
            log::warn!("could not read {note_id} for its title job: {e}");
            return TitleDue::No;
        }
    };
    if record.state != SessionState::Saved || record.home_path.is_none() {
        return TitleDue::No;
    }
    // The Move puts the note in the new storage folder (KTD9).
    if !matches!(
        store.move_step(note_id),
        Ok(None | Some(session_core::MoveStep::Done))
    ) {
        return TitleDue::Dropped;
    }
    match store.checkpoint_value(note_id, Stage::Summarize, summarize::TITLE_REQUEST) {
        Ok(Some(_)) => {}
        Ok(None) => return TitleDue::No,
        Err(e) => {
            log::warn!("could not read the title request of {note_id}: {e}");
            return TitleDue::No;
        }
    }
    // A stop between the title checkpoint and the delete of the request
    // leaves a request whose title is already proposed.
    match store.checkpoint_value(note_id, Stage::Summarize, summarize::TITLE) {
        Ok(None) => TitleDue::Yes,
        Ok(Some(_)) => TitleDue::Dropped,
        Err(e) => {
            log::warn!("could not read the title proposal of {note_id}: {e}");
            TitleDue::No
        }
    }
}

/// `None` when the title job of `note_id` can send its request now.
/// Otherwise the result of the job, after the request is deleted when the
/// note dropped it. The caller holds the store lock.
fn title_gate(store: &session_core::Store, claims: &Claims, note_id: &str) -> Option<TitleJob> {
    match title_due(store, claims, note_id) {
        TitleDue::Yes => None,
        TitleDue::No => Some(TitleJob::Skipped),
        TitleDue::Dropped => {
            drop_title_request(store, claims, note_id);
            Some(TitleJob::Skipped)
        }
    }
}

/// Delete the title request of `note_id`, then its summary cache (KTD12
/// of the transcript removal plan). A note that a delete, a Move or a
/// regeneration holds keeps the cache: a regeneration uses it. An error
/// is logged: the note keeps its title in every case. The caller holds
/// the store lock.
fn drop_title_request(store: &session_core::Store, claims: &Claims, note_id: &str) {
    if let Err(e) = store.delete_checkpoint(note_id, Stage::Summarize, summarize::TITLE_REQUEST) {
        log::warn!("could not delete the title request of {note_id}: {e}");
        return;
    }
    if claims.busy(note_id) {
        return;
    }
    if let Err(e) = save::clear_summary_cache(store, note_id) {
        log::warn!("could not clear the summary cache of {note_id}: {e}");
    }
}

/// The blocking part of a title job (KTD13). The caller holds a turn in
/// the summary queue, after a wait of `queued`.
fn send_title(
    env: &Environment,
    claims: &Claims,
    note_id: &str,
    control: &TitleControl,
    queued: Duration,
) -> TitleJob {
    // The job can wait long for its turn.
    if let Some(job) = title_gate(&lock(&env.store), claims, note_id) {
        return job;
    }
    let kept = |why: &str| {
        log::warn!("proposing a title for {note_id} failed: {why}; the note keeps its title");
        drop_title_request(&lock(&env.store), claims, note_id);
        TitleJob::Kept
    };
    let request = match summarize::pending_title_request(&env.store, note_id) {
        Ok(Some(request)) => request,
        Ok(None) => return TitleJob::Skipped,
        Err(e) => return kept(&e.to_string()),
    };
    let endpoint = match env.engines.summary_endpoint() {
        Ok(endpoint) => endpoint,
        // The request stays, and the next launch sends it (R20b).
        Err(EndpointProblem::Locked(message)) => {
            log::info!("the title job of {note_id} waits for the secret store: {message}");
            return TitleJob::Skipped;
        }
        // The summary had an endpoint moments ago. A title is not worth a
        // wait for the settings.
        Err(EndpointProblem::Unconfigured(message)) => return kept(&message),
    };
    let title = match summarize::send_title_request(note_id, &endpoint, &request, control, queued) {
        Ok(Ok(title)) => title,
        Ok(Err(why)) => return kept(&why),
        Err(StageError::Cancelled) => {
            // A quit leaves the request for the next launch. A delete, a
            // Move or a regeneration deletes it.
            // One guard for the check and the drop: a second lock in
            // the same thread waits for ever.
            let store = lock(&env.store);
            if let TitleDue::Dropped = title_due(&store, claims, note_id) {
                drop_title_request(&store, claims, note_id);
            }
            return TitleJob::Skipped;
        }
        Err(e) => return kept(&e.to_string()),
    };

    // The title changes as a rename does: it waits while the Move holds
    // the note, and the folder takes the name under the same lock.
    let mut guard = crate::move_notes::lock_unheld(&env.store, claims, note_id);
    if let Some(job) = title_gate(&guard, claims, note_id) {
        return job;
    }
    let proposed = if title.is_empty() {
        Ok(None)
    } else {
        folder_rename::propose_locked(
            &mut guard,
            note_id,
            &title,
            &|id| claims.busy(id),
            &session_save::RealFs,
        )
    };
    // The title checkpoint comes before the request is deleted, so a
    // resumed Summarize stage stores no request again.
    let job = match proposed {
        Ok(renamed) => {
            let recorded = serde_json::to_string(&title)
                .map_err(StageError::from)
                .and_then(|text| {
                    Ok(guard.put_checkpoint(note_id, Stage::Summarize, summarize::TITLE, &text)?)
                });
            if let Err(e) = recorded {
                log::warn!("could not record the title proposal of {note_id}: {e}");
            }
            match renamed {
                Some(FolderRename::Kept(why)) => {
                    log::warn!("the folder of {note_id} keeps its name: {why}");
                    TitleJob::Proposed(title)
                }
                Some(_) => TitleJob::Proposed(title),
                None => {
                    log::info!("{note_id} keeps its title: the title turn gave no new title");
                    TitleJob::Kept
                }
            }
        }
        Err(e) => {
            log::warn!("could not propose a title for {note_id}: {e}; the note keeps its title");
            TitleJob::Kept
        }
    };
    drop_title_request(&guard, claims, note_id);
    job
}

/// The control of a title job: it stops on shutdown, and when a delete, a
/// Move or a regeneration starts on the note, or the note is no longer
/// saved with a pending request. A title job is a job of the summary
/// queue, so it does not wait while capture is active, and it keeps its
/// power lease (KTD6).
struct TitleControl {
    cancel: CancellationToken,
    env: Arc<Environment>,
    claims: Claims,
    note_id: String,
    power: ActivePower,
}

impl TitleControl {
    fn stopped(&self) -> bool {
        self.cancel.is_cancelled()
            || !matches!(
                title_due(&lock(&self.env.store), &self.claims, &self.note_id),
                TitleDue::Yes
            )
    }
}

impl Control for TitleControl {
    fn proceed(&self) -> Result<(), StageError> {
        if self.stopped() {
            return Err(StageError::Cancelled);
        }
        self.power.resume();
        Ok(())
    }

    fn is_cancelled(&self) -> bool {
        self.stopped()
    }
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

/// The control of a regeneration of a saved note: it stops on shutdown,
/// with no processing state to be taken back from. A regeneration is a
/// job of the summary queue, so it does not wait while capture is active,
/// and it keeps its power lease (KTD6).
struct Unattended {
    cancel: CancellationToken,
    power: ActivePower,
}

impl Control for Unattended {
    fn proceed(&self) -> Result<(), StageError> {
        if self.cancel.is_cancelled() {
            return Err(StageError::Cancelled);
        }
        self.power.resume();
        Ok(())
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

/// The mark that a note waits (KTD10): for the local permit, or for its
/// turn in the summary queue. Making the mark tells the app. Dropping the
/// mark removes it, also when the wait is cancelled. The event that ends
/// the wait (`StageStarted`, or `Waits` with `None`) comes from the caller.
struct WaitMark<'a> {
    runner: &'a Runner,
    id: String,
}

impl<'a> WaitMark<'a> {
    fn new(runner: &'a Runner, id: &str, wait: Wait) -> Self {
        runner.mark(id, wait);
        Self {
            runner,
            id: id.to_string(),
        }
    }
}

impl Drop for WaitMark<'_> {
    fn drop(&mut self) {
        self.runner.unmark(&self.id);
    }
}

/// The log level and the log text at the end of a regeneration (R14): the
/// note, the time that the regeneration waited for its turn, the time
/// that it took after that, and the path of the summary or the error.
fn regeneration_end(
    note_id: &str,
    waited: Duration,
    took: Duration,
    result: &Result<PathBuf, StageError>,
) -> (log::Level, String) {
    let head = format!(
        "regeneration end: note {note_id}, {} ms waiting for its turn, {} ms",
        waited.as_millis(),
        took.as_millis()
    );
    match result {
        Ok(path) => (
            log::Level::Info,
            format!("{head}, saved {}", path.display()),
        ),
        Err(e) => (log::Level::Warn, format!("{head}, failed: {e}")),
    }
}

/// The parts of a note task that end with the task, also when the task is
/// dropped: the heartbeat of the claim (KTD5), the cancel token in
/// `current` (KTD4) and the mark that the note waits for the summary
/// queue. The heartbeat runs from dequeue to the end of the task, so a
/// note that waits for the summary queue keeps its claim.
struct NoteTask<'a> {
    runner: &'a Runner,
    id: String,
    heartbeat: tokio::task::JoinHandle<()>,
}

impl<'a> NoteTask<'a> {
    fn start(runner: &'a Runner, id: &str) -> Self {
        let store = Arc::clone(&runner.env.store);
        let (beat_id, owner) = (id.to_string(), runner.owner.clone());
        let heartbeat = tokio::spawn(async move {
            loop {
                tokio::time::sleep(HEARTBEAT).await;
                if let Err(e) = lock(&store).heartbeat(&beat_id, &owner, now()) {
                    log::warn!("heartbeat for {beat_id} failed: {e}");
                }
            }
        });
        Self {
            runner,
            id: id.to_string(),
            heartbeat,
        }
    }
}

impl Drop for NoteTask<'_> {
    fn drop(&mut self) {
        self.heartbeat.abort();
        self.runner.unmark(&self.id);
        held(&self.runner.audio).remove(&self.id);
        held(&self.runner.current).remove(&self.id);
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
        fn summary_prompt(&self, _: Option<&str>) -> summarize::Prompt {
            unreachable!()
        }
        fn background(&self) -> String {
            unreachable!()
        }
        fn destination(&self) -> Option<PathBuf> {
            unreachable!()
        }
    }

    use std::future::Future;
    use std::sync::atomic::AtomicUsize;
    use std::task::{Context, Poll, Waker};

    /// A runner whose power owner counts each lease that it gives.
    fn counted_runner(
        events: Box<dyn Fn(RunnerEvent) + Send + Sync>,
    ) -> (Arc<Runner>, Arc<AtomicUsize>, tempfile::TempDir) {
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
                events,
            },
            power,
        );
        (runner, acquired, root)
    }

    #[tokio::test]
    async fn an_audio_stage_waiting_for_the_local_permit_acquires_no_power() {
        let (runner, acquired, _root) = counted_runner(Box::new(|event| {
            assert!(
                matches!(
                    event,
                    RunnerEvent::Waits {
                        wait: Some(Wait::AnotherNote),
                        ..
                    }
                ),
                "a waiting stage must not start: {event:?}"
            );
        }));
        // Another job owns the permit, including while it is suspended.
        let _other_job = runner.permit.clone().acquire_owned().await.unwrap();
        let mut attempt = crate::recognition::durable::Attempt::default();
        let stage = runner.run_stage(
            "waiting",
            "model",
            Stage::Assemble,
            CancellationToken::new(),
            &mut attempt,
            Duration::ZERO,
        );
        let mut stage = Box::pin(stage);
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(stage.as_mut().poll(&mut context), Poll::Pending));
        assert_eq!(
            acquired.load(Ordering::SeqCst),
            0,
            "waiting for another job must not acquire processing power"
        );
        let processing = SessionState::Processing { stopped_at: now() };
        assert_eq!(
            runner
                .waits()
                .of("waiting", &processing, Some(Stage::Assemble)),
            Some(Wait::AnotherNote)
        );
        // A cancelled wait leaves no mark.
        drop(stage);
        assert_eq!(
            runner
                .waits()
                .of("waiting", &processing, Some(Stage::Assemble)),
            None
        );
    }

    /// The wait of a note from the runner's marks, the claimed notes and
    /// the capture state (KTD10).
    #[test]
    fn a_note_waits_for_the_recording_only_before_an_audio_stage() {
        let processing = SessionState::Processing { stopped_at: now() };
        let queued = SessionState::Queued { stopped_at: now() };
        let mut waits = Waits {
            recording: true,
            ..Waits::default()
        };
        assert_eq!(
            waits.of("a", &processing, Some(Stage::Transcribe)),
            Some(Wait::Recording)
        );
        assert_eq!(waits.of("a", &queued, None), Some(Wait::Recording));
        assert_eq!(waits.of("a", &processing, Some(Stage::Summarize)), None);
        // A note that waits for its summary turn says so also during a
        // recording.
        waits.marks.insert("a".into(), Wait::SummaryTurn);
        assert_eq!(
            waits.of("a", &processing, Some(Stage::Summarize)),
            Some(Wait::SummaryTurn)
        );
        assert_eq!(
            waits.of("a", &SessionState::Saved, None),
            Some(Wait::SummaryTurn),
            "a regeneration waits for its turn"
        );

        let mut waits = Waits::default();
        assert_eq!(waits.of("q", &queued, None), None);
        waits.audio.insert("other".into());
        assert_eq!(waits.of("q", &queued, None), Some(Wait::AnotherNote));
        waits.no_model.insert("q".into());
        assert_eq!(waits.of("q", &queued, None), Some(Wait::Model));
        assert_eq!(
            waits.of("other", &processing, Some(Stage::Assemble)),
            None,
            "a note in its audio stages does not wait for itself"
        );
        assert_eq!(
            waits.of("q", &SessionState::Saved, None),
            None,
            "only a queued note waits for a model"
        );
    }

    #[tokio::test]
    async fn a_note_waiting_for_the_summary_queue_acquires_no_power_and_frees_the_local_permit() {
        let (runner, acquired, _root) = counted_runner(Box::new(|event| {
            assert!(
                matches!(
                    event,
                    RunnerEvent::Waits {
                        wait: Some(Wait::SummaryTurn),
                        ..
                    }
                ),
                "a waiting note must not start: {event:?}"
            );
        }));
        // A regeneration has the turn of the summary queue.
        let regeneration = runner.summary.acquire(Class::Regeneration).await;
        let cancel = CancellationToken::new();
        let wait = runner.summary_turn("waiting", Stage::Summarize, &cancel);
        let mut wait = std::pin::pin!(wait);
        let mut context = Context::from_waker(Waker::noop());
        assert!(wait.as_mut().poll(&mut context).is_pending());
        assert!(runner.waits_to_summarize("waiting"));
        assert_eq!(acquired.load(Ordering::SeqCst), 0);
        assert_eq!(
            runner.permit.available_permits(),
            1,
            "the next note can start its audio stages"
        );
        // A cancel stops the wait, and the note leaves the queue.
        cancel.cancel();
        assert!(matches!(
            wait.as_mut().poll(&mut context),
            Poll::Ready(Err(StageError::Cancelled))
        ));
        assert!(!runner.waits_to_summarize("waiting"));
        drop(regeneration);
        assert!(!runner.summary.is_active());
    }

    #[tokio::test]
    async fn the_runner_works_while_a_job_runs_or_waits_in_either_queue() {
        let (runner, _, _root) = counted_runner(Box::new(|_| {}));
        assert!(!runner.is_working(), "a runner with no job is not working");

        let stage = runner.permit.clone().acquire_owned().await.unwrap();
        assert!(runner.is_working(), "an audio stage runs");
        drop(stage);
        assert!(!runner.is_working());

        let turn = runner.summary.acquire(Class::Title).await;
        assert!(runner.is_working(), "a job of the summary queue runs");
        let queue = runner.summary.clone();
        let waits = queue.acquire(Class::NewNote);
        let mut waits = std::pin::pin!(waits);
        assert!(
            waits
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(turn);
        assert!(runner.is_working(), "a job waits for its turn");
        let Poll::Ready(turn) = waits.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
            panic!("the waiting job gets the turn");
        };
        drop(turn);
        assert!(!runner.is_working());

        // A note has its task between two stages, with no permit and no
        // turn.
        runner
            .current
            .lock()
            .unwrap()
            .insert("s".into(), CancellationToken::new());
        assert!(runner.is_working(), "a note task is in flight");
        runner.current.lock().unwrap().remove("s");
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

    /// R14: a failed regeneration writes one warning line with the error.
    #[test]
    fn a_failed_regeneration_writes_one_end_line_with_the_error() {
        let failed = Err(StageError::Failed("the endpoint did not answer".into()));
        let (level, line) = regeneration_end(
            "2026-10-14-0900",
            Duration::from_millis(4_200),
            Duration::from_millis(91_000),
            &failed,
        );
        assert_eq!(level, log::Level::Warn);
        assert_eq!(
            line,
            "regeneration end: note 2026-10-14-0900, 4200 ms waiting for its turn, \
             91000 ms, failed: the endpoint did not answer"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn a_regeneration_that_saves_writes_its_end_line_at_info_with_the_path() {
        let saved = Ok(PathBuf::from("/notes/Weekly sync/summary.md"));
        let (level, line) =
            regeneration_end("s", Duration::ZERO, Duration::from_millis(30), &saved);
        assert_eq!(level, log::Level::Info);
        assert_eq!(
            line,
            "regeneration end: note s, 0 ms waiting for its turn, 30 ms, \
             saved /notes/Weekly sync/summary.md"
        );
    }
}
