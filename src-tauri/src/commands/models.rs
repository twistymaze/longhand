//! Thin IPC over the models manager.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::managers::capture::CaptureManager;
use crate::managers::models::{self, ModelStatus, ModelsManager, SpeakerModelsStatus};
use crate::managers::processing::ProcessingManager;
use crate::managers::settings;

#[tauri::command]
#[specta::specta]
pub fn model_statuses(app: AppHandle, manager: State<'_, Arc<ModelsManager>>) -> Vec<ModelStatus> {
    manager.statuses(&settings::get_settings(&app).asr_model)
}

/// The speaker-detection weights as one item (R7).
#[tauri::command]
#[specta::specta]
pub fn speaker_models_status(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
) -> SpeakerModelsStatus {
    models::speaker_models(&manager.statuses(&settings::get_settings(&app).asr_model))
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

/// Delete a downloaded recognition model to recover disk space (R6). The
/// app refuses a model that it still needs (KTD12).
#[tauri::command]
#[specta::specta]
pub fn delete_model(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    capture: State<'_, Arc<CaptureManager>>,
    model_id: String,
) -> Result<(), String> {
    let settings = settings::get_settings(&app);
    let sessions = {
        let store = capture.shared_store();
        let store = store.lock().unwrap_or_else(|e| e.into_inner());
        store.sessions().map_err(|e| e.to_string())?
    };
    let live_model = manager.live_model();
    if let Some(reason) =
        models::delete_refusal(&model_id, &settings, live_model.as_deref(), &sessions)
    {
        return Err(reason);
    }
    if let Some(processing) = app.try_state::<Arc<ProcessingManager>>() {
        processing.unload_engine(&model_id);
    }
    manager.delete_files(&model_id)
}
