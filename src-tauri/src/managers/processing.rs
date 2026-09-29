//! Wires the pipeline runner (U8) into the app: models from the models
//! directory, suspension while capture is active (KTD20), and runner events
//! out to the surface.

use std::sync::Arc;

use asr::{Engine, Recognizer};
use diarize::{DiarizeError, OnnxAnalyzer, SpeakerAnalyzer};
use models::ModelKind;
use pipeline::reprocess::{Readiness, Refusal};
use pipeline::{
    EndpointProblem, EngineProvider, Environment, Runner, RunnerEvent, SummaryEndpoint,
};
use secrets::{SecretError, SecretStore};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;

use crate::managers::capture::CaptureManager;
use crate::managers::library::{LibraryChanged, LibraryWatcher, Timing};
use crate::managers::live::LiveManager;
use crate::managers::models::{ModelState, ModelsManager};
use crate::managers::settings;

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

impl From<RunnerEvent> for ProcessingUpdate {
    fn from(event: RunnerEvent) -> Self {
        let (session_id, update) = match event {
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
        Self { session_id, update }
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
/// completes, and an id with no row keeps its staging (KTD9).
fn completed(store: &pipeline::SharedStore, id: &str) -> bool {
    store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .session(id)
        .ok()
        .flatten()
        .is_some_and(|r| r.state == session_core::SessionState::Saved)
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
                if let Err(e) = ProcessingUpdate::from(event).emit(&events) {
                    log::warn!("could not emit a processing update: {e}");
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
        Self {
            runner,
            live: Arc::clone(live),
            app: app.clone(),
            models,
            store: capture.shared_store(),
            liveness,
            library: std::sync::Mutex::default(),
            library_status: Arc::default(),
            mover: MoveWorker::default(),
        }
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
        if let Some(shared) = self
            .app
            .try_state::<Arc<crate::managers::shared::SharedSettingsManager>>()
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
        let mut reconciler =
            library::reconcile::Reconciler::default().with_status(Arc::clone(&self.library_status));
        match self.runner.reconcile_library(&roots, &mut reconciler) {
            Ok(changes) => {
                log::info!(
                    "library: the launch scan changed {} note(s) and removed {}",
                    changes.changed.len(),
                    changes.removed.len()
                );
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
        let woken = Arc::downgrade(&self.runner);
        self.live
            .attach(self.runner.live_workers().with_idle(move || {
                if let Some(runner) = woken.upgrade() {
                    runner.wake();
                }
            }));
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
            let mut state = self.mover.state.lock().unwrap_or_else(|e| e.into_inner());
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
            self.mover
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .running = false;
        }
    }

    /// The Move worker. A new Move that opens as the last one ends runs
    /// too.
    fn run_move(&self) {
        loop {
            self.move_until_done();
            let mut state = self.mover.state.lock().unwrap_or_else(|e| e.into_inner());
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
                emit_move(
                    &self.app,
                    MoveProgress {
                        moved: progress.moved as u32,
                        total: progress.total as u32,
                        state: MoveState::Running,
                    },
                );
            });
            let wait = match end {
                Ok(pipeline::move_notes::PassEnd::Finished) => {
                    self.set_move_state(MoveState::Finished);
                    emit_move(
                        &self.app,
                        MoveProgress {
                            moved: last.0,
                            total: last.1,
                            state: MoveState::Finished,
                        },
                    );
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
        let mut state = self.mover.state.lock().unwrap_or_else(|e| e.into_inner());
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
        self.mover
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last = Some(last);
    }

    fn emit_move_progress(&self, state: MoveState) {
        if let Some(status) = self.move_status() {
            emit_move(
                &self.app,
                MoveProgress {
                    moved: status.moved,
                    total: status.total,
                    state,
                },
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
                Ok(changes)
            }),
            // On the watcher thread, never the main thread: a read can
            // wait for an online-only file (KTD13, R20).
            Box::new(move || {
                if let Some(shared) =
                    reader.try_state::<Arc<crate::managers::shared::SharedSettingsManager>>()
                    && shared.reread()
                {
                    log::info!("shared settings: another computer changed them");
                    settings::emit_settings_changed(&reader);
                    // Another computer can choose another recognition
                    // configuration: its downloads start (R18, AE5).
                    if let Some(models) = reader.try_state::<Arc<ModelsManager>>() {
                        models.follow_choice(&reader);
                    }
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
            let mut state = self.mover.state.lock().unwrap_or_else(|e| e.into_inner());
            state.stopped = true;
            self.mover.signal.notify_all();
        }
        self.live.shutdown();
        self.runner.shutdown();
    }

    /// A session was queued or capture ended.
    pub fn wake(&self) {
        self.runner.wake();
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
        match self.runner.retry_awaiting() {
            Ok(ids) if !ids.is_empty() => {
                log::info!("{} session(s) will try the new destination", ids.len())
            }
            Ok(_) => {}
            Err(e) => log::error!("could not retry sessions awaiting a destination: {e}"),
        }
    }

    pub fn retry(&self, session_id: &str) -> Result<(), String> {
        self.runner.retry(session_id).map_err(|e| e.to_string())
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
        let settings = settings::get_settings(&self.app).recognition_config();
        self.runner
            .check_transcribe_again(session_id, &settings, &|spec| self.readiness(spec))
            .map(|spec| spec.config.asr_model)
    }

    /// Queue a saved note to be transcribed again with the recognition
    /// settings (R15, KTD2). `model_id` is the model that the user
    /// confirmed.
    pub fn transcribe_again(&self, session_id: &str, model_id: &str) -> Result<(), Refusal> {
        let settings = settings::get_settings(&self.app).recognition_config();
        self.runner
            .transcribe_again(session_id, &settings, model_id, &|spec| {
                self.readiness(spec)
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
        self.runner
            .regenerate_summary(session_id)
            .await
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(|e| e.to_string())
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
}
