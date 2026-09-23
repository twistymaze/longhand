//! Wires the pipeline runner (U8) into the app: models from the models
//! directory, suspension while capture is active (KTD20), and runner events
//! out to the surface.

use std::sync::Arc;

use asr::{Engine, Recognizer};
use diarize::{DiarizeError, OnnxAnalyzer, SpeakerAnalyzer};
use models::ModelKind;
use pipeline::{
    EndpointProblem, EngineProvider, Environment, Runner, RunnerEvent, SummaryEndpoint,
};
use secrets::{SecretError, SecretStore};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use tauri_specta::Event as _;

use crate::managers::capture::CaptureManager;
use crate::managers::models::ModelsManager;
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
    /// Summarization failed; the session lands with a stub summary.
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
    Landed {
        path: String,
        copied: bool,
    },
    Cancelled,
    Finished,
}

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
            RunnerEvent::Landed {
                session_id,
                path,
                copied,
            } => (
                session_id,
                ProcessingStep::Landed {
                    path: path.to_string_lossy().into_owned(),
                    copied,
                },
            ),
            RunnerEvent::Cancelled { session_id } => (session_id, ProcessingStep::Cancelled),
            RunnerEvent::Finished { session_id } => (session_id, ProcessingStep::Finished),
        };
        Self { session_id, update }
    }
}

struct Models {
    app: AppHandle,
    models: Arc<ModelsManager>,
    secrets: Arc<SecretStore>,
}

impl EngineProvider for Models {
    fn summary_endpoint(&self) -> Result<SummaryEndpoint, EndpointProblem> {
        let s = settings::get_settings(&self.app);
        if !s.endpoint_configured() {
            return Err(EndpointProblem::Unconfigured(
                "no summarization endpoint is set; choose one in Settings".into(),
            ));
        }
        // The credential is keyed by the configured host: a changed base URL
        // has none until one is entered for it (R30a).
        let credential = self
            .secrets
            .endpoint_credential(&s.endpoint_base_url)
            .map_err(|e| match e {
                SecretError::Locked { .. } | SecretError::Unavailable { .. } => {
                    EndpointProblem::Locked(e.to_string())
                }
                other => EndpointProblem::Unconfigured(other.to_string()),
            })?;
        Ok(SummaryEndpoint {
            learned_limit: s
                .learned_context_limits
                .get(&settings::learned_limit_key(
                    &s.endpoint_base_url,
                    &s.endpoint_model,
                ))
                .copied(),
            base_url: s.endpoint_base_url,
            model: s.endpoint_model,
            credential,
            context_limit: s.context_limit_tokens,
        })
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

    /// Read when the session lands, so the last destination chosen wins (AE7).
    fn destination(&self) -> Option<std::path::PathBuf> {
        settings::get_settings(&self.app)
            .current_destination
            .map(std::path::PathBuf::from)
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
        Engine::load(model_id, &path)
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
fn notify_landed(app: &AppHandle, path: &std::path::Path) {
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
        log::warn!("could not post the landing notification: {e}");
    }
}

pub struct ProcessingManager {
    runner: Arc<Runner>,
}

impl ProcessingManager {
    /// Build the runner and return sessions a previous run left mid-stage to
    /// the queue. Call [`ProcessingManager::start`] to begin working.
    pub fn new(
        app: &AppHandle,
        capture: &Arc<CaptureManager>,
        models: Arc<ModelsManager>,
        secrets: Arc<SecretStore>,
        liveness: session_core::jobs::Liveness,
    ) -> Self {
        let suspended = Arc::clone(capture);
        let events = app.clone();
        let runner = Runner::new(Environment {
            store: capture.shared_store(),
            working_root: capture.working_root().to_path_buf(),
            engines: Box::new(Models {
                app: app.clone(),
                models,
                secrets,
            }),
            suspended: Box::new(move || suspended.is_capturing()),
            events: Box::new(move |event| {
                if let RunnerEvent::Landed { path, .. } = &event {
                    notify_landed(&events, path);
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
        Self { runner }
    }

    pub fn start(&self) {
        tauri::async_runtime::spawn(Arc::clone(&self.runner).run());
    }

    /// A session was queued or capture ended.
    /// Before the process exits: stop processing and free the models.
    pub fn shutdown(&self) {
        self.runner.shutdown();
    }

    pub fn wake(&self) {
        self.runner.wake();
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

    /// Summarize a landed session again, replacing its `summary.md` (AE17).
    pub async fn regenerate_summary(&self, session_id: &str) -> Result<String, String> {
        self.runner
            .regenerate_summary(session_id)
            .await
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(|e| e.to_string())
    }
}
