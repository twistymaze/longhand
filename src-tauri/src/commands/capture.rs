//! Thin IPC over the capture manager. The shortcut (U7) drives the same
//! manager; these let the surface start, stop, and resume as well.

use std::sync::Arc;

use session_core::{SessionState, SpeakerCount};
use tauri::{AppHandle, State};

use crate::managers::capture::CaptureManager;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn start_recording(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
) -> Result<String, String> {
    let manager = Arc::clone(&manager);
    let count: Option<SpeakerCount> = None;
    blocking(move || {
        manager
            .start_session(&app, count)
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn stop_recording(manager: State<'_, Arc<CaptureManager>>) -> Result<String, String> {
    let manager = Arc::clone(&manager);
    blocking(move || {
        manager
            .stop_session()
            .map(|(id, state): (String, SessionState)| {
                log::info!("session {id} is now {}", state.name());
                id
            })
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn resume_recording(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
    session_id: String,
) -> Result<String, String> {
    let manager = Arc::clone(&manager);
    blocking(move || {
        manager
            .resume_session(&app, &session_id)
            .map(|state| state.name().to_string())
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub fn is_recording(manager: State<'_, Arc<CaptureManager>>) -> bool {
    manager.is_capturing()
}
