//! Wires the pipeline runner (U8) into the app: models from the models
//! directory, suspension while capture is active (KTD20), and runner events
//! out to the surface.

use std::sync::Arc;

use asr::{Engine, Recognizer};
use diarize::{DiarizeError, OnnxAnalyzer, SpeakerAnalyzer};
use models::ModelKind;
use pipeline::early_home::EarlyHomes;
use pipeline::empty_note::{Leave, Removal};
use pipeline::in_use::InUseKeeper;
use pipeline::reprocess::{Readiness, Refusal};
use pipeline::{
    EndpointProblem, EngineProvider, Environment, Runner, RunnerEvent, SummaryEndpoint,
};
use secrets::{SecretError, SecretStore};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;

use crate::managers::audio_removal::AudioRemovalManager;
use crate::managers::capture::CaptureManager;
use crate::managers::import::ImportManager;
use crate::managers::library::{LibraryChanged, LibraryWatcher, Timing};
use crate::managers::live::LiveManager;
use crate::managers::models::{ModelState, ModelsManager};
use crate::managers::settings;
use crate::managers::shared::SharedSettingsManager;
use crate::managers::watched_folder::{
    self, Environment as WatchedEnvironment, Timing as WatchedTiming, WatchedFolder,
    WatchedFolderChanged, WatchedFolderStatus,
};

/// What processing is doing, for the surface.
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct ProcessingUpdate {
    pub note_id: String,
    pub update: ProcessingStep,
}

#[derive(Serialize, Clone, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessingStep {
    StageStarted {
        stage: String,
    },
    /// `summary_step` is the step of a summary in parts: "parts", "groups"
    /// or "final". It is `None` for the transcribe stage.
    Progress {
        stage: String,
        segment: u32,
        summary_step: Option<String>,
        done: u32,
        total: u32,
    },
    /// The speaker stage's progress over the whole note (KTD1).
    SpeakerProgress(SpeakerProgressView),
    StageFailed {
        stage: String,
        message: String,
    },
    /// The secret store holding the endpoint credential is locked (R20b).
    Blocked {
        message: String,
    },
    /// Speaker separation failed; the transcript comes without speakers.
    SpeakersUnavailable {
        message: String,
    },
    /// A model is not on disk; the session keeps its place in the queue.
    Waiting {
        what: String,
    },
    /// What the note waits for changed (KTD10). `None` when its wait
    /// ended. The note's entry gives the current wait.
    Waits {
        wait: Option<WaitView>,
    },
    /// Summarization failed; the session is saved with a stub summary.
    SummaryUnavailable {
        message: String,
    },
    /// The destination cannot take the session; it waits, whole, until
    /// one is chosen or comes back.
    AwaitingDestination {
        problem: String,
    },
    /// In the destination. `copied` when it crossed volumes, so the move
    /// was a verified copy rather than one atomic step.
    Saved {
        path: String,
        copied: bool,
    },
    /// A Transcribe Again run failed; the session keeps its previous
    /// transcript and summary (R19).
    TranscribeAgainFailed {
        stage: String,
        message: String,
    },
    Cancelled,
    Finished,
}

/// What a note waits for before its next stage can run (KTD10).
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum WaitView {
    /// A recording is active, and the next stage is an audio stage.
    Recording,
    /// Another note has the local permit or is in its audio stages.
    AnotherNote,
    /// The note waits for its turn in the summary queue: a new note, or a
    /// regeneration of a saved note.
    SummaryTurn,
    /// A model that the note needs is not on disk.
    Model,
}

impl From<pipeline::runner::Wait> for WaitView {
    fn from(wait: pipeline::runner::Wait) -> Self {
        use pipeline::runner::Wait;
        match wait {
            Wait::Recording => Self::Recording,
            Wait::AnotherNote => Self::AnotherNote,
            Wait::SummaryTurn => Self::SummaryTurn,
            Wait::Model => Self::Model,
        }
    }
}

/// What the Move of the notes to a new storage folder does (KTD9).
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum MoveState {
    /// The Move puts the notes in the new folder.
    Running,
    /// A note records, saves or is processed: it moves when it is free.
    Waiting,
    /// A full disk or a folder that is not available stopped the Move.
    /// Retry continues it.
    Paused,
    /// Every note is in the new folder.
    Finished,
}

/// Why the Move is paused, with the text that Settings shows.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type)]
pub struct MovePauseCause {
    pub kind: MovePauseKind,
    pub message: String,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum MovePauseKind {
    DiskFull,
    FolderNotAvailable,
    Failed,
}

impl From<&session_core::MovePause> for MovePauseCause {
    fn from(pause: &session_core::MovePause) -> Self {
        use session_core::MovePause as P;
        let (kind, message) = match pause {
            P::DiskFull { folder } => (
                MovePauseKind::DiskFull,
                format!("The disk of {folder} is full. Make space on it, then select Retry."),
            ),
            P::NotReachable { folder } => (
                MovePauseKind::FolderNotAvailable,
                format!("{folder} is not available. Connect its disk, then select Retry."),
            ),
            P::Failed { message } => (
                MovePauseKind::Failed,
                format!("The notes could not be moved: {message}. Select Retry to try again."),
            ),
        };
        Self { kind, message }
    }
}

/// The progress of the Move, sent after each note and when the state
/// changes.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type, tauri_specta::Event)]
pub struct MoveProgress {
    pub moved: u32,
    pub total: u32,
    pub state: MoveState,
}

impl MoveProgress {
    /// The progress of `(moved, total)` notes in `state`.
    fn new((moved, total): (u32, u32), state: MoveState) -> Self {
        Self {
            moved,
            total,
            state,
        }
    }
}

/// The speaker stage's progress in one processing attempt. Only the stage's
/// completion means 100%.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type)]
pub struct SpeakerProgressView {
    /// A view keeps the value of the current attempt only.
    pub attempt: String,
    /// The measured share of the whole note, 0 to 99. `None` before the
    /// first measurement.
    pub percent: Option<u32>,
    /// False while the stage does work that it cannot measure.
    pub measuring: bool,
}

impl From<pipeline::progress::SpeakerProgress> for SpeakerProgressView {
    fn from(progress: pipeline::progress::SpeakerProgress) -> Self {
        Self {
            attempt: progress.attempt,
            percent: progress.work.percent.map(u32::from),
            measuring: progress.work.measuring,
        }
    }
}

/// The Move paused (R10).
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type, tauri_specta::Event)]
pub struct MovePaused {
    pub cause: MovePauseCause,
}

/// The open Move, for Settings > Storage.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Type)]
pub struct MoveStatus {
    pub old_folder: String,
    pub new_folder: String,
    pub moved: u32,
    pub total: u32,
    pub state: MoveState,
    pub cause: Option<MovePauseCause>,
}

/// The Move worker: one thread while a Move journal is open.
#[derive(Default)]
struct MoveWorker {
    state: std::sync::Mutex<MoveWorkerState>,
    signal: std::sync::Condvar,
}

#[derive(Default)]
struct MoveWorkerState {
    running: bool,
    /// A Retry or a new Move came while the worker waited.
    woken: bool,
    /// The app quits.
    stopped: bool,
    /// The state of the last pass.
    last: Option<MoveState>,
}

/// How long the worker waits before a pass tries a busy note again.
const MOVE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

pub(crate) fn stage_name(stage: session_core::Stage) -> String {
    serde_json::to_value(stage)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// What the app sends for a runner event.
enum Sent {
    Processing(ProcessingUpdate),
    /// A proposed title changes a saved note and its folder name, as a
    /// sync does. Home and the note page read the note again on it
    /// (KTD13).
    Library(LibraryChanged),
}

impl From<RunnerEvent> for Sent {
    fn from(event: RunnerEvent) -> Self {
        let (note_id, update) = match event {
            RunnerEvent::TitleProposed { note_id, .. } => {
                return Self::Library(LibraryChanged {
                    changed: vec![note_id],
                    removed: Vec::new(),
                });
            }
            RunnerEvent::Waits { note_id, wait } => (
                note_id,
                ProcessingStep::Waits {
                    wait: wait.map(WaitView::from),
                },
            ),
            RunnerEvent::StageStarted { note_id, stage } => (
                note_id,
                ProcessingStep::StageStarted {
                    stage: stage_name(stage),
                },
            ),
            RunnerEvent::Progress {
                note_id,
                stage,
                segment,
                summary_step,
                done,
                total,
            } => (
                note_id,
                ProcessingStep::Progress {
                    stage: stage_name(stage),
                    segment,
                    summary_step: summary_step.map(|s| s.name().to_string()),
                    done: done as u32,
                    total: total as u32,
                },
            ),
            RunnerEvent::SpeakerProgress { note_id, progress } => {
                (note_id, ProcessingStep::SpeakerProgress(progress.into()))
            }
            RunnerEvent::StageFailed {
                note_id,
                stage,
                message,
            } => (
                note_id,
                ProcessingStep::StageFailed {
                    stage: stage_name(stage),
                    message,
                },
            ),
            RunnerEvent::SpeakersUnavailable { note_id, message } => {
                (note_id, ProcessingStep::SpeakersUnavailable { message })
            }
            RunnerEvent::Blocked { note_id, message } => {
                (note_id, ProcessingStep::Blocked { message })
            }
            RunnerEvent::Waiting { note_id, what } => (note_id, ProcessingStep::Waiting { what }),
            RunnerEvent::SummaryUnavailable { note_id, message } => {
                (note_id, ProcessingStep::SummaryUnavailable { message })
            }
            RunnerEvent::AwaitingDestination { note_id, problem } => {
                (note_id, ProcessingStep::AwaitingDestination { problem })
            }
            RunnerEvent::Saved {
                note_id,
                path,
                copied,
            } => (
                note_id,
                ProcessingStep::Saved {
                    path: path.to_string_lossy().into_owned(),
                    copied,
                },
            ),
            RunnerEvent::TranscribeAgainFailed {
                note_id,
                stage,
                message,
            } => (
                note_id,
                ProcessingStep::TranscribeAgainFailed {
                    stage: stage_name(stage),
                    message,
                },
            ),
            RunnerEvent::Cancelled { note_id } => (note_id, ProcessingStep::Cancelled),
            RunnerEvent::Finished { note_id } => (note_id, ProcessingStep::Finished),
        };
        Self::Processing(ProcessingUpdate { note_id, update })
    }
}

/// The message when no provider is active (R11, AE2).
const NO_PROVIDER: &str = "no AI provider is set, so the note is saved without a summary";

/// The summary endpoint of the active provider in `s`, with the key that
/// `read_key` reads from the provider's Keychain location (KTD8). The
/// request gate (R12) applies: no provider, or a cloud provider with no
/// saved key, gives `Unconfigured`, and no request goes out. A job takes
/// this snapshot when it starts (R25).
fn endpoint_from_settings(
    s: settings::Settings,
    read_key: impl FnOnce(&settings::KeyLocation) -> Result<Option<String>, SecretError>,
) -> Result<SummaryEndpoint, EndpointProblem> {
    use settings::{EndpointGate, NotRequestedReason};
    let Some(provider) = s.active_provider else {
        return Err(EndpointProblem::Unconfigured(NO_PROVIDER.into()));
    };
    let address = s.provider_address(provider);
    let entry = s.providers.get(provider).clone();
    if address.is_empty() || entry.model.trim().is_empty() {
        return Err(EndpointProblem::Unconfigured(
            "the AI provider has no address or no model; choose them in Settings".into(),
        ));
    }
    // The key is kept for the provider and its host (R10).
    let location = s.key_location(provider);
    let read = location.as_ref().map_or(Ok(None), read_key);
    let key = settings::key_state(&read);
    match settings::provider_gate(Some(provider), true, &address, key) {
        EndpointGate::NotRequested(NotRequestedReason::KeyNeeded) => {
            let host = location.map_or_else(|| address.clone(), |l| l.host());
            return Err(EndpointProblem::Unconfigured(format!(
                "Add an API key for {host} in Settings"
            )));
        }
        EndpointGate::NotRequested(
            NotRequestedReason::NoProvider
            | NotRequestedReason::NoAddress
            | NotRequestedReason::NotChosen,
        ) => {
            return Err(EndpointProblem::Unconfigured(NO_PROVIDER.into()));
        }
        // The Keychain error below gives the message.
        EndpointGate::NotRequested(
            NotRequestedReason::KeychainLocked | NotRequestedReason::KeychainFailed,
        )
        | EndpointGate::RequestWithKey
        | EndpointGate::RequestWithoutKey => {}
    }
    let credential = read
        .map_err(|e| match e {
            SecretError::Locked { .. } | SecretError::Unavailable { .. } => {
                EndpointProblem::Locked(e.to_string())
            }
            other => EndpointProblem::Unconfigured(other.to_string()),
        })?
        // A blank key is no key (see `key_state`): send no Bearer header.
        .filter(|key| !key.trim().is_empty());
    Ok(SummaryEndpoint {
        provider,
        base_url: address,
        model: entry.model,
        credential,
        context_limit: entry.context_limit_tokens,
        retry_waits: summarize::client::RETRY_WAITS.to_vec(),
    })
}

/// Resolve the private helper beside the application, without a PATH search.
fn packaged_nemo_helper() -> Result<std::path::PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let parent = executable
        .parent()
        .ok_or("The application path has no parent")?;
    #[cfg(target_os = "macos")]
    let packaged = parent.join("../Frameworks/LonghandNeMo/longhand-nemo-helper");
    #[cfg(not(target_os = "macos"))]
    let packaged = parent.join("nemo-runtime/bin/longhand-nemo-helper.exe");
    if packaged.is_file() {
        return Ok(packaged);
    }
    #[cfg(debug_assertions)]
    {
        let name = if cfg!(target_os = "windows") {
            "longhand-nemo-helper.exe"
        } else {
            "longhand-nemo-helper"
        };
        let development = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("nemo-runtime/bin")
            .join(name);
        if development.is_file() {
            return Ok(development);
        }
    }
    Err("The application package does not contain its NeMo helper".into())
}

struct Models {
    app: AppHandle,
    models: Arc<ModelsManager>,
    secrets: Arc<SecretStore>,
}

impl EngineProvider for Models {
    fn ready_exact(&self, spec: &models::recognition::ExecutionSpec) -> bool {
        self.models.missing_spec(spec).is_empty()
    }

    fn artifact_path(
        &self,
        artifact: &models::recognition::ArtifactSpec,
    ) -> Result<std::path::PathBuf, String> {
        let path = self
            .models
            .artifact_path(artifact)
            .ok_or_else(|| format!("The selected artifact is not installed: {}", artifact.id))?;
        // This runs on the processing worker. UI readiness checks use metadata.
        let actual = models::download::compute_sha256(&path).map_err(|error| error.to_string())?;
        if actual != artifact.sha256 {
            return Err(format!(
                "The installed artifact does not match its selected SHA-256: {}",
                artifact.id
            ));
        }
        Ok(path)
    }

    fn load_exact(
        &self,
        spec: &models::recognition::ExecutionSpec,
    ) -> Result<Box<dyn Recognizer>, String> {
        if !spec.runtimes.iter().any(|runtime| {
            runtime.name == "transcribe.cpp"
                && models::recognition::runtime_accepted(&runtime.name, &runtime.revision)
        }) {
            return Err("The selected transcription runtime is not installed".into());
        }
        let artifact = spec
            .artifacts
            .iter()
            .find(|artifact| {
                artifact.id == spec.config.asr_model && artifact.format == "transcribe-gguf"
            })
            .ok_or("The selected transcription artifact is missing from the note configuration")?;
        let path = self.artifact_path(artifact)?;
        let mut engine = Engine::load_with(
            &spec.config.asr_model,
            &path,
            spec.capabilities.live_transcription,
        )
        .map_err(|error| error.to_string())?;
        engine.set_language(spec.config.language.clone());
        Ok(Box::new(engine))
    }

    fn analyzer_exact(
        &self,
        spec: &models::recognition::ExecutionSpec,
    ) -> Result<Box<dyn SpeakerAnalyzer>, DiarizeError> {
        let path = |id: &str| {
            let artifact = spec
                .artifacts
                .iter()
                .find(|artifact| artifact.id == id)
                .ok_or_else(|| DiarizeError::MissingWeights(id.into()))?;
            self.artifact_path(artifact)
                .map_err(DiarizeError::MissingWeights)
        };
        OnnxAnalyzer::load(
            &path("pyannote-segmentation-3.0")?,
            &path("wespeaker-voxceleb-resnet34-lm")?,
        )
        .map(|analyzer| Box::new(analyzer) as Box<dyn SpeakerAnalyzer>)
    }

    fn nemo_helper(&self) -> Result<std::path::PathBuf, String> {
        packaged_nemo_helper()
    }

    fn nemo_gpu(&self) -> i32 {
        0
    }

    fn summary_endpoint(&self) -> Result<SummaryEndpoint, EndpointProblem> {
        endpoint_from_settings(settings::get_settings(&self.app), |location| {
            location.read(self.secrets.as_ref())
        })
    }

    fn summary_prompt(&self, note_type: Option<&str>) -> summarize::Prompt {
        settings::get_settings(&self.app).summary_prompt_for_type(note_type)
    }

    /// The Background of the shared view. It is the cached text while the
    /// Background file or the storage folder is not available (KTD3).
    fn background(&self) -> String {
        settings::get_background(&self.app)
    }

    /// Read when the note is saved, so a note that records during a change
    /// of folder saves to the new storage folder (AE7).
    fn destination(&self) -> Option<std::path::PathBuf> {
        settings::get_settings(&self.app).storage_folder()
    }

    /// Everything processing needs, diarization weights included: a
    /// session is processed once its models are all on disk.
    fn ready(&self, model_id: &str) -> bool {
        self.models.missing(model_id).is_empty()
    }

    fn load(&self, model_id: &str) -> Result<Box<dyn Recognizer>, String> {
        let path = self
            .models
            .file_path(model_id)
            .ok_or_else(|| format!("{model_id} is not a known model"))?;
        // The catalog says whether the model streams, so the live worker
        // gets a stream from the engine that processing also uses (KTD5).
        let streaming = models::model(model_id).is_some_and(|m| m.streaming);
        Engine::load_with(model_id, &path, streaming)
            .map(|e| Box::new(e) as Box<dyn Recognizer>)
            .map_err(|e| e.to_string())
    }

    fn default_model(&self) -> String {
        settings::get_settings(&self.app).asr_model
    }

    fn analyzer(&self) -> Result<Box<dyn SpeakerAnalyzer>, DiarizeError> {
        let path = |kind: ModelKind| {
            models::CATALOG
                .iter()
                .find(|m| m.kind == kind)
                .and_then(|m| self.models.file_path(&m.id))
                .ok_or_else(|| DiarizeError::MissingWeights(format!("{kind:?} weights")))
        };
        let analyzer = OnnxAnalyzer::load(
            &path(ModelKind::Segmentation)?,
            &path(ModelKind::Embedding)?,
        )?;
        Ok(Box::new(analyzer))
    }
}

/// A session finished while the window was away (R15). Nothing is posted
/// while the window is in front: the surface already says so.
/// Remove staging folders that completed sessions left in the storage
/// folder (U18). Only with the instance lock held, so no other run is
/// saving into them.
fn sweep_staging(app: &AppHandle, store: &pipeline::SharedStore) {
    for root in library_roots(app, store) {
        match session_save::sweep_staging(&root, |id| completed(store, id)) {
            Ok(removed) => {
                for path in removed {
                    log::info!("removed leftover staging folder {}", path.display());
                }
            }
            Err(e) => log::warn!(
                "could not sweep {} for staging folders: {e}",
                root.display()
            ),
        }
    }
}

/// The folders that the library lists notes from: the storage folder (R1)
/// and, while a Move journal is open, the old folder (KTD10). Empty only
/// before the launch sets a storage folder.
fn library_roots(app: &AppHandle, store: &pipeline::SharedStore) -> Vec<std::path::PathBuf> {
    let storage = settings::get_settings(app).storage_folder();
    let store = store.lock().unwrap_or_else(|e| e.into_inner());
    pipeline::move_notes::library_roots(&store, storage)
}

/// Forget the rows of the notes that are in none of the library roots, for
/// example after a change of folder with Leave them (R7, R8).
fn forget_outside(
    app: &AppHandle,
    store: &pipeline::SharedStore,
    claimed: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let roots = library_roots(app, store);
    let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
    library::reconcile::forget_outside(&mut store, &roots, claimed).unwrap_or_else(|e| {
        log::error!("could not forget the notes outside the storage folder: {e}");
        Vec::new()
    })
}

/// Whether the staging folder of `id` is left over from a save that
/// completed: the id has a Saved row. A first save's staging has an id
/// that no other computer knows, so its row is not Saved until the save
/// completes, and an id with no row keeps its staging (KTD9). A note that
/// opened with no recording is Saved before its early write is done: its
/// staging is that write's, and stays (KTD4).
fn completed(store: &pipeline::SharedStore, id: &str) -> bool {
    let store = store.lock().unwrap_or_else(|e| e.into_inner());
    store
        .session(id)
        .ok()
        .flatten()
        .is_some_and(|r| r.state == session_core::SessionState::Saved)
        && !pipeline::early_home::unfinished(&store, id).unwrap_or(true)
}

/// The runs whose in-use markers are this computer's own work, this run
/// `run_id` first (KTD5). The store adds this run to the runs of this
/// computer. A quit or a crash keeps the markers of a run, and they stay
/// fresh for a time, so the earlier runs are this computer's too. Only
/// with the instance lock held: without it, another run on this computer
/// can be alive and work on its notes. When the store cannot add the run,
/// only this run counts, and the marker of an earlier run counts as work on
/// another computer.
fn own_runs(
    store: &pipeline::SharedStore,
    run_id: &str,
    liveness: session_core::jobs::Liveness,
) -> Vec<String> {
    let earlier = store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .add_run(run_id)
        .unwrap_or_else(|e| {
            log::error!("could not add this run to the runs of this computer: {e}");
            Vec::new()
        });
    let mut runs = vec![run_id.to_string()];
    if matches!(liveness, session_core::jobs::Liveness::OnlyInstance) {
        runs.extend(earlier);
    }
    runs
}

/// Refuses a change of folder while a Move runs or is paused (KTD11).
pub const MOVE_OPEN: &str =
    "Longhand is moving your notes. Wait until the move is done, then change the folder.";

fn emit_move(app: &AppHandle, progress: MoveProgress) {
    if let Err(e) = progress.emit(app) {
        log::warn!("could not emit the move progress: {e}");
    }
}

fn emit_library_changed(app: &AppHandle, changes: LibraryChanged) {
    if let Err(e) = changes.emit(app) {
        log::warn!("could not emit a library change: {e}");
    }
}

/// The title and the body of the save notification of the note folder
/// `folder`. The body says that the note has a summary only when
/// `has_summary` is true.
fn saved_notice(folder: &str, has_summary: bool) -> (&'static str, String) {
    let body = if has_summary {
        format!("{folder} has its transcript and summary.")
    } else {
        format!("{folder} has its transcript. The summary could not be made.")
    };
    ("Note ready", body)
}

/// Whether the Summarize stage made the summary of `note_id`: the stage
/// is complete for each segment and no summary failure is recorded. In
/// the other cases the save writes the stub summary. A read error gives
/// false.
fn summary_made(store: &pipeline::SharedStore, note_id: &str) -> bool {
    use session_core::Stage;
    let read = || -> Result<bool, pipeline::StageError> {
        let segments = store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .segments(note_id)?
            .len();
        Ok(
            pipeline::checkpoint::is_complete(store, note_id, Stage::Summarize, segments)?
                && pipeline::checkpoint::get::<String>(
                    store,
                    note_id,
                    Stage::Summarize,
                    pipeline::save::SUMMARY_FAILED,
                )?
                .is_none(),
        )
    };
    read().unwrap_or_else(|e| {
        log::warn!("could not read if the note {note_id} has a summary: {e}");
        false
    })
}

/// Whether `event` ends the work on a note: its in-use marker goes at
/// Saved, and a failed Transcribe Again run is Saved again.
fn ends_work(event: &RunnerEvent) -> bool {
    matches!(
        event,
        RunnerEvent::Saved { .. }
            | RunnerEvent::StageFailed { .. }
            | RunnerEvent::Cancelled { .. }
            | RunnerEvent::TranscribeAgainFailed { .. }
    )
}

/// True while a recording runs, or while the live worker saves its last
/// part after Stop and holds the engine (KTD9): the audio stages and the
/// conversion of an import wait. It locks the capture state, so a caller
/// must not hold the store lock.
fn suspended(
    capture: &Arc<CaptureManager>,
    live: &Arc<LiveManager>,
) -> impl Fn() -> bool + Send + Sync + 'static {
    let (capture, live) = (Arc::clone(capture), Arc::clone(live));
    move || capture.is_capturing() || live.busy()
}

fn notify_saved(
    app: &AppHandle,
    store: &pipeline::SharedStore,
    note_id: &str,
    path: &std::path::Path,
) {
    use tauri::Manager;
    use tauri_plugin_notification::NotificationExt;
    let visible = app
        .get_webview_window(crate::window::MAIN_WINDOW)
        .and_then(|w| Some(w.is_visible().ok()? && w.is_focused().ok()?))
        .unwrap_or(false);
    if visible {
        return;
    }
    let folder = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (title, body) = saved_notice(&folder, summary_made(store, note_id));
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!("could not post the save notification: {e}");
    }
}

/// The readiness of a run whose artifacts `missing` are not on disk. An
/// artifact that is not downloading comes first: the user must start its
/// download in Settings.
fn readiness_of(missing: Vec<String>, downloading: impl Fn(&str) -> bool) -> Readiness {
    match missing.iter().find(|id| !downloading(id)) {
        Some(id) => Readiness::Missing {
            artifact: id.clone(),
        },
        None => match missing.into_iter().next() {
            Some(artifact) => Readiness::Downloading { artifact },
            None => Readiness::Ready,
        },
    }
}

pub struct ProcessingManager {
    runner: Arc<Runner>,
    live: Arc<LiveManager>,
    app: AppHandle,
    /// The Transcribe Again check reads if the artifacts of a run are on
    /// disk (KTD2).
    models: Arc<ModelsManager>,
    store: pipeline::SharedStore,
    liveness: session_core::jobs::Liveness,
    library: std::sync::Mutex<LibrarySlot>,
    /// What the latest library scans found. The launch scan and the
    /// watcher write it, and the note commands read it (KTD10).
    library_status: Arc<library::status::LibraryStatus>,
    /// The Move of the notes to a new storage folder (KTD9).
    mover: MoveWorker,
    /// The in-use markers of the saved notes that this run resumes or
    /// processes (KTD4 of the audio removal plan).
    in_use: Arc<InUseKeeper>,
    /// The runs whose in-use markers are this computer's own work, this
    /// run first ([`own_runs`]).
    own_runs: Vec<String>,
    /// The passes that remove the audio of old notes (U6 of the audio
    /// removal plan).
    audio_removal: Arc<AudioRemovalManager>,
    /// The early writes of the home folders of new notes (KTD4).
    early_homes: Arc<EarlyHomes>,
    /// The imports of audio files (U4 of the audio import plan).
    imports: Arc<ImportManager>,
    /// Held by each pass of the bulk edits: the type rename, merge and
    /// delete edits (KTD7) and the tag rename and delete edits (KTD11), so
    /// two passes of one edit do not run at the same time.
    bulk_edits: std::sync::Mutex<()>,
    /// The watcher of the watched folder (U6 of the audio import plan).
    /// It starts after the import worker, with the instance lock held.
    watched: std::sync::Mutex<WatchedSlot>,
}

/// The watcher of the watched folder, and whether the quit stopped it.
#[derive(Default)]
struct WatchedSlot {
    watcher: Option<WatchedFolder>,
    stopped: bool,
}

/// The library watcher, which starts at the end of the launch steps
/// (KTD9), and whether the quit stopped it.
#[derive(Default)]
struct LibrarySlot {
    watcher: Option<LibraryWatcher>,
    stopped: bool,
}

impl ProcessingManager {
    /// Build the runner, return sessions a previous run left mid-stage to
    /// the queue, remove leftover staging folders and settle the folder
    /// renames that a crash left (KTD9). Call [`ProcessingManager::start`]
    /// for the rest of the launch steps.
    pub fn new(
        app: &AppHandle,
        capture: &Arc<CaptureManager>,
        live: &Arc<LiveManager>,
        models: Arc<ModelsManager>,
        secrets: Arc<SecretStore>,
        liveness: session_core::jobs::Liveness,
    ) -> Self {
        let events = app.clone();
        let saved = capture.shared_store();
        let runner = Runner::new(Environment {
            store: capture.shared_store(),
            working_root: capture.working_root().to_path_buf(),
            engines: Box::new(Models {
                app: app.clone(),
                models: Arc::clone(&models),
                secrets,
            }),
            suspended: Box::new(suspended(capture, live)),
            events: Box::new(move |event| {
                if let RunnerEvent::Saved { note_id, path, .. } = &event {
                    notify_saved(&events, &saved, note_id, path);
                }
                if ends_work(&event)
                    && let Some(processing) = events.try_state::<Arc<ProcessingManager>>()
                {
                    processing.in_use_changed();
                    // A type move that waited for the work can run now
                    // (R5).
                    processing.run_type_moves_soon();
                }
                // A first Save that waits for the early write of its home
                // folder in the storage folder: the write starts now. The
                // job of a note that has one gets more work, which does no
                // harm.
                if let RunnerEvent::AwaitingDestination { note_id, problem } = &event
                    && let Some(processing) = events.try_state::<Arc<ProcessingManager>>()
                {
                    processing.start_unfinished_early_write(note_id, problem);
                }
                match Sent::from(event) {
                    Sent::Processing(update) => {
                        if let Err(e) = update.emit(&events) {
                            log::warn!("could not emit a processing update: {e}");
                        }
                    }
                    Sent::Library(changes) => emit_library_changed(&events, changes),
                }
            }),
        });
        match runner.reclaim(liveness) {
            Ok(ids) if !ids.is_empty() => {
                log::info!("returned {} session(s) to the queue", ids.len())
            }
            Ok(_) => {}
            Err(e) => log::error!("launch sweep of the job queue failed: {e}"),
        }
        if matches!(liveness, session_core::jobs::Liveness::OnlyInstance) {
            sweep_staging(app, &capture.shared_store());
            // Before the library scan: a folder rename that a crash stopped
            // completes (KTD8, KTD9). Only with the instance lock held, so
            // no other run renames a folder at the same time.
            if let Err(e) = runner.resolve_rename_intents() {
                log::error!("launch recovery of folder renames failed: {e}");
            }
        }
        // The run ID of the in-use markers is the claim owner of this run.
        let in_use = InUseKeeper::new(capture.shared_store(), runner.owner());
        let own_runs = own_runs(&capture.shared_store(), runner.owner(), liveness);
        // The early writes use the same run ID for their in-use markers.
        // After each write, the in-use markers refresh, and a note whose
        // save waits for its folder tries again.
        let storage = app.clone();
        let written = app.clone();
        let early_homes = EarlyHomes::new(
            capture.shared_store(),
            runner.owner(),
            Box::new(move || settings::get_settings(&storage).storage_folder()),
            Box::new(move |_| {
                if let Some(processing) = written.try_state::<Arc<ProcessingManager>>() {
                    processing.in_use_changed();
                    // Also a note whose home folder outside the storage
                    // folder could not be read before tries again.
                    processing.retry_awaiting();
                }
            }),
        )
        .with_earlier_runs(own_runs.get(1..).unwrap_or_default().to_vec());
        // The conversion of an import waits as the audio stages wait
        // (KTD9 of the audio import plan).
        let imports = Arc::new(ImportManager::new(
            app,
            capture,
            Box::new(suspended(capture, live)),
            runner.claims().clone(),
            Arc::clone(&early_homes),
        ));
        let library_status: Arc<library::status::LibraryStatus> = Arc::default();
        let audio_removal = AudioRemovalManager::new(
            app,
            capture.shared_store(),
            runner.claims().clone(),
            Arc::clone(&library_status),
        );
        Self {
            in_use,
            own_runs,
            audio_removal,
            early_homes,
            imports,
            runner,
            live: Arc::clone(live),
            app: app.clone(),
            models,
            store: capture.shared_store(),
            liveness,
            library: std::sync::Mutex::default(),
            library_status,
            mover: MoveWorker::default(),
            bulk_edits: std::sync::Mutex::default(),
            watched: std::sync::Mutex::default(),
        }
    }

    /// The passes that remove the audio of old notes.
    pub fn audio_removal(&self) -> &Arc<AudioRemovalManager> {
        &self.audio_removal
    }

    /// The imports of audio files. The import commands and the watched
    /// folder call [`ImportManager::import`].
    pub fn imports(&self) -> &Arc<ImportManager> {
        &self.imports
    }

    /// The early writes of the home folders of new notes (KTD4).
    pub fn early_homes(&self) -> &Arc<EarlyHomes> {
        &self.early_homes
    }

    /// The removal of empty notes (U6) with this run's store, claims,
    /// early write jobs and run ID, and the storage folder `destination`.
    fn removal<'a>(&'a self, destination: Option<&'a std::path::Path>) -> Removal<'a> {
        Removal {
            store: &self.store,
            claims: self.runner.claims(),
            homes: Some(&self.early_homes),
            destination,
            run_id: self.run_id(),
            earlier_runs: &self.own_runs[1..],
        }
    }

    /// The user left the note `id`: the main window shows another view or
    /// closes, or the user starts a new note (R18). The note and its home
    /// folder go when the note is empty and every check passes. Otherwise
    /// the note stays. This reads the storage folder: call it off the main
    /// thread.
    pub fn leave_note(&self, id: &str) -> Result<Leave, String> {
        self.leave_note_unless_shown(id, &|_| false)
    }

    /// [`ProcessingManager::leave_note`], and the note stays while
    /// `shown(id)` is true: the main window shows the note again.
    pub fn leave_note_unless_shown(
        &self,
        id: &str,
        shown: &dyn Fn(&str) -> bool,
    ) -> Result<Leave, String> {
        let destination = settings::get_settings(&self.app).storage_folder();
        let left = self
            .removal(destination.as_deref())
            .leave_unless_shown(id, chrono::Local::now().fixed_offset(), shown)
            .map_err(|e| e.to_string())?;
        if left == Leave::Removed {
            self.removed_empty_notes(vec![id.to_string()]);
        }
        Ok(left)
    }

    /// The launch pass of empty notes: each empty note that this computer
    /// started before `launched` (R18, KTD8). A note that stays because the
    /// storage folder is not available gets the next pass at the next
    /// launch.
    fn remove_empty_notes(&self, launched: chrono::DateTime<chrono::FixedOffset>) {
        let destination = settings::get_settings(&self.app).storage_folder();
        let passed = match self
            .removal(destination.as_deref())
            .launch_pass(launched, chrono::Local::now().fixed_offset())
        {
            Ok(passed) => passed,
            Err(e) => {
                log::error!("the launch pass of empty notes failed: {e}");
                return;
            }
        };
        let removed: Vec<String> = passed
            .into_iter()
            .filter(|(_, left)| *left == Leave::Removed)
            .map(|(id, _)| id)
            .collect();
        if !removed.is_empty() {
            log::info!("the launch pass removed {} empty note(s)", removed.len());
            self.removed_empty_notes(removed);
        }
    }

    /// Empty notes are gone: Home drops them, and the in-use markers
    /// refresh.
    fn removed_empty_notes(&self, removed: Vec<String>) {
        self.in_use_changed();
        emit_library_changed(
            &self.app,
            LibraryChanged {
                changed: Vec::new(),
                removed,
            },
        );
    }

    /// The run ID of this run, which names its in-use markers.
    pub fn run_id(&self) -> &str {
        self.in_use.run_id()
    }

    /// The runs whose in-use markers are this computer's own work: this run
    /// and the earlier runs on this computer (KTD5).
    pub fn own_runs(&self) -> &[String] {
        &self.own_runs
    }

    /// True while the summary of `note_id` is regenerated or waits to
    /// be. The regeneration continues when the user leaves the note.
    pub fn is_regenerating(&self, note_id: &str) -> bool {
        self.runner.is_regenerating(note_id)
    }

    /// What each note waits for now (KTD10). This locks the capture
    /// state, so call it before the store lock.
    pub fn waits(&self) -> pipeline::runner::Waits {
        self.runner.waits()
    }

    /// What the latest library scans found about each note.
    pub fn library_status(&self) -> &library::status::LibraryStatus {
        &self.library_status
    }

    /// Attach the live workers, then do the rest of the launch steps, in
    /// order, on one background thread, so the window opens at once with
    /// the stored rows (KTD9): the library scan and reconcile, the folder
    /// reconcile, the retry of notes that wait for a folder, the runner and
    /// the library watcher. A note that starts while these steps run gets
    /// its live transcript.
    pub fn start(self: &Arc<Self>) {
        let woken = Arc::downgrade(&self.runner);
        self.live
            .attach(self.runner.live_workers().with_idle(move || {
                if let Some(runner) = woken.upgrade() {
                    runner.wake();
                }
            }));
        // The first refresh marks the saved notes that an earlier run left
        // open (a Resume, the queue, a failure).
        if let Err(e) = self.in_use.start() {
            log::error!("could not start the in-use markers: {e}");
        }
        // A note that opens from now on can be on the page: the launch
        // pass of empty notes leaves it (U6).
        let launched = chrono::Local::now().fixed_offset();
        let manager = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("launch".into())
            .spawn(move || manager.launch(launched));
        if let Err(e) = spawned {
            log::error!("could not start the launch steps: {e}");
        }
    }

    fn launch(self: &Arc<Self>, launched: chrono::DateTime<chrono::FixedOffset>) {
        // Before the library scan: the shared settings of the storage folder
        // apply, and a folder with no settings file gets this computer's
        // shared settings (R21, KTD8).
        if let Some(shared) = self.app.try_state::<Arc<SharedSettingsManager>>()
            && shared.join_at_launch()
        {
            settings::emit_settings_changed(&self.app);
        }
        // The chosen recognition configuration can differ from the one in
        // use, for example after a change on another computer while this
        // one was off (R18).
        self.models.follow_choice(&self.app);
        let roots = self.roots();
        // Before the folder reconcile: a title that a sync delivered is in
        // the row first, so no folder is renamed back (KTD9).
        // The in-use markers of this run and of the earlier runs on this
        // computer are its own work: they do not show a note as in use on
        // another computer (KTD5).
        let mut reconciler = library::reconcile::Reconciler::default()
            .with_status(Arc::clone(&self.library_status))
            .with_own_runs(self.own_runs.iter().cloned());
        match self.runner.reconcile_library(&roots, &mut reconciler) {
            Ok(changes) => {
                log::info!(
                    "library: the launch scan changed {} note(s) and removed {}",
                    changes.changed.len(),
                    changes.removed.len()
                );
                // A sync can deliver a removal marker for a resumed note
                // that failed processing. The reconcile does not report
                // such a note as changed (KTD8).
                self.runner.drop_removed_old_audio_of_failed();
                if !changes.is_empty() {
                    emit_library_changed(&self.app, changes.into());
                }
            }
            Err(e) => log::error!("the launch scan of the library failed: {e}"),
        }
        // The notes of earlier folders, and of an old folder that Leave them
        // left while a note was busy, go out of the library (R7, R8).
        self.forget_outside_roots();
        if matches!(self.liveness, session_core::jobs::Liveness::OnlyInstance) {
            // Each saved folder takes the name its title gives (KTD8).
            if let Err(e) = self.runner.reconcile_folders() {
                log::error!("launch reconcile of folder names failed: {e}");
            }
            // The type moves, the type edits and the tag edits that a quit
            // or a crash stopped continue (KTD5, KTD7, KTD11).
            let changed = self.run_type_moves();
            self.emit_changed(changed);
        }
        // After the library scan, so the rows and the removed flags come
        // from the folders. The pass at launch runs only with the instance
        // lock held. The timer passes run in each case (KTD6).
        self.audio_removal.start(matches!(
            self.liveness,
            session_core::jobs::Liveness::OnlyInstance
        ));
        // The early writes that a quit or a crash stopped continue (R4).
        // Only with the instance lock held, so no other run writes the
        // same folders.
        if matches!(self.liveness, session_core::jobs::Liveness::OnlyInstance) {
            // First the empty notes that this computer started and did not
            // remove before it quit (R18, KTD8): a folder that the pass
            // removes is not written again.
            self.remove_empty_notes(launched);
            // A stop after a change of storage folder with Leave them can
            // come before the rehome of a note that has had no first save.
            self.rehome_outside();
            self.early_homes.resume();
            // The imports that a quit or a crash stopped continue (KTD6 of
            // the audio import plan). Only with the instance lock held, so
            // no other run uses the same journal.
            self.imports.start();
            // After the import worker: a file that arrived in the watched
            // folder while Longhand was closed is imported now (R5).
            self.start_watched_folder();
            // A note that the crash recovery made a note with no audio
            // again gets the flag "no recording" in its folder (R19).
            if let Some(capture) = self.app.try_state::<Arc<CaptureManager>>() {
                for id in capture.take_recovered_without_audio() {
                    self.early_homes.mark_pending(&id);
                }
            }
        }
        // The volume may be back, or the folder restored (U18).
        self.retry_awaiting();
        tauri::async_runtime::spawn(Arc::clone(&self.runner).run());
        // The folder watcher starts after the runner (KTD9), and keeps the
        // pending deletes of the launch reconcile.
        self.start_library(reconciler);
        // A Move that a quit or a crash stopped continues (R10). Only with
        // the instance lock held, so no other run moves the same notes.
        if matches!(self.liveness, session_core::jobs::Liveness::OnlyInstance) && self.move_open() {
            self.start_move();
        }
    }

    /// Whether a Move runs or is paused.
    pub fn move_open(&self) -> bool {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        matches!(store.move_journal(), Ok(Some(_)))
    }

    /// Open the Move journal from `old` to `new` (KTD9). Call before the
    /// storage folder changes, so the library keeps the old folder's notes
    /// meanwhile (KTD10). Refused while a Move runs or is paused (KTD11).
    pub fn open_move(&self, old: &std::path::Path, new: &std::path::Path) -> Result<(), String> {
        match pipeline::move_notes::open(&self.store, old, new) {
            Ok(true) => Ok(()),
            Ok(false) => Err(MOVE_OPEN.into()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Close a Move journal that [`ProcessingManager::open_move`] opened
    /// when the change of folder then failed. No note has moved.
    pub fn close_move(&self) {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = store.close_move() {
            log::error!("could not close the move journal: {e}");
        }
    }

    /// The open Move, or `None`.
    pub fn move_status(&self) -> Option<MoveStatus> {
        let status = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            pipeline::move_notes::status(&store)
        };
        let status = match status {
            Ok(status) => status?,
            Err(e) => {
                log::error!("could not read the move journal: {e}");
                return None;
            }
        };
        let last = self
            .mover
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last;
        let cause = status.journal.pause.as_ref().map(MovePauseCause::from);
        let state = if cause.is_some() {
            MoveState::Paused
        } else {
            match last {
                Some(MoveState::Waiting) => MoveState::Waiting,
                _ => MoveState::Running,
            }
        };
        Some(MoveStatus {
            old_folder: status.journal.old_folder.to_string_lossy().into_owned(),
            new_folder: status.journal.new_folder.to_string_lossy().into_owned(),
            moved: status.progress.moved as u32,
            total: status.progress.total as u32,
            state,
            cause,
        })
    }

    /// Continue a paused Move (R10, AE13).
    pub fn retry_move(self: &Arc<Self>) -> Result<(), String> {
        pipeline::move_notes::retry(&self.store).map_err(|e| e.to_string())?;
        self.start_move();
        Ok(())
    }

    /// Start the Move worker, or wake it when it waits.
    pub fn start_move(self: &Arc<Self>) {
        {
            let mut state = self.worker();
            if state.stopped {
                return;
            }
            if state.running {
                state.woken = true;
                self.mover.signal.notify_all();
                return;
            }
            state.running = true;
            state.woken = false;
        }
        // The library watches the old folder too while the Move runs.
        self.roots_changed();
        let manager = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("move".into())
            .spawn(move || manager.run_move());
        if let Err(e) = spawned {
            log::error!("could not start the move of the notes: {e}");
            self.worker().running = false;
        }
    }

    /// The Move worker. A new Move that opens as the last one ends runs
    /// too.
    fn run_move(&self) {
        loop {
            self.move_until_done();
            let mut state = self.worker();
            if state.woken && !state.stopped {
                state.woken = false;
                continue;
            }
            state.running = false;
            return;
        }
    }

    /// A pass, then a wait while a note is busy or the Move is paused,
    /// until every note is in the new folder or the app quits.
    fn move_until_done(&self) {
        let claims = self.runner.claims().clone();
        loop {
            let mover = pipeline::move_notes::Mover {
                store: &self.store,
                claims: &claims,
                fs: &session_save::RealFs,
                copy: &pipeline::move_notes::copy_note,
            };
            self.emit_move_progress(MoveState::Running);
            let mut last = self
                .move_status()
                .map(|s| (s.moved, s.total))
                .unwrap_or_default();
            let end = mover.pass(&mut |id, progress| {
                last = (progress.moved as u32, progress.total as u32);
                emit_library_changed(
                    &self.app,
                    library::reconcile::Changes {
                        changed: vec![id.to_string()],
                        removed: Vec::new(),
                        notices: false,
                    }
                    .into(),
                );
                emit_move(&self.app, MoveProgress::new(last, MoveState::Running));
            });
            let wait = match end {
                Ok(pipeline::move_notes::PassEnd::Finished) => {
                    self.set_move_state(MoveState::Finished);
                    emit_move(&self.app, MoveProgress::new(last, MoveState::Finished));
                    // The old folder is not a library root any more.
                    self.roots_changed();
                    self.forget_outside_roots();
                    break;
                }
                Ok(pipeline::move_notes::PassEnd::Waiting) => {
                    self.set_move_state(MoveState::Waiting);
                    self.emit_move_progress(MoveState::Waiting);
                    Some(MOVE_WAIT)
                }
                Ok(pipeline::move_notes::PassEnd::Paused(pause)) => {
                    self.set_move_state(MoveState::Paused);
                    self.emit_move_progress(MoveState::Paused);
                    let paused = MovePaused {
                        cause: MovePauseCause::from(&pause),
                    };
                    if let Err(e) = paused.emit(&self.app) {
                        log::warn!("could not emit the move pause: {e}");
                    }
                    None
                }
                Err(e) => {
                    log::error!("the move of the notes failed: {e}");
                    Some(MOVE_WAIT)
                }
            };
            if !self.wait_move(wait) {
                break;
            }
        }
    }

    /// Wait for `timeout`, or for Retry when it is `None`. Returns false
    /// when the app quits.
    fn wait_move(&self, timeout: Option<std::time::Duration>) -> bool {
        let mut state = self.worker();
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        while !state.woken && !state.stopped {
            match deadline {
                Some(deadline) => {
                    let now = std::time::Instant::now();
                    if now >= deadline {
                        break;
                    }
                    state = self
                        .mover
                        .signal
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
                None => {
                    state = self
                        .mover
                        .signal
                        .wait(state)
                        .unwrap_or_else(|e| e.into_inner());
                }
            }
        }
        state.woken = false;
        !state.stopped
    }

    fn set_move_state(&self, last: MoveState) {
        self.worker().last = Some(last);
    }

    fn worker(&self) -> std::sync::MutexGuard<'_, MoveWorkerState> {
        self.mover.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit_move_progress(&self, state: MoveState) {
        if let Some(status) = self.move_status() {
            emit_move(
                &self.app,
                MoveProgress::new((status.moved, status.total), state),
            );
        }
    }

    /// The folders that the library lists notes from.
    fn roots(&self) -> Vec<std::path::PathBuf> {
        library_roots(&self.app, &self.store)
    }

    /// The storage folder, whose `.longhand/` folder holds the shared
    /// settings (KTD13).
    fn storage_folder(&self) -> Option<std::path::PathBuf> {
        settings::get_settings(&self.app).storage_folder()
    }

    /// Forget the rows of the notes that are in no library root, and tell
    /// the surface. A busy note stays until a later call: each library
    /// reconcile makes this call again (R7).
    pub fn forget_outside_roots(&self) {
        let claims = self.runner.claims();
        let removed = forget_outside(&self.app, &self.store, &|id| claims.busy(id));
        if !removed.is_empty() {
            let changes = library::reconcile::Changes {
                changed: Vec::new(),
                removed,
                notices: false,
            };
            emit_library_changed(&self.app, changes.into());
        }
    }

    /// The store of this run's notes.
    pub fn shared_store(&self) -> &pipeline::SharedStore {
        &self.store
    }

    /// The first Save of `id` waits for its folder because of `problem`.
    /// When the early write of the note is not done, for example after the
    /// first Save gave the note a new early write in the storage folder, the
    /// job of that write starts. When the save waits for the early write and
    /// the write is done already, its job ended before the runner recorded
    /// the wait. The job then runs again: it writes nothing, and its end
    /// puts the waiting notes back in line ([`ProcessingManager::retry_awaiting`]).
    /// The runner records the wait before it sends this event, so that retry
    /// finds the note. The retry runs on the job thread, not in the run of
    /// the note that sends this event, so the run ends first.
    fn start_unfinished_early_write(&self, id: &str, problem: &str) {
        let unfinished = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            pipeline::early_home::unfinished(&store, id)
        };
        // Only for these causes: another cause can stay, and a retry for it
        // would repeat at once.
        let waits_for_write = problem == pipeline::save::WAITS_FOR_EARLY_WRITE
            || problem == pipeline::save::WAITS_FOR_NEW_EARLY_WRITE;
        match unfinished {
            Ok(unfinished) if unfinished || waits_for_write => self.early_homes.start(id),
            Ok(_) => {}
            Err(e) => log::error!("could not read the early write state of {id}: {e}"),
        }
    }

    /// True while the runner works on the note `id` now.
    pub fn runs(&self, id: &str) -> bool {
        self.runner.runs(id)
    }

    /// Each note that has had no first save and whose home folder is not in
    /// the storage folder gets a new early write in the storage folder, and
    /// its job starts. Its first Save then publishes there, and the library
    /// keeps it. The old home folder goes after the new write is done.
    /// While a Move is open, nothing changes. A note whose folder cannot be
    /// read now tries again at the next call.
    fn rehome_outside(&self) {
        let Some(current) = self.storage_folder() else {
            return;
        };
        match pipeline::early_home::rehome_outside(&self.store, &current) {
            Ok(ids) => {
                for id in ids {
                    self.early_homes.start(&id);
                }
            }
            Err(e) => log::error!(
                "could not give the notes that are not saved a home folder in the storage folder: {e}"
            ),
        }
    }

    fn start_library(&self, reconciler: library::reconcile::Reconciler) {
        let store = Arc::clone(&self.store);
        let claims = self.runner.claims().clone();
        let events = self.app.clone();
        let app = self.app.clone();
        let reader = self.app.clone();
        let audio_removal = Arc::clone(&self.audio_removal);
        let runner = Arc::clone(&self.runner);
        let started = LibraryWatcher::start(
            self.roots(),
            self.storage_folder(),
            reconciler,
            Box::new(move |target, reconciler| {
                let claimed = |id: &str| claims.busy(id);
                let mut changes = reconciler
                    .run(&store, target, &claimed)
                    .map_err(|e| e.to_string())?;
                // Each scan tries the type moves that wait (KTD6).
                if let Some(processing) = app.try_state::<Arc<ProcessingManager>>() {
                    changes.changed.extend(processing.run_type_moves());
                    changes.changed.sort();
                    changes.changed.dedup();
                }
                let removed = forget_outside(&app, &store, &claimed);
                if !removed.is_empty() {
                    changes.changed.retain(|id| !removed.contains(id));
                    changes.removed.extend(removed);
                    changes.removed.sort();
                    changes.removed.dedup();
                }
                // A sync can deliver a removal marker for a resumed note
                // that failed processing. The reconcile does not report
                // such a note as changed (KTD8).
                runner.drop_removed_old_audio_of_failed();
                Ok(changes)
            }),
            // On the watcher thread, never the main thread: a read can
            // wait for an online-only file (KTD13, R20).
            Box::new(move || {
                if let Some(shared) = reader.try_state::<Arc<SharedSettingsManager>>()
                    && shared.reread()
                {
                    log::info!("shared settings: another computer changed them");
                    settings::emit_settings_changed(&reader);
                    // Another computer can choose another recognition
                    // configuration: its downloads start (R18, AE5).
                    if let Some(models) = reader.try_state::<Arc<ModelsManager>>() {
                        models.follow_choice(&reader);
                    }
                    // The change can turn the audio removal on or shorten
                    // its delay. It needs no confirmation (KTD6).
                    audio_removal.run_now();
                }
            }),
            Box::new(move |changes| emit_library_changed(&events, changes)),
            Timing::default(),
        );
        let watcher = match started {
            Ok(watcher) => watcher,
            Err(e) => {
                log::error!("could not start the library watcher: {e}");
                return;
            }
        };
        let mut library = self.library.lock().unwrap_or_else(|e| e.into_inner());
        if library.stopped {
            // The quit came during the launch steps.
            drop(library);
            watcher.stop();
        } else {
            library.watcher = Some(watcher);
        }
    }

    /// The storage folder changed: the library watches start again on the
    /// new roots, and a full scan runs (KTD7).
    pub fn roots_changed(&self) {
        let library = self.library.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(watcher) = &library.watcher {
            watcher.set_roots(self.roots(), self.storage_folder());
        }
    }

    /// Start the watcher of the watched folder that the settings hold (R5).
    fn start_watched_folder(&self) {
        let env = match self.watched_environment() {
            Ok(env) => env,
            Err(e) => {
                log::error!("could not start the watched folder: {e}");
                return;
            }
        };
        let folder = settings::get_settings(&self.app)
            .watched_folder
            .map(std::path::PathBuf::from);
        let watcher = match WatchedFolder::start(folder, env, WatchedTiming::default()) {
            Ok(watcher) => watcher,
            Err(e) => {
                log::error!("could not start the watched folder: {e}");
                return;
            }
        };
        let mut slot = self.watched.lock().unwrap_or_else(|e| e.into_inner());
        if slot.stopped {
            // The quit came during the launch steps.
            drop(slot);
            watcher.stop();
        } else {
            slot.watcher = Some(watcher);
        }
    }

    /// What the watcher of the watched folder needs from the app.
    fn watched_environment(&self) -> Result<WatchedEnvironment, String> {
        let data = self.app.path().app_data_dir().map_err(|e| e.to_string())?;
        let computer = pipeline::lock(&self.store)
            .computer_id()
            .map_err(|e| e.to_string())?;
        let computer_id = import_record::ComputerId::parse(&computer)
            .ok_or_else(|| format!("{computer} is not a computer id"))?;
        let capture = self
            .app
            .try_state::<Arc<CaptureManager>>()
            .ok_or("the capture manager is not ready")?;
        let (storage, typed, events) = (self.app.clone(), self.app.clone(), self.app.clone());
        let (importer, sources, retry) = (
            Arc::clone(&self.imports),
            Arc::clone(&self.imports),
            Arc::clone(&self.imports),
        );
        Ok(WatchedEnvironment {
            storage_folder: Box::new(move || settings::get_settings(&storage).storage_folder()),
            computer_id,
            longhand_version: self.app.package_info().version.to_string(),
            // The default note type and no tags (R5).
            import: Box::new(move |path| {
                let note_type = crate::commands::capture::type_of_new_note(&typed, None)?;
                importer.import(pipeline::import::Request {
                    source: path.to_path_buf(),
                    origin: pipeline::import::Origin::WatchedFolder,
                    note_type,
                    tags: Vec::new(),
                    title: None,
                })
            }),
            open_sources: Box::new(move || sources.open_sources()),
            retry: Box::new(move || retry.retry()),
            suspended: Box::new(suspended(&capture, &self.live)),
            free_space: Box::new(recorder::storage::available_space),
            dataless: Box::new(watched_folder::is_dataless),
            hash: Arc::new(
                |path: &std::path::Path, read: &std::sync::atomic::AtomicU64| {
                    audio_import::hash_file_counted(path, read)
                },
            ),
            snapshot: data.join(watched_folder::SNAPSHOT_FILE),
            status: Box::new(move |status| {
                let changed = WatchedFolderChanged {
                    status: status.clone(),
                };
                if let Err(e) = changed.emit(&events) {
                    log::warn!("could not emit a watched folder change: {e}");
                }
            }),
        })
    }

    /// The watched folder setting changed: the watcher watches the new
    /// folder, or nothing. `skip` holds the files that the user chose not
    /// to import (R6). Before the watcher starts, the snapshot file gets
    /// them, and the start reads it.
    pub fn watched_folder_changed(&self, skip: Vec<watched_folder::Held>) {
        let folder = settings::get_settings(&self.app)
            .watched_folder
            .map(std::path::PathBuf::from);
        let slot = self.watched.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(watcher) = &slot.watcher {
            watcher.set_folder(folder, skip);
            return;
        }
        drop(slot);
        let Ok(data) = self.app.path().app_data_dir() else {
            return;
        };
        let file = data.join(watched_folder::SNAPSHOT_FILE);
        let written = match &folder {
            Some(folder) => watched_folder::write_snapshot(&file, folder, &skip),
            None => watched_folder::remove_snapshot(&file),
        };
        if let Err(e) = written {
            log::error!("could not write the watched folder snapshot: {e}");
        }
    }

    /// The state of the watched folder (R4, and the notice of U8).
    pub fn watched_folder_status(&self) -> WatchedFolderStatus {
        let slot = self.watched.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(watcher) = &slot.watcher {
            return watcher.status();
        }
        drop(slot);
        match settings::get_settings(&self.app).watched_folder {
            Some(path) => WatchedFolderStatus::Watching { path },
            None => WatchedFolderStatus::Off,
        }
    }

    /// Before the process exits: stop the library watcher and processing,
    /// and free the models. The live workers stop first: a worker holds the
    /// engine slot while it streams, and the runner frees the engine after
    /// it (KTD9).
    pub fn shutdown(&self) {
        let watcher = {
            let mut library = self.library.lock().unwrap_or_else(|e| e.into_inner());
            library.stopped = true;
            library.watcher.take()
        };
        if let Some(watcher) = watcher {
            watcher.stop();
        }
        {
            let mut state = self.worker();
            state.stopped = true;
            self.mover.signal.notify_all();
        }
        self.audio_removal.stop();
        // Before the import worker: the watcher starts no new import.
        let watched = {
            let mut slot = self.watched.lock().unwrap_or_else(|e| e.into_inner());
            slot.stopped = true;
            slot.watcher.take()
        };
        if let Some(watched) = watched {
            watched.stop();
        }
        self.imports.stop();
        self.early_homes.stop();
        self.live.shutdown();
        self.runner.shutdown();
        self.in_use.stop();
    }

    /// A session was queued or capture ended.
    pub fn wake(&self) {
        self.runner.wake();
    }

    /// A saved note started a Resume or entered the queue, or the work on
    /// a note ended: refresh the in-use markers now. This does not wait
    /// and takes no lock that a caller can hold.
    pub fn in_use_changed(&self) {
        self.in_use.wake();
    }

    /// Free the loaded engine when it holds `model`, before a delete of
    /// its file (KTD12).
    pub fn unload_engine(&self, model: &str) {
        if self.runner.unload_engine(model) {
            log::info!("unloaded model '{model}' before its delete");
        }
    }

    /// A resume took `note_id` back; its stage stops at the next checkpoint.
    pub fn cancel(&self, note_id: &str) {
        self.runner.cancel(note_id);
    }

    /// The destination changed: sessions waiting for one go back in line.
    pub fn retry_awaiting(&self) {
        // First the rule of a change of storage folder: a note whose home
        // folder is outside the storage folder waits for its new early
        // write, not for that folder.
        self.rehome_outside();
        // The early writes wait for the same storage folder (R5).
        self.early_homes.wake();
        match self.runner.retry_awaiting() {
            Ok(ids) if !ids.is_empty() => {
                log::info!("{} session(s) will try the new destination", ids.len())
            }
            Ok(_) => {}
            Err(e) => log::error!("could not retry sessions awaiting a destination: {e}"),
        }
    }

    pub fn retry(&self, note_id: &str, start_over: bool) -> Result<(), String> {
        self.runner
            .retry(note_id, start_over)
            .map_err(|e| e.to_string())?;
        self.in_use_changed();
        Ok(())
    }

    pub fn unblock(&self, note_id: &str) -> Result<(), String> {
        self.runner.unblock(note_id).map_err(|e| e.to_string())
    }

    /// True while the live worker drains or the runner runs a stage or a
    /// summary regeneration (KTD7 of the preview builds plan). A note that
    /// waits in the queue does not count. This locks neither the capture
    /// state nor the store.
    pub fn is_working(&self) -> bool {
        self.live.busy() || self.runner.is_working()
    }

    /// The claims that a delete takes: a summary regeneration and a delete
    /// exclude each other, and a folder rename waits for both (R8).
    pub fn claims(&self) -> session_core::jobs::Claims {
        self.runner.claims().clone()
    }

    /// Convert the notes in the old layout that the latest library scans
    /// found (U5 of the safe agent edits plan, R10, R12). A note that a
    /// computer works on now is skipped, and the next run converts it. Only
    /// the user starts this. It reads and writes note folders, so call it
    /// off the main thread.
    pub fn convert_old_notes(&self) -> pipeline::convert::ConvertReport {
        let notes = self.library_status.old_layout_notes();
        let claims = self.claims();
        let report = pipeline::convert::convert_notes(&self.store, &notes, &self.own_runs, &|id| {
            claims.busy(id)
        });
        self.library_status.forget_old_layout(&report.converted);
        emit_library_changed(
            &self.app,
            LibraryChanged {
                changed: report.converted.clone(),
                removed: Vec::new(),
            },
        );
        report
    }

    /// Rename a session (R9). The title changes at once; see
    /// [`Runner::rename_session`] for the folder. This can reach a synced
    /// folder, so call it off the main thread.
    pub fn rename_session(
        &self,
        note_id: &str,
        title: &str,
    ) -> Result<pipeline::folder_rename::FolderRename, String> {
        refuse_with_problem(&self.library_status, note_id)?;
        self.runner
            .rename_session(note_id, title)
            .map_err(|e| e.to_string())
    }

    /// Change the type of a note (R5). The app shows the new type at once;
    /// see [`Runner::change_note_type`] for the folder. The caller checks
    /// that `to` is a type of the effective list. This can reach a synced
    /// folder, so call it off the main thread.
    pub fn change_note_type(
        &self,
        note_id: &str,
        to: Option<&str>,
    ) -> Result<pipeline::type_move::TypeChange, String> {
        refuse_with_problem(&self.library_status, note_id)?;
        let changed = self
            .runner
            .change_note_type(note_id, to, &self.own_runs)
            .map_err(|e| e.to_string())?;
        emit_library_changed(
            &self.app,
            LibraryChanged {
                changed: vec![note_id.to_string()],
                removed: Vec::new(),
            },
        );
        Ok(changed)
    }

    /// Rename the note type `name` to `new_name`, or merge it into the type
    /// with that name (R6), and change each note of the type (KTD7).
    /// Returns the number of notes that changed.
    pub fn rename_note_type(
        &self,
        shared: &SharedSettingsManager,
        folders: &crate::managers::shared::StorageFolders,
        name: &str,
        new_name: &str,
    ) -> Result<u32, String> {
        self.edit_note_types(|before_save| {
            shared.rename_note_type(name, new_name, folders, before_save)
        })
    }

    /// Delete the note type `name` (R6): each note of the type gets General,
    /// and its folder goes to the storage folder (KTD7). Returns the number
    /// of notes that changed.
    pub fn delete_note_type(
        &self,
        shared: &SharedSettingsManager,
        folders: &crate::managers::shared::StorageFolders,
        name: &str,
    ) -> Result<u32, String> {
        self.edit_note_types(|before_save| {
            shared
                .delete_note_type(name, folders, before_save)
                .map(Some)
        })
    }

    /// A type rename, merge or delete (KTD7): record the edit with the
    /// notes that show the old type, change the shared list with `save`,
    /// then change each of these notes. The record goes when every note has
    /// moved or has its pending type. Otherwise each launch and scan
    /// continues the edit with the notes that are not done.
    fn edit_note_types(
        &self,
        save: impl FnOnce(
            &dyn Fn(&session_core::BulkEdit) -> Result<(), String>,
        ) -> Result<Option<session_core::BulkEdit>, String>,
    ) -> Result<u32, String> {
        let _edits = self.bulk_edits.lock().unwrap_or_else(|e| e.into_inner());
        let recorded: std::sync::Mutex<Option<(i64, Vec<String>)>> = std::sync::Mutex::default();
        let record = |edit: &session_core::BulkEdit| {
            let store = pipeline::lock(&self.store);
            let mut recorded = recorded.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((id, _)) = recorded.take() {
                store.remove_bulk_edit(id).map_err(|e| e.to_string())?;
            }
            let notes =
                pipeline::type_move::matching_notes(&store, edit).map_err(|e| e.to_string())?;
            let id = store
                .add_bulk_edit(edit, &notes)
                .map_err(|e| e.to_string())?;
            *recorded = Some((id, notes));
            Ok(())
        };
        let saved = save(&record);
        let id = recorded.into_inner().unwrap_or_else(|e| e.into_inner());
        let edit = match saved {
            Ok(Some(edit)) => edit,
            Ok(None) => return Ok(0),
            Err(e) => {
                if let Some((id, _)) = id {
                    self.remove_bulk_edit(id);
                }
                return Err(e);
            }
        };
        let Some((id, notes)) = id else {
            return Err("The change of the note types was not recorded.".into());
        };
        pipeline::lock(&self.store)
            .set_bulk_edit_shared_saved(id)
            .map_err(|e| e.to_string())?;
        settings::emit_settings_changed(&self.app);
        let changed = self.continue_type_edit(id, &edit, &notes);
        let count = u32::try_from(changed.len()).unwrap_or(u32::MAX);
        self.emit_changed(changed);
        Ok(count)
    }

    /// One pass of the per-note part of the type edit `id` over its
    /// recorded notes `notes` (KTD7). The record goes when the edit is
    /// done. Otherwise it keeps the notes that are not done. Returns the
    /// notes that changed.
    fn continue_type_edit(
        &self,
        id: i64,
        edit: &session_core::BulkEdit,
        notes: &[String],
    ) -> Vec<String> {
        let Some(root) = self.storage_folder() else {
            return Vec::new();
        };
        match self
            .runner
            .apply_type_edit(&root, id, edit, notes, &self.own_runs)
        {
            Ok(pass) => {
                self.save_bulk_pass(id, pass.done, &pass.left);
                pass.changed
            }
            Err(e) => {
                log::warn!("the type edit {id} continues later: {e}");
                Vec::new()
            }
        }
    }

    /// Add the tags `add` to a note and remove the tags `remove` (R10). This
    /// works during a recording and during processing. Returns the tags of
    /// the note. This can reach a synced folder, so call it off the main
    /// thread.
    pub fn set_note_tags(
        &self,
        note_id: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<Vec<String>, String> {
        refuse_with_problem(&self.library_status, note_id)?;
        let tags = self
            .runner
            .set_note_tags(note_id, add, remove)
            .map_err(|e| e.to_string())?;
        self.emit_changed(vec![note_id.to_string()]);
        Ok(tags)
    }

    /// Rename the tag `name` to `new_name` on every note (R13). A new name
    /// with the key of another tag merges the two tags. Returns the number
    /// of notes that have the tag (R31).
    pub fn rename_tag(&self, name: &str, new_name: &str) -> Result<u32, String> {
        let new_name = pipeline::tags::check_tag(new_name)?;
        let name = name.trim();
        if name == new_name {
            return Ok(0);
        }
        self.edit_tags(session_core::BulkEdit::RenameTag {
            from: name.to_string(),
            to: new_name,
        })
    }

    /// Delete the tag `name` from every note (R13). Returns the number of
    /// notes that had the tag (R31).
    pub fn delete_tag(&self, name: &str) -> Result<u32, String> {
        self.edit_tags(session_core::BulkEdit::DeleteTag {
            name: name.trim().to_string(),
        })
    }

    /// A tag rename or delete (KTD11): record the edit with the notes that
    /// have the tag, before the first note changes, then change each of
    /// these notes. The record goes when every note is done. Otherwise each
    /// launch and scan continues the edit with the notes that are not done.
    fn edit_tags(&self, edit: session_core::BulkEdit) -> Result<u32, String> {
        let _edits = self.bulk_edits.lock().unwrap_or_else(|e| e.into_inner());
        let (id, notes) = {
            let store = pipeline::lock(&self.store);
            let notes = pipeline::tags::matching_notes(&store, &edit).map_err(|e| e.to_string())?;
            let id = store
                .add_bulk_edit(&edit, &notes)
                .map_err(|e| e.to_string())?;
            // A tag edit has no shared-settings part.
            store
                .set_bulk_edit_shared_saved(id)
                .map_err(|e| e.to_string())?;
            (id, notes)
        };
        let pass = self.continue_tag_edit(id, &edit, &notes);
        self.emit_changed(pass.changed);
        Ok(u32::try_from(pass.matched).unwrap_or(u32::MAX))
    }

    /// One pass of the tag edit `id` over its recorded notes `notes`
    /// (KTD11). The record goes when the edit is done. Otherwise it keeps
    /// the notes that are not done.
    fn continue_tag_edit(
        &self,
        id: i64,
        edit: &session_core::BulkEdit,
        notes: &[String],
    ) -> pipeline::tags::TagPass {
        match self.runner.apply_tag_edit(edit, notes) {
            Ok(pass) => {
                self.save_bulk_pass(id, pass.done, &pass.left);
                pass
            }
            Err(e) => {
                log::warn!("the tag edit {id} continues later: {e}");
                pipeline::tags::TagPass::default()
            }
        }
    }

    /// After a pass of the bulk edit `id`: remove its record when the edit
    /// is `done`, else record that only the notes `left` are not done. A
    /// failure goes to the log.
    fn save_bulk_pass(&self, id: i64, done: bool, left: &[String]) {
        if done {
            self.remove_bulk_edit(id);
        } else if let Err(e) = pipeline::lock(&self.store).set_bulk_edit_notes(id, left) {
            log::error!("could not record the notes of the bulk edit {id}: {e}");
        }
    }

    /// Remove the record of the bulk edit `id`. A failure goes to the log.
    fn remove_bulk_edit(&self, id: i64) {
        if let Err(e) = pipeline::lock(&self.store).remove_bulk_edit(id) {
            log::error!("could not remove the bulk edit {id}: {e}");
        }
    }

    /// Continue the type and tag edits that are not done (KTD7, KTD11),
    /// then run the type moves that this computer owes and take over the
    /// moves that waited for 7 days (KTD5, KTD13). Returns the notes that
    /// changed. Only with the instance lock held, so no other run moves the
    /// same folders.
    pub fn run_type_moves(&self) -> Vec<String> {
        if !matches!(self.liveness, session_core::jobs::Liveness::OnlyInstance) {
            return Vec::new();
        }
        let mut changed = Vec::new();
        {
            let _edits = self.bulk_edits.lock().unwrap_or_else(|e| e.into_inner());
            let open = pipeline::lock(&self.store).bulk_edits();
            for open in open.unwrap_or_else(|e| {
                log::error!("could not read the bulk edits: {e}");
                Vec::new()
            }) {
                // A tag edit has no shared-settings part (KTD11).
                if open.edit.is_tag_edit() {
                    changed.extend(
                        self.continue_tag_edit(open.id, &open.edit, &open.notes)
                            .changed,
                    );
                    continue;
                }
                if !open.shared_saved {
                    let Some(shared) = self.app.try_state::<Arc<SharedSettingsManager>>() else {
                        continue;
                    };
                    if let Err(e) = shared.finish_type_edit(&open.edit) {
                        log::warn!("the type edit {} continues later: {e}", open.id);
                        continue;
                    }
                    settings::emit_settings_changed(&self.app);
                    let store = pipeline::lock(&self.store);
                    if let Err(e) = store.set_bulk_edit_shared_saved(open.id) {
                        log::error!("could not record the type edit {}: {e}", open.id);
                        continue;
                    }
                }
                changed.extend(self.continue_type_edit(open.id, &open.edit, &open.notes));
            }
        }
        match self.runner.run_type_moves(&self.own_runs) {
            Ok(moved) => changed.extend(moved),
            Err(e) => log::error!("could not run the type moves: {e}"),
        }
        changed.sort();
        changed.dedup();
        changed
    }

    /// Run the type moves on another thread, for example after the work on
    /// a note ended, and tell the surface what changed.
    fn run_type_moves_soon(self: &Arc<Self>) {
        let manager = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("type-moves".into())
            .spawn(move || {
                let changed = manager.run_type_moves();
                manager.emit_changed(changed);
            });
        if let Err(e) = spawned {
            log::error!("could not start the type moves: {e}");
        }
    }

    /// Tell the surface that the notes `changed` changed.
    fn emit_changed(&self, changed: Vec<String>) {
        if !changed.is_empty() {
            emit_library_changed(
                &self.app,
                LibraryChanged {
                    changed,
                    removed: Vec::new(),
                },
            );
        }
    }

    /// The names of the type folders that a type rename, merge or delete
    /// of this computer still empties (KTD7). The effective type list does
    /// not show them.
    pub fn leaving_types(&self) -> Vec<String> {
        let store = pipeline::lock(&self.store);
        store
            .bulk_edits()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|open| match open.edit {
                session_core::BulkEdit::RenameType { from, to }
                    if !session_save::naming::same_type_name(&from, &to) =>
                {
                    Some(from)
                }
                session_core::BulkEdit::RenameType { .. } => None,
                session_core::BulkEdit::DeleteType { name } => Some(name),
                session_core::BulkEdit::RenameTag { .. }
                | session_core::BulkEdit::DeleteTag { .. } => None,
            })
            .collect()
    }

    /// Check if Transcribe Again can start on a saved note now, with the
    /// recognition settings (KTD1, KTD2). Returns the transcription model
    /// of the run.
    pub fn check_transcribe_again(&self, note_id: &str) -> Result<String, Refusal> {
        self.refuse_while_syncing(note_id)?;
        let settings = settings::get_settings(&self.app).recognition_config();
        self.runner
            .check_transcribe_again(note_id, &settings, &|spec| self.readiness(spec))
            .map(|spec| spec.config.asr_model)
    }

    /// Queue a saved note to be transcribed again with the recognition
    /// settings (R15, KTD2). `model_id` is the model that the user
    /// confirmed.
    pub fn transcribe_again(&self, note_id: &str, model_id: &str) -> Result<(), Refusal> {
        self.refuse_while_syncing(note_id)?;
        let settings = settings::get_settings(&self.app).recognition_config();
        self.runner
            .transcribe_again(note_id, &settings, model_id, &|spec| self.readiness(spec))?;
        self.in_use_changed();
        Ok(())
    }

    /// [`refuse_while_syncing`] with the status of the latest scans, as a
    /// Transcribe Again refusal.
    fn refuse_while_syncing(&self, note_id: &str) -> Result<(), Refusal> {
        refuse_while_syncing(&self.library_status, note_id).map_err(|message| Refusal {
            message,
            in_settings: false,
        })
    }

    /// Whether the artifacts of `spec` are on disk, or downloading.
    fn readiness(&self, spec: &models::recognition::ExecutionSpec) -> Readiness {
        let missing = self.models.missing_spec(spec);
        if missing.is_empty() {
            return Readiness::Ready;
        }
        let statuses = self.models.statuses(&spec.config.asr_model);
        readiness_of(missing, |id| {
            statuses
                .iter()
                .any(|s| s.id == id && s.state == ModelState::Downloading)
        })
    }

    /// Summarize a saved session again, replacing its `summary.md` (AE17).
    pub async fn regenerate_summary(&self, note_id: &str) -> Result<String, String> {
        refuse_while_syncing(&self.library_status, note_id)?;
        self.runner
            .regenerate_summary(note_id)
            .await
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(|e| e.to_string())
    }
}

/// The text of an action that waits for the sync of the note (R7). The note
/// page shows the same text.
pub(crate) const WAIT_FOR_SYNC: &str = "Wait until the note finishes syncing.";

/// Refuse an action that reads or replaces the result of the note `id`
/// (Transcribe Again, Regenerate Summary, Resume) while its transcript or
/// summary waits for the sync (R7, KTD11).
pub(crate) fn refuse_while_syncing(
    status: &library::status::LibraryStatus,
    id: &str,
) -> Result<(), String> {
    let note = status.note(id);
    if let Some(problem) = &note.problem {
        return Err(problem_reason(problem));
    }
    if note.result_waits() {
        Err(WAIT_FOR_SYNC.to_string())
    } else {
        Ok(())
    }
}

/// Why the actions that write to a note wait while a file of its folder
/// gives no note (R6). The text tells the user what to do.
pub(crate) fn problem_reason(problem: &library::scan::FileProblem) -> String {
    use library::scan::ProblemKind;
    match problem.kind {
        ProblemKind::UserFile => match problem.line {
            Some(line) => format!(
                "{} has an error on line {line}. Correct the file to change this note.",
                problem.file
            ),
            None => format!(
                "{} has an error. Correct the file to change this note.",
                problem.file
            ),
        },
        ProblemKind::Record => format!(
            "This note waits for {} to sync. If it does not sync, update Longhand on the other computer.",
            problem.file
        ),
        ProblemKind::NewerRecord => {
            "A newer version of Longhand saved this note. Update Longhand to change it.".to_string()
        }
    }
}

/// Refuse a write to the note `id` while a file of its folder gives no
/// note (R6): a title, tag or type change, and the actions that read or
/// replace the result. Delete does not call this.
pub(crate) fn refuse_with_problem(
    status: &library::status::LibraryStatus,
    id: &str,
) -> Result<(), String> {
    match status.note(id).problem {
        Some(problem) => Err(problem_reason(&problem)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use summarize::Provider;

    /// Settings with `provider` active, the model `gpt-4o` and, when given,
    /// the address `address`.
    fn active(provider: Provider, address: Option<&str>) -> settings::Settings {
        let mut s = settings::Settings::default();
        s.choose_provider(Some(provider));
        s.set_provider_model(provider, "gpt-4o");
        if let Some(address) = address {
            s.set_provider_address(provider, address).unwrap();
        }
        s
    }

    /// The processing update that the app sends for `event`.
    fn processing_update(event: RunnerEvent) -> ProcessingUpdate {
        match Sent::from(event) {
            Sent::Processing(update) => update,
            Sent::Library(_) => panic!("a library change, not a processing update"),
        }
    }

    #[test]
    fn a_proposed_title_reaches_the_app_as_a_library_change_of_the_note() {
        // Home and the note page read the note again on a library change.
        let Sent::Library(changes) = Sent::from(RunnerEvent::TitleProposed {
            note_id: "s".into(),
            title: "Weekly sync".into(),
        }) else {
            panic!("a processing update, not a library change");
        };
        assert_eq!(
            changes,
            LibraryChanged {
                changed: vec!["s".into()],
                removed: Vec::new(),
            }
        );
    }

    #[test]
    fn speaker_progress_reaches_the_surface_with_its_attempt() {
        let update = processing_update(RunnerEvent::SpeakerProgress {
            note_id: "s".into(),
            progress: pipeline::progress::SpeakerProgress {
                attempt: "a1".into(),
                work: pipeline::progress::SpeakerWork {
                    percent: Some(42),
                    measuring: false,
                },
            },
        });
        assert_eq!(
            serde_json::to_value(&update).unwrap(),
            serde_json::json!({
                "note_id": "s",
                "update": {
                    "kind": "speaker_progress",
                    "attempt": "a1",
                    "percent": 42,
                    "measuring": false
                }
            })
        );
    }

    #[test]
    fn transcription_progress_keeps_its_window_counts() {
        let update = processing_update(RunnerEvent::Progress {
            note_id: "s".into(),
            stage: session_core::Stage::Transcribe,
            segment: 1,
            summary_step: None,
            done: 3,
            total: 8,
        });
        assert_eq!(
            serde_json::to_value(&update).unwrap()["update"],
            serde_json::json!({
                "kind": "progress",
                "stage": "transcribe",
                "segment": 1,
                "summary_step": null,
                "done": 3,
                "total": 8
            })
        );
    }

    #[test]
    fn the_staging_sweep_keeps_the_staging_of_an_id_with_no_row() {
        let root = tempfile::tempdir().unwrap();
        let store: pipeline::SharedStore = Arc::new(std::sync::Mutex::new(
            session_core::Store::open_in_memory().unwrap(),
        ));
        let staging = root
            .path()
            .join(session_save::naming::staging_name("unknown"));
        std::fs::create_dir_all(&staging).unwrap();
        let removed = session_save::sweep_staging(root.path(), |id| completed(&store, id)).unwrap();
        assert!(removed.is_empty());
        assert!(staging.is_dir());
    }

    #[test]
    fn the_staging_sweep_keeps_the_staging_of_an_early_write_that_is_not_done() {
        let root = tempfile::tempdir().unwrap();
        let store: pipeline::SharedStore = Arc::new(std::sync::Mutex::new(
            session_core::Store::open_in_memory().unwrap(),
        ));
        let opened = chrono::DateTime::parse_from_rfc3339("2026-09-30T09:00:00Z").unwrap();
        {
            let mut s = store.lock().unwrap();
            // A note that opened with no recording is Saved at once.
            s.create_notes_only("n", opened, None).unwrap();
            pipeline::early_home::begin(&s, "n").unwrap();
        }
        let staging = root.path().join(session_save::naming::staging_name("n"));
        std::fs::create_dir_all(&staging).unwrap();
        let removed = session_save::sweep_staging(root.path(), |id| completed(&store, id)).unwrap();
        assert!(removed.is_empty());
        assert!(staging.is_dir());
    }

    #[test]
    fn a_move_pause_names_its_cause_and_the_action_in_plain_words() {
        use session_core::MovePause;
        let full = MovePauseCause::from(&MovePause::DiskFull {
            folder: "/Volumes/Disk/Notes".into(),
        });
        assert_eq!(full.kind, MovePauseKind::DiskFull);
        assert!(full.message.contains("/Volumes/Disk/Notes") && full.message.contains("Retry"));
        let gone = MovePauseCause::from(&MovePause::NotReachable {
            folder: "/Volumes/Disk/Notes".into(),
        });
        assert_eq!(gone.kind, MovePauseKind::FolderNotAvailable);
        assert!(gone.message.contains("not available"), "{}", gone.message);
        let failed = MovePauseCause::from(&MovePause::Failed {
            message: "denied".into(),
        });
        assert_eq!(failed.kind, MovePauseKind::Failed);
        assert!(failed.message.contains("denied"));
    }

    #[test]
    fn an_artifact_that_is_not_downloading_is_missing_before_one_that_is() {
        let downloading = |id: &str| id == "a";
        assert_eq!(
            readiness_of(vec!["a".into(), "b".into()], downloading),
            Readiness::Missing {
                artifact: "b".into()
            }
        );
        assert_eq!(
            readiness_of(vec!["a".into()], downloading),
            Readiness::Downloading {
                artifact: "a".into()
            }
        );
        assert_eq!(readiness_of(vec![], downloading), Readiness::Ready);
    }

    #[test]
    fn with_no_provider_nothing_is_sent_and_the_note_has_no_summary() {
        // Covers AE2. The message is the summary problem that the
        // processing page shows.
        let mut read = false;
        let result = endpoint_from_settings(settings::Settings::default(), |_| {
            read = true;
            Ok(Some("sk-test".into()))
        });
        assert_eq!(
            result,
            Err(EndpointProblem::Unconfigured(NO_PROVIDER.into()))
        );
        assert!(NO_PROVIDER.contains("no AI provider is set"));
        assert!(!read, "no Keychain read with no provider");
    }

    #[test]
    fn a_cloud_provider_with_no_key_is_not_set_up() {
        let result = endpoint_from_settings(active(Provider::OpenAi, None), |_| Ok(None));
        assert_eq!(
            result,
            Err(EndpointProblem::Unconfigured(
                "Add an API key for api.openai.com in Settings".into()
            ))
        );
    }

    #[test]
    fn a_cloud_provider_with_a_key_uses_its_provider_address_and_key() {
        let mut read = None;
        let mut s = active(Provider::Anthropic, None);
        s.set_provider_context_limit(Provider::Anthropic, Some(90_000))
            .unwrap();
        let endpoint = endpoint_from_settings(s, |location| {
            read = Some(location.clone());
            Ok(Some("sk-test".into()))
        })
        .unwrap();
        assert_eq!(
            read,
            Some(settings::KeyLocation {
                provider: Provider::Anthropic,
                address: "https://api.anthropic.com".into(),
                legacy: false,
            })
        );
        assert_eq!(endpoint.provider, Provider::Anthropic);
        assert_eq!(endpoint.base_url, "https://api.anthropic.com");
        assert_eq!(endpoint.model, "gpt-4o");
        assert_eq!(endpoint.credential.as_deref(), Some("sk-test"));
        assert_eq!(endpoint.context_limit, Some(90_000));
    }

    #[test]
    fn a_chosen_local_provider_sends_no_key_and_reads_none() {
        let mut read = false;
        let endpoint = endpoint_from_settings(active(Provider::Ollama, None), |_| {
            read = true;
            Ok(Some("k".into()))
        })
        .unwrap();
        assert!(!read);
        assert_eq!(endpoint.provider, Provider::Ollama);
        assert_eq!(endpoint.base_url, "http://localhost:11434");
        assert_eq!(endpoint.credential, None);
    }

    #[test]
    fn custom_with_an_address_and_no_key_sends_no_key() {
        let endpoint = endpoint_from_settings(
            active(Provider::Custom, Some("http://studio.local:4000/v1")),
            |_| Ok(None),
        )
        .unwrap();
        assert_eq!(endpoint.provider, Provider::Custom);
        assert_eq!(endpoint.base_url, "http://studio.local:4000/v1");
        assert_eq!(endpoint.credential, None);
    }

    #[test]
    fn custom_with_no_address_or_no_model_is_not_configured() {
        let no_address =
            endpoint_from_settings(active(Provider::Custom, None), |_| Ok(Some("k".into())));
        assert!(matches!(no_address, Err(EndpointProblem::Unconfigured(_))));
        let mut no_model = active(Provider::OpenAi, None);
        no_model.set_provider_model(Provider::OpenAi, "");
        let result = endpoint_from_settings(no_model, |_| Ok(Some("k".into())));
        assert!(matches!(result, Err(EndpointProblem::Unconfigured(_))));
    }

    #[test]
    fn a_locked_keychain_blocks_and_another_failure_is_not_configured() {
        let locked = endpoint_from_settings(active(Provider::OpenAi, None), |_| {
            Err(SecretError::Locked {
                message: "locked".into(),
            })
        });
        assert_eq!(locked, Err(EndpointProblem::Locked("locked".into())));
        let failed = endpoint_from_settings(active(Provider::OpenAi, None), |_| {
            Err(SecretError::Platform {
                message: "broken".into(),
            })
        });
        assert_eq!(failed, Err(EndpointProblem::Unconfigured("broken".into())));
    }

    /// A saved note in a new storage folder, adopted by a scan at `now`.
    /// Returns the storage folder, the note folder, the store and the
    /// reconciler with its status.
    fn scanned_note(
        now: std::time::Instant,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        pipeline::SharedStore,
        library::reconcile::Reconciler,
    ) {
        use session_core::folder::{SessionDoc, fingerprint};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(library::scan::write_marker(&root));
        let home = root.join("2026-09-25 0900 Planning");
        std::fs::create_dir_all(home.join("audio")).unwrap();
        let start = chrono::DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap();
        let segment = session_core::Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: start,
            wall_end: start + chrono::TimeDelta::minutes(1),
            audio_file: "audio/segment-01.opus".into(),
            channels: 1,
            ends_in_pause: false,
        };
        let transcript = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
        let summary = "# Summary\n\nHello.\n";
        let mut doc = SessionDoc::new("a1", None, &[segment]).unwrap();
        doc.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
        doc.summary_fingerprint = Some(fingerprint(summary.as_bytes()));
        session_save::note_files::write_doc(&home, &doc).unwrap();
        std::fs::write(home.join("transcript.md"), transcript).unwrap();
        std::fs::write(home.join("summary.md"), summary).unwrap();
        std::fs::write(home.join("notes.md"), "My notes\n").unwrap();
        std::fs::write(home.join("audio/segment-01.opus"), b"opus").unwrap();
        let store: pipeline::SharedStore = Arc::new(std::sync::Mutex::new(
            session_core::Store::open_in_memory().unwrap(),
        ));
        let mut reconciler = library::reconcile::Reconciler::default();
        reconciler
            .run_at(
                &store,
                &library::reconcile::Target::Full(vec![root]),
                &|_| false,
                now,
            )
            .unwrap();
        (dir, home, store, reconciler)
    }

    // Covers R7, R8, KTD11: the actions wait while the result waits for the
    // sync, and run after the wait ends.
    #[test]
    fn an_action_on_a_note_that_waits_for_the_sync_is_refused_until_the_wait_ends() {
        use library::reconcile::{HAND_EDIT_WAIT, Target};
        let start = std::time::Instant::now();
        let (dir, home, store, mut reconciler) = scanned_note(start);
        let status = Arc::clone(reconciler.status());
        assert_eq!(refuse_while_syncing(&status, "a1"), Ok(()));

        std::fs::write(home.join("summary.md"), "# Summary\n\nEdited.\n").unwrap();
        let target = Target::Full(vec![dir.path().canonicalize().unwrap()]);
        reconciler
            .run_at(&store, &target, &|_| false, start)
            .unwrap();
        assert_eq!(
            refuse_while_syncing(&status, "a1"),
            Err("Wait until the note finishes syncing.".to_string())
        );

        reconciler
            .run_at(&store, &target, &|_| false, start + HAND_EDIT_WAIT)
            .unwrap();
        assert_eq!(refuse_while_syncing(&status, "a1"), Ok(()));
    }

    // Covers R6: a `session.md` that does not parse holds the writes at
    // once, with a reason that names the file and the line. A good parse
    // releases them.
    #[test]
    fn a_note_whose_session_md_does_not_parse_refuses_its_writes() {
        use library::reconcile::Target;
        let start = std::time::Instant::now();
        let (dir, home, store, mut reconciler) = scanned_note(start);
        let status = Arc::clone(reconciler.status());
        let target = Target::Full(vec![dir.path().canonicalize().unwrap()]);
        let good = std::fs::read_to_string(home.join("session.md")).unwrap();

        std::fs::write(
            home.join("session.md"),
            "---\ntitle: A\n  bad: indent\n---\n",
        )
        .unwrap();
        reconciler
            .run_at(&store, &target, &|_| false, start)
            .unwrap();
        let reason =
            "session.md has an error on line 3. Correct the file to change this note.".to_string();
        assert_eq!(refuse_with_problem(&status, "a1"), Err(reason.clone()));
        assert_eq!(refuse_while_syncing(&status, "a1"), Err(reason));

        std::fs::write(home.join("session.md"), good).unwrap();
        reconciler
            .run_at(&store, &target, &|_| false, start)
            .unwrap();
        assert_eq!(refuse_with_problem(&status, "a1"), Ok(()));
    }

    // Covers R6, KTD6: a `longhand.json` that is not valid JSON, or that a
    // newer Longhand wrote, holds the writes, each with its reason. A good
    // `longhand.json` releases them.
    #[test]
    fn a_note_whose_longhand_json_cannot_be_read_refuses_its_writes() {
        use library::reconcile::Target;
        use session_core::note_record::NOTE_RECORD_FILE;
        let start = std::time::Instant::now();
        let (dir, home, store, mut reconciler) = scanned_note(start);
        let status = Arc::clone(reconciler.status());
        let target = Target::Full(vec![dir.path().canonicalize().unwrap()]);
        let good = std::fs::read_to_string(home.join(NOTE_RECORD_FILE)).unwrap();
        let mut newer: serde_json::Value = serde_json::from_str(&good).unwrap();
        newer["layout"] = serde_json::json!(99);

        for (text, reason) in [
            (
                "{ not json".to_string(),
                "This note waits for longhand.json to sync. If it does not sync, update \
                 Longhand on the other computer.",
            ),
            (
                serde_json::to_string_pretty(&newer).unwrap(),
                "A newer version of Longhand saved this note. Update Longhand to change it.",
            ),
        ] {
            std::fs::write(home.join(NOTE_RECORD_FILE), &text).unwrap();
            reconciler
                .run_at(&store, &target, &|_| false, start)
                .unwrap();
            assert_eq!(
                refuse_with_problem(&status, "a1"),
                Err(reason.to_string()),
                "{text}"
            );
            assert_eq!(
                refuse_while_syncing(&status, "a1"),
                Err(reason.to_string()),
                "{text}"
            );

            std::fs::write(home.join(NOTE_RECORD_FILE), &good).unwrap();
            reconciler
                .run_at(&store, &target, &|_| false, start)
                .unwrap();
            assert_eq!(refuse_with_problem(&status, "a1"), Ok(()), "{text}");
        }
    }

    #[test]
    fn the_save_notice_names_a_note_and_says_a_summary_only_when_the_note_has_one() {
        let (title, body) = saved_notice("2026-09-25 0900 Planning", true);
        assert_eq!(title, "Note ready");
        assert_eq!(
            body,
            "2026-09-25 0900 Planning has its transcript and summary."
        );
        let (title, body) = saved_notice("2026-09-25 0900 Planning", false);
        assert_eq!(title, "Note ready");
        assert_eq!(
            body,
            "2026-09-25 0900 Planning has its transcript. The summary could not be made."
        );
        for has_summary in [true, false] {
            let (title, body) = saved_notice("x", has_summary);
            assert!(!title.to_lowercase().contains("session"), "{title}");
            assert!(!body.to_lowercase().contains("session"), "{body}");
        }
    }

    #[test]
    fn a_note_has_a_summary_only_when_the_summarize_stage_completed_without_a_failure() {
        use session_core::Stage;
        let store: pipeline::SharedStore = Arc::new(std::sync::Mutex::new(
            session_core::Store::open_in_memory().unwrap(),
        ));
        let start = chrono::DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap();
        let segment = |index: u32| session_core::Segment {
            index,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: start,
            wall_end: start + chrono::TimeDelta::minutes(1),
            audio_file: format!("audio/segment-0{}.opus", index + 1),
            channels: 1,
            ends_in_pause: false,
        };
        {
            let mut s = store.lock().unwrap();
            for id in ["n", "bad"] {
                s.create_session(id, start, None).unwrap();
                s.put_segment(id, &segment(0)).unwrap();
                s.put_segment(id, &segment(1)).unwrap();
            }
            // A stage record that does not parse gives a read error.
            s.put_checkpoint("bad", Stage::Summarize, pipeline::checkpoint::DONE, "{")
                .unwrap();
        }
        // The Summarize stage did not run.
        assert!(!summary_made(&store, "n"));
        // The stage is complete for one of the two segments only.
        pipeline::checkpoint::mark_complete(&store, "n", Stage::Summarize, 1).unwrap();
        assert!(!summary_made(&store, "n"));
        pipeline::checkpoint::mark_complete(&store, "n", Stage::Summarize, 2).unwrap();
        assert!(summary_made(&store, "n"));
        pipeline::save::record_summary_failure(&store, "n", "no summary service").unwrap();
        assert!(!summary_made(&store, "n"));
        assert!(!summary_made(&store, "bad"));
    }

    #[test]
    fn a_failed_transcribe_again_run_ends_the_work_on_the_note() {
        use session_core::Stage;
        let id = || "s".to_string();
        for event in [
            RunnerEvent::Saved {
                note_id: id(),
                path: "/notes/a".into(),
                copied: false,
            },
            RunnerEvent::StageFailed {
                note_id: id(),
                stage: Stage::Transcribe,
                message: "failed".into(),
            },
            RunnerEvent::Cancelled { note_id: id() },
            RunnerEvent::TranscribeAgainFailed {
                note_id: id(),
                stage: Stage::Transcribe,
                message: "failed".into(),
            },
        ] {
            assert!(ends_work(&event), "{event:?}");
        }
        for event in [
            RunnerEvent::Progress {
                note_id: id(),
                stage: Stage::Transcribe,
                segment: 0,
                summary_step: None,
                done: 1,
                total: 2,
            },
            RunnerEvent::Finished { note_id: id() },
            RunnerEvent::SummaryUnavailable {
                note_id: id(),
                message: "no summary service".into(),
            },
        ] {
            assert!(!ends_work(&event), "{event:?}");
        }
    }
}
