//! Thin IPC over the capture manager. The shortcut drives the same manager;
//! these let the surface start, stop and resume, and carry the scratchpad.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::managers::capture::{CaptureManager, NoteView, RecordingStatus};
use crate::managers::settings::{self, SpeakerCountSetting};
use crate::shortcut::{self, ShortcutRegistration, ShortcutStatus};

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
    let count = shortcut::speaker_count(settings::get_settings(&app).default_speaker_count);
    blocking(move || {
        manager
            .start_session(&app, count)
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn stop_recording(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
) -> Result<String, String> {
    let manager = Arc::clone(&manager);
    blocking(move || {
        manager
            .stop_session(&app)
            .map(|(id, state)| {
                log::info!("session {id} is now {}", state.name());
                state.name().to_string()
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
pub async fn recording_status(
    manager: State<'_, Arc<CaptureManager>>,
) -> Result<RecordingStatus, String> {
    let manager = Arc::clone(&manager);
    blocking(move || manager.status().map_err(|e| e.to_string())).await
}

/// Called at a line's first keystroke; the anchor is fixed now (R12).
#[tauri::command]
#[specta::specta]
pub async fn begin_note(
    manager: State<'_, Arc<CaptureManager>>,
    text: String,
) -> Result<NoteView, String> {
    let manager = Arc::clone(&manager);
    blocking(move || manager.begin_note(&text).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn edit_note(
    manager: State<'_, Arc<CaptureManager>>,
    note_id: i64,
    text: String,
) -> Result<(), String> {
    let manager = Arc::clone(&manager);
    blocking(move || manager.edit_note(note_id, &text).map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn scratchpad_notes(
    manager: State<'_, Arc<CaptureManager>>,
) -> Result<Vec<NoteView>, String> {
    let manager = Arc::clone(&manager);
    blocking(move || manager.notes().map_err(|e| e.to_string())).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_session_speaker_count(
    manager: State<'_, Arc<CaptureManager>>,
    count: Option<SpeakerCountSetting>,
) -> Result<(), String> {
    if let Some(count) = &count {
        count.validate()?;
    }
    let manager = Arc::clone(&manager);
    blocking(move || {
        manager
            .set_speaker_count(shortcut::speaker_count(count))
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub fn shortcut_status(registration: State<'_, ShortcutRegistration>) -> ShortcutStatus {
    registration.status()
}
