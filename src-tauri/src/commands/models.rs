//! Thin IPC over the models manager.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::managers::capture::CaptureManager;
use crate::managers::models::{
    self, DeleteInputs, DeleteScope, ModelDeleteFailure, ModelDeletionSummary, ModelStatus,
    ModelsManager, SpeakerModelsStatus,
};
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

/// The data of the "Delete models…" dialog: the downloaded models with the
/// `unused` flag, the count of unfinished notes, and why "Delete all" is
/// off (KTD4).
#[tauri::command]
#[specta::specta]
pub fn model_deletion_summary(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    capture: State<'_, Arc<CaptureManager>>,
) -> Result<ModelDeletionSummary, String> {
    let settings = settings::get_settings(&app);
    // Read the recording state before the store lock: the capture manager
    // locks its state and then the store.
    let recording = capture.is_capturing();
    let sessions = {
        let store = capture.shared_store();
        let store = store.lock().unwrap_or_else(|e| e.into_inner());
        store.sessions().map_err(|e| e.to_string())?
    };
    let live_model = manager.live_model();
    let statuses = statuses_on_disk(&manager, &settings);
    Ok(models::deletion_summary(
        &statuses,
        &DeleteInputs {
            settings: &settings,
            live_model: live_model.as_deref(),
            sessions: &sessions,
            recording,
            downloading: manager.is_downloading(),
        },
    ))
}

/// Delete the downloaded models of `scope` (KTD5). Returns the models that
/// the app could not delete. The `all` scope refuses while a recording, a
/// download, or a transcription is in progress.
#[tauri::command]
#[specta::specta]
pub fn delete_models(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    capture: State<'_, Arc<CaptureManager>>,
    scope: DeleteScope,
) -> Result<Vec<ModelDeleteFailure>, String> {
    let settings = settings::get_settings(&app);
    let processing = app.try_state::<Arc<ProcessingManager>>();
    let statuses = statuses_on_disk(&manager, &settings);
    // Keep the capture state lock and the store lock from the checks to
    // the last delete. Then no recording can start, and the runner cannot
    // claim a queued note, between them. The unload and the delete do not
    // lock the capture state or the store.
    capture.with_recording_held(|recording, store| {
        let sessions = store.sessions().map_err(|e| e.to_string())?;
        let live_model = manager.live_model();
        let inputs = DeleteInputs {
            settings: &settings,
            live_model: live_model.as_deref(),
            sessions: &sessions,
            recording,
            downloading: manager.is_downloading(),
        };
        models::delete_by_scope(scope, &statuses, &inputs, |id| {
            if let Some(processing) = &processing {
                processing.unload_engine(id);
            }
            manager.delete_files(id)
        })
    })
}

/// The model statuses. Only the `Present` state matters to a delete, so
/// a configuration that does not resolve falls back to the selected model.
fn statuses_on_disk(manager: &ModelsManager, settings: &settings::Settings) -> Vec<ModelStatus> {
    manager
        .statuses_for(&settings.recognition_config())
        .unwrap_or_else(|_| manager.statuses(&settings.asr_model))
}

#[tauri::command]
#[specta::specta]
pub fn resolve_recognition(
    config: ::models::recognition::RecognitionConfig,
) -> Result<::models::recognition::ExecutionSpec, String> {
    ::models::recognition::resolve(&config).map_err(|e| e.to_string())
}

/// Fetch the exact artifacts of a saved configuration.
#[tauri::command]
#[specta::specta]
pub fn download_recognition_artifacts(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    spec: ::models::recognition::ExecutionSpec,
) -> Result<(), String> {
    manager.start_spec_downloads(&app, &spec)
}
