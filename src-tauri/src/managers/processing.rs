//! Wires the pipeline runner (U8) into the app: models from the models
//! directory, suspension while capture is active (KTD20), and runner events
//! out to the surface.

use std::sync::Arc;

use asr::{Engine, Recognizer};
use diarize::{DiarizeError, OnnxAnalyzer, SpeakerAnalyzer};
use models::ModelKind;
use pipeline::early_home::EarlyHomes;
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
use crate::managers::library::{LibraryChanged, LibraryWatcher, Timing};
use crate::managers::live::LiveManager;
use crate::managers::models::{ModelState, ModelsManager};
use crate::managers::settings;
use crate::managers::shared::SharedSettingsManager;

/// What processing is doing, for the surface.
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct ProcessingUpdate {
    pub session_id: String,
    pub update: ProcessingStep,
}

#[derive(Serialize, Clone, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessingStep {
    StageStarted {
        stage: String,
    },
    Progress {
        stage: String,
        segment: u32,
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
        let (session_id, update) = match event {
            RunnerEvent::TitleProposed { session_id, .. } => {
                return Self::Library(LibraryChanged {
                    changed: vec![session_id],
                    removed: Vec::new(),
                });
            }
            RunnerEvent::StageStarted { session_id, stage } => (
                session_id,
                ProcessingStep::StageStarted {
                    stage: stage_name(stage),
                },
            ),
            RunnerEvent::Progress {
                session_id,
                stage,
                segment,
                done,
                total,
            } => (
                session_id,
                ProcessingStep::Progress {
                    stage: stage_name(stage),
                    segment,
                    done: done as u32,
                    total: total as u32,
                },
            ),
            RunnerEvent::SpeakerProgress {
                session_id,
                progress,
            } => (session_id, ProcessingStep::SpeakerProgress(progress.into())),
            RunnerEvent::StageFailed {
                session_id,
                stage,
                message,
            } => (
                session_id,
                ProcessingStep::StageFailed {
                    stage: stage_name(stage),
                    message,
                },
            ),
            RunnerEvent::SpeakersUnavailable {
                session_id,
                message,
            } => (session_id, ProcessingStep::SpeakersUnavailable { message }),
            RunnerEvent::Blocked {
                session_id,
                message,
            } => (session_id, ProcessingStep::Blocked { message }),
            RunnerEvent::Waiting { session_id, what } => {
                (session_id, ProcessingStep::Waiting { what })
            }
            RunnerEvent::SummaryUnavailable {
                session_id,
                message,
            } => (session_id, ProcessingStep::SummaryUnavailable { message }),
            RunnerEvent::AwaitingDestination {
                session_id,
                problem,
            } => (session_id, ProcessingStep::AwaitingDestination { problem }),
            RunnerEvent::Saved {
                session_id,
                path,
                copied,
            } => (
                session_id,
                ProcessingStep::Saved {
                    path: path.to_string_lossy().into_owned(),
                    copied,
                },
            ),
            RunnerEvent::TranscribeAgainFailed {
                session_id,
                stage,
                message,
            } => (
                session_id,
                ProcessingStep::TranscribeAgainFailed {
                    stage: stage_name(stage),
                    message,
                },
            ),
            RunnerEvent::Cancelled { session_id } => (session_id, ProcessingStep::Cancelled),
            RunnerEvent::Finished { session_id } => (session_id, ProcessingStep::Finished),
        };
        Self::Processing(ProcessingUpdate { session_id, update })
    }
}

/// The summary endpoint from `s`, with the key that `read_key` reads for the
/// effective Base URL. The request gate (KTD3) applies: the built-in default
/// Base URL with no saved key gives `Unconfigured`, and no request goes out.
fn endpoint_from_settings(
    s: settings::Settings,
    read_key: impl FnOnce(&str) -> Result<Option<String>, SecretError>,
) -> Result<SummaryEndpoint, EndpointProblem> {
    use settings::{EndpointGate, NotRequestedReason};
    const NO_ENDPOINT: &str = "no summarization endpoint is set; choose one in Settings";
    if !s.endpoint_configured() {
        return Err(EndpointProblem::Unconfigured(NO_ENDPOINT.into()));
    }
    let base_url = s.effective_endpoint_base_url().to_string();
    // The credential is keyed by the configured host: a changed base URL
    // has none until one is entered for it (R30a).
    let read = read_key(&base_url);
    let key = settings::key_state(&read);
    match settings::endpoint_gate(&base_url, s.endpoint_base_url_saved(), key) {
        EndpointGate::NotRequested(NotRequestedReason::KeyNeeded) => {
            let host = secrets::endpoint_key(&base_url).unwrap_or_else(|_| base_url.clone());
            return Err(EndpointProblem::Unconfigured(format!(
                "Add an API key for {host}, or enter a Base URL"
            )));
        }
        EndpointGate::NotRequested(NotRequestedReason::NoBaseUrl) => {
            return Err(EndpointProblem::Unconfigured(NO_ENDPOINT.into()));
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
        learned_limit: s
            .learned_context_limits
            .get(&settings::learned_limit_key(&base_url, &s.endpoint_model))
            .copied(),
        base_url,
        model: s.endpoint_model,
        credential,
        context_limit: s.context_limit_tokens,
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
                && runtime.revision == models::recognition::TRANSCRIBE_REVISION
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
        endpoint_from_settings(settings::get_settings(&self.app), |base_url| {
            self.secrets.endpoint_credential(base_url)
        })
    }

    fn summary_prompt(&self) -> summarize::Prompt {
        settings::get_settings(&self.app).active_summary_prompt_snapshot()
    }

    fn learn_limit(&self, base_url: &str, model: &str, limit: u32) {
        let key = settings::learned_limit_key(base_url, model);
        if let Err(e) = settings::update_settings(&self.app, |s| {
            s.learned_context_limits.insert(key, limit);
            Ok(())
        }) {
            log::warn!("could not remember the context limit for {model}: {e}");
        }
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

fn notify_saved(app: &AppHandle, path: &std::path::Path) {
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
    if let Err(e) = app
        .notification()
        .builder()
        .title("Session ready")
        .body(format!("{folder} has its transcript and summary."))
        .show()
    {
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
    /// The passes that remove the audio of old notes (U6 of the audio
    /// removal plan).
    audio_removal: Arc<AudioRemovalManager>,
    /// The early writes of the home folders of new notes (KTD4).
    early_homes: Arc<EarlyHomes>,
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
        let suspended = Arc::clone(capture);
        // Processing stays suspended after Stop until the live worker has
        // saved its last part and released the engine (KTD9).
        let draining = Arc::clone(live);
        let events = app.clone();
        let runner = Runner::new(Environment {
            store: capture.shared_store(),
            working_root: capture.working_root().to_path_buf(),
            engines: Box::new(Models {
                app: app.clone(),
                models: Arc::clone(&models),
                secrets,
            }),
            suspended: Box::new(move || suspended.is_capturing() || draining.busy()),
            events: Box::new(move |event| {
                if let RunnerEvent::Saved { path, .. } = &event {
                    notify_saved(&events, path);
                }
                // The work on a note can end: its in-use marker goes at
                // Saved, and a failed Transcribe Again run is Saved again.
                if matches!(
                    event,
                    RunnerEvent::Saved { .. }
                        | RunnerEvent::StageFailed { .. }
                        | RunnerEvent::Cancelled { .. }
                ) && let Some(processing) = events.try_state::<Arc<ProcessingManager>>()
                {
                    processing.in_use_changed();
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
                    if let Err(e) = processing.runner.retry_awaiting() {
                        log::error!("could not retry sessions awaiting a destination: {e}");
                    }
                }
            }),
        );
        let library_status: Arc<library::status::LibraryStatus> = Arc::default();
        let audio_removal = AudioRemovalManager::new(
            app,
            capture.shared_store(),
            runner.claims().clone(),
            Arc::clone(&library_status),
        );
        Self {
            in_use,
            audio_removal,
            early_homes,
            runner,
            live: Arc::clone(live),
            app: app.clone(),
            models,
            store: capture.shared_store(),
            liveness,
            library: std::sync::Mutex::default(),
            library_status,
            mover: MoveWorker::default(),
        }
    }

    /// The passes that remove the audio of old notes.
    pub fn audio_removal(&self) -> &Arc<AudioRemovalManager> {
        &self.audio_removal
    }

    /// The early writes of the home folders of new notes (KTD4).
    pub fn early_homes(&self) -> &Arc<EarlyHomes> {
        &self.early_homes
    }

    /// What the latest library scans found about each note.
    pub fn library_status(&self) -> &library::status::LibraryStatus {
        &self.library_status
    }

    /// The rest of the launch steps, in order, on one background thread,
    /// so the window opens at once with the stored rows (KTD9): the library
    /// scan and reconcile, the folder reconcile, the live workers, the
    /// retry of notes that wait for a folder, the runner and the library
    /// watcher.
    pub fn start(self: &Arc<Self>) {
        // The first refresh marks the saved notes that an earlier run left
        // open (a Resume, the queue, a failure).
        if let Err(e) = self.in_use.start() {
            log::error!("could not start the in-use markers: {e}");
        }
        let manager = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("launch".into())
            .spawn(move || manager.launch());
        if let Err(e) = spawned {
            log::error!("could not start the launch steps: {e}");
        }
    }

    fn launch(self: &Arc<Self>) {
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
        // The in-use markers of this run are its own work: they do not
        // show a note as in use on another computer (KTD5).
        let mut reconciler = library::reconcile::Reconciler::default()
            .with_status(Arc::clone(&self.library_status))
            .with_own_run(self.runner.owner());
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
        }
        // After the library scan, so the rows and the removed flags come
        // from the folders. The pass at launch runs only with the instance
        // lock held. The timer passes run in each case (KTD6).
        self.audio_removal.start(matches!(
            self.liveness,
            session_core::jobs::Liveness::OnlyInstance
        ));
        let woken = Arc::downgrade(&self.runner);
        self.live
            .attach(self.runner.live_workers().with_idle(move || {
                if let Some(runner) = woken.upgrade() {
                    runner.wake();
                }
            }));
        // The early writes that a quit or a crash stopped continue (R4).
        // Only with the instance lock held, so no other run writes the
        // same folders.
        if matches!(self.liveness, session_core::jobs::Liveness::OnlyInstance) {
            self.early_homes.resume();
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
            };
            emit_library_changed(&self.app, changes.into());
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

    /// A resume took `session_id` back; its stage stops at the next checkpoint.
    pub fn cancel(&self, session_id: &str) {
        self.runner.cancel(session_id);
    }

    /// The destination changed: sessions waiting for one go back in line.
    pub fn retry_awaiting(&self) {
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

    pub fn retry(&self, session_id: &str) -> Result<(), String> {
        self.runner.retry(session_id).map_err(|e| e.to_string())?;
        self.in_use_changed();
        Ok(())
    }

    pub fn unblock(&self, session_id: &str) -> Result<(), String> {
        self.runner.unblock(session_id).map_err(|e| e.to_string())
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

    /// Rename a session (R9). The title changes at once; see
    /// [`Runner::rename_session`] for the folder. This can reach a synced
    /// folder, so call it off the main thread.
    pub fn rename_session(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<pipeline::folder_rename::FolderRename, String> {
        self.runner
            .rename_session(session_id, title)
            .map_err(|e| e.to_string())
    }

    /// Check if Transcribe Again can start on a saved note now, with the
    /// recognition settings (KTD1, KTD2). Returns the transcription model
    /// of the run.
    pub fn check_transcribe_again(&self, session_id: &str) -> Result<String, Refusal> {
        self.refuse_while_syncing(session_id)?;
        let settings = settings::get_settings(&self.app).recognition_config();
        self.runner
            .check_transcribe_again(session_id, &settings, &|spec| self.readiness(spec))
            .map(|spec| spec.config.asr_model)
    }

    /// Queue a saved note to be transcribed again with the recognition
    /// settings (R15, KTD2). `model_id` is the model that the user
    /// confirmed.
    pub fn transcribe_again(&self, session_id: &str, model_id: &str) -> Result<(), Refusal> {
        self.refuse_while_syncing(session_id)?;
        let settings = settings::get_settings(&self.app).recognition_config();
        self.runner
            .transcribe_again(session_id, &settings, model_id, &|spec| {
                self.readiness(spec)
            })?;
        self.in_use_changed();
        Ok(())
    }

    /// [`refuse_while_syncing`] with the status of the latest scans, as a
    /// Transcribe Again refusal.
    fn refuse_while_syncing(&self, session_id: &str) -> Result<(), Refusal> {
        refuse_while_syncing(&self.library_status, session_id).map_err(|message| Refusal {
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
    pub async fn regenerate_summary(&self, session_id: &str) -> Result<String, String> {
        refuse_while_syncing(&self.library_status, session_id)?;
        self.runner
            .regenerate_summary(session_id)
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
    if status.note(id).result_waits() {
        Err(WAIT_FOR_SYNC.to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_model(base_url: Option<&str>) -> settings::Settings {
        settings::Settings {
            endpoint_base_url: base_url.map(str::to_string),
            endpoint_model: "gpt-4o".into(),
            ..Default::default()
        }
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
            session_id: "s".into(),
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
            session_id: "s".into(),
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
                "session_id": "s",
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
            session_id: "s".into(),
            stage: session_core::Stage::Transcribe,
            segment: 1,
            done: 3,
            total: 8,
        });
        assert_eq!(
            serde_json::to_value(&update).unwrap()["update"],
            serde_json::json!({
                "kind": "progress",
                "stage": "transcribe",
                "segment": 1,
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
    fn the_default_endpoint_with_no_key_is_not_configured() {
        let result = endpoint_from_settings(with_model(None), |_| Ok(None));
        assert_eq!(
            result,
            Err(EndpointProblem::Unconfigured(
                "Add an API key for api.openai.com, or enter a Base URL".into()
            ))
        );
    }

    #[test]
    fn the_default_endpoint_with_a_key_uses_that_key() {
        let mut read = None;
        let endpoint = endpoint_from_settings(with_model(None), |url| {
            read = Some(url.to_string());
            Ok(Some("sk-test".into()))
        })
        .unwrap();
        assert_eq!(read.as_deref(), Some("https://api.openai.com/v1"));
        assert_eq!(endpoint.base_url, "https://api.openai.com/v1");
        assert_eq!(endpoint.model, "gpt-4o");
        assert_eq!(endpoint.credential.as_deref(), Some("sk-test"));
    }

    #[test]
    fn a_saved_base_url_with_no_key_sends_no_key() {
        let endpoint =
            endpoint_from_settings(with_model(Some("http://localhost:11434/v1")), |_| Ok(None))
                .unwrap();
        assert_eq!(endpoint.base_url, "http://localhost:11434/v1");
        assert_eq!(endpoint.credential, None);
    }

    #[test]
    fn a_learned_limit_is_read_for_the_effective_base_url() {
        let mut s = with_model(None);
        s.learned_context_limits.insert(
            settings::learned_limit_key("https://api.openai.com/v1", "gpt-4o"),
            8000,
        );
        let endpoint = endpoint_from_settings(s, |_| Ok(Some("k".into()))).unwrap();
        assert_eq!(endpoint.learned_limit, Some(8000));
    }

    #[test]
    fn a_cleared_base_url_or_no_model_is_not_configured() {
        let cleared = endpoint_from_settings(with_model(Some("")), |_| Ok(Some("k".into())));
        assert!(matches!(cleared, Err(EndpointProblem::Unconfigured(_))));
        let mut no_model = with_model(None);
        no_model.endpoint_model.clear();
        let result = endpoint_from_settings(no_model, |_| Ok(Some("k".into())));
        assert!(matches!(result, Err(EndpointProblem::Unconfigured(_))));
    }

    #[test]
    fn a_locked_keychain_blocks_and_another_failure_is_not_configured() {
        let locked = endpoint_from_settings(with_model(None), |_| {
            Err(SecretError::Locked {
                message: "locked".into(),
            })
        });
        assert_eq!(locked, Err(EndpointProblem::Locked("locked".into())));
        let failed = endpoint_from_settings(with_model(None), |_| {
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
        };
        let transcript = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
        let summary = "# Summary\n\nHello.\n";
        let mut doc = SessionDoc::new("A1", None, &[segment]).unwrap();
        doc.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
        doc.summary_fingerprint = Some(fingerprint(summary.as_bytes()));
        std::fs::write(home.join("session.md"), doc.render()).unwrap();
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
        assert_eq!(refuse_while_syncing(&status, "A1"), Ok(()));

        std::fs::write(home.join("summary.md"), "# Summary\n\nEdited.\n").unwrap();
        let target = Target::Full(vec![dir.path().canonicalize().unwrap()]);
        reconciler
            .run_at(&store, &target, &|_| false, start)
            .unwrap();
        assert_eq!(
            refuse_while_syncing(&status, "A1"),
            Err("Wait until the note finishes syncing.".to_string())
        );

        reconciler
            .run_at(&store, &target, &|_| false, start + HAND_EDIT_WAIT)
            .unwrap();
        assert_eq!(refuse_while_syncing(&status, "A1"), Ok(()));
    }
}
