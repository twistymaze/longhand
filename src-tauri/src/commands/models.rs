//! Thin IPC over the models manager.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use super::blocking;
use crate::managers::capture::CaptureManager;
use crate::managers::models::{self, ModelStatus, ModelsManager, SpeakerModelsStatus};
use crate::managers::processing::ProcessingManager;
use crate::managers::settings;

#[tauri::command]
#[specta::specta]
pub fn model_statuses(app: AppHandle, manager: State<'_, Arc<ModelsManager>>) -> Vec<ModelStatus> {
    manager
        .statuses_for(&settings::get_settings(&app).recognition_config())
        .unwrap_or_default()
}

/// The speaker-detection weights as one item (R7).
#[tauri::command]
#[specta::specta]
pub fn speaker_models_status(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
) -> SpeakerModelsStatus {
    models::speaker_models(
        &manager
            .statuses_for(&settings::get_settings(&app).recognition_config())
            .unwrap_or_default(),
    )
}

/// Start fetching `ids`; progress and the outcome arrive as
/// `ModelDownloadEvent`s.
#[tauri::command]
#[specta::specta]
pub fn download_models(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    ids: Vec<String>,
) -> Result<(), String> {
    manager.start_downloads(&app, ids)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_model_downloads(manager: State<'_, Arc<ModelsManager>>) {
    manager.cancel_all();
}

/// Delete a downloaded transcription model, and each supporting model that
/// no kept bundle needs (R8). The app refuses a model that it still needs
/// (KTD12).
#[tauri::command]
#[specta::specta]
pub async fn delete_model(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    capture: State<'_, Arc<CaptureManager>>,
    model_id: String,
) -> Result<(), String> {
    let (manager, capture) = (Arc::clone(&manager), Arc::clone(&capture));
    blocking(move || delete_held(app, manager, capture, model_id)).await
}

/// The delete of [`delete_model`]. It waits for the capture state and the
/// store.
fn delete_held(
    app: AppHandle,
    manager: Arc<ModelsManager>,
    capture: Arc<CaptureManager>,
    model_id: String,
) -> Result<(), String> {
    let processing = app.try_state::<Arc<ProcessingManager>>();
    let delete = |id: &str| {
        if let Some(processing) = &processing {
            processing.unload_engine(id);
        }
        manager.delete_files(id)
    };
    // Keep the capture state lock and the store lock from the checks to
    // the last delete (KTD8). Then no recording can start, and the runner
    // cannot claim a queued note, between them. The unload and the delete
    // do not lock the capture state or the store.
    capture.with_recording_held(|_, store| {
        let settings = settings::get_settings(&app);
        let sessions = store.sessions().map_err(|e| e.to_string())?;
        let live_model = manager.live_model();
        if let Some(reason) =
            models::delete_refusal(&model_id, &settings, live_model.as_deref(), &sessions)
        {
            return Err(reason);
        }
        delete(&model_id)?;
        remove_unneeded(
            &manager,
            &settings,
            live_model.as_deref(),
            &sessions,
            delete,
        );
        Ok(())
    })
}

/// At launch, delete each supporting model that no kept bundle needs (R9).
pub fn remove_unneeded_supporting_models(app: &AppHandle) {
    let manager = app.state::<Arc<ModelsManager>>();
    let capture = app.state::<Arc<CaptureManager>>();
    capture.with_recording_held(|_, store| {
        let sessions = match store.sessions() {
            Ok(sessions) => sessions,
            Err(e) => {
                log::warn!("could not read the notes for the model cleanup: {e}");
                return;
            }
        };
        let live_model = manager.live_model();
        remove_unneeded(
            &manager,
            &settings::get_settings(app),
            live_model.as_deref(),
            &sessions,
            |id| manager.delete_files(id),
        );
    });
}

/// Delete the supporting models of [`models::unneeded_supporting_models`]
/// that [`models::delete_refusal`] allows. A failure is logged and does not
/// stop the others (KTD8).
fn remove_unneeded(
    manager: &ModelsManager,
    settings: &settings::Settings,
    live_model: Option<&str>,
    sessions: &[session_core::SessionRecord],
    delete: impl Fn(&str) -> Result<(), String>,
) {
    let statuses = manager
        .statuses_for(&settings.recognition_config())
        .unwrap_or_else(|_| manager.statuses(&settings.asr_model));
    for id in models::unneeded_supporting_models(&statuses, settings) {
        if models::delete_refusal(&id, settings, live_model, sessions).is_some() {
            continue;
        }
        match delete(&id) {
            Ok(()) => log::info!("deleted supporting model '{id}': no kept bundle needs it"),
            Err(e) => log::warn!("could not delete supporting model '{id}': {e}"),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn resolve_recognition(
    config: ::models::recognition::RecognitionConfig,
) -> Result<::models::recognition::ExecutionSpec, String> {
    ::models::recognition::resolve(&config).map_err(|e| e.to_string())
}

/// Download the missing files of the bundle of `asr_model` (R10).
#[tauri::command]
#[specta::specta]
pub fn download_model_bundle(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    asr_model: String,
) -> Result<(), String> {
    manager.start_bundle_downloads(&app, &asr_model, &[])
}
