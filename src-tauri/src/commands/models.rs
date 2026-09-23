//! Thin IPC over the models manager.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::managers::models::{ModelStatus, ModelsManager};
use crate::managers::settings;

#[tauri::command]
#[specta::specta]
pub fn model_statuses(app: AppHandle, manager: State<'_, Arc<ModelsManager>>) -> Vec<ModelStatus> {
    manager.statuses(&settings::get_settings(&app).asr_model)
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
