//! Thin IPC over the capture manager. The shortcut drives the same manager;
//! these let the surface start, stop and resume, and carry the scratchpad.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::managers::capture::{CaptureManager, RecordingStatus};
use crate::managers::processing::ProcessingManager;
use crate::managers::settings::{self, MicrophoneChoice, SpeakerCountSetting};
use crate::shortcut::{self, ShortcutKind, ShortcutRegistration, ShortcutStatuses};

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
    processing: State<'_, Arc<ProcessingManager>>,
) -> Result<String, String> {
    let manager = Arc::clone(&manager);
    let state = blocking(move || {
        manager
            .stop_session(&app)
            .map(|(id, state)| {
                log::info!("session {id} is now {}", state.name());
                state.name().to_string()
            })
            .map_err(|e| e.to_string())
    })
    .await?;
    processing.wake();
    Ok(state)
}

#[tauri::command]
#[specta::specta]
pub async fn resume_recording(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<String, String> {
    let manager = Arc::clone(&manager);
    let id = session_id.clone();
    let state = blocking(move || {
        manager
            .resume_session(&app, &id)
            .map(|state| state.name().to_string())
            .map_err(|e| e.to_string())
    })
    .await?;
    // Out of the queue (R20c); a stage running on it stops at its next checkpoint.
    if state == "capturing" {
        processing.cancel(&session_id);
    }
    Ok(state)
}

#[tauri::command]
#[specta::specta]
pub async fn recording_status(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
) -> Result<RecordingStatus, String> {
    let manager = Arc::clone(&manager);
    blocking(move || manager.status(&app).map_err(|e| e.to_string())).await
}

/// Change the microphone of one note from the recording page (R2). With no
/// `session_id`, the next note uses it. `None` is "System default". Settings
/// does not change. Returns the status after the change.
#[tauri::command]
#[specta::specta]
pub async fn set_note_microphone(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
    session_id: Option<String>,
    microphone: Option<MicrophoneChoice>,
) -> Result<RecordingStatus, String> {
    let manager = Arc::clone(&manager);
    blocking(move || {
        manager
            .set_note_microphone(&app, session_id.as_deref(), microphone)
            .and_then(|()| manager.status(&app))
            .map_err(|e| e.to_string())
    })
    .await
}

/// The refusal while Computer audio is not available.
pub const SYSTEM_AUDIO_UNAVAILABLE: &str = "Computer audio is not available yet.";

/// Turn system audio on or off for one note from the recording page (R8).
/// With no `session_id`, the next note starts with it. Settings does not
/// change. Returns the status after the change.
#[tauri::command]
#[specta::specta]
pub async fn set_note_system_audio(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
    session_id: Option<String>,
    enabled: bool,
) -> Result<RecordingStatus, String> {
    if enabled && !crate::managers::capture::SYSTEM_AUDIO_AVAILABLE {
        return Err(SYSTEM_AUDIO_UNAVAILABLE.to_string());
    }
    let manager = Arc::clone(&manager);
    blocking(move || {
        manager
            .set_note_system_audio(&app, session_id.as_deref(), enabled)
            .and_then(|()| manager.status(&app))
            .map_err(|e| e.to_string())
    })
    .await
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
pub fn shortcut_status(registration: State<'_, ShortcutRegistration>) -> ShortcutStatuses {
    registration.statuses()
}

/// Set, change or clear one shortcut. `None` clears it. Async, so the plugin
/// calls it makes run off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn set_shortcut(
    app: AppHandle,
    kind: ShortcutKind,
    chord: Option<String>,
) -> Result<ShortcutStatuses, String> {
    blocking(move || shortcut::set_shortcut(&app, kind, chord)).await
}

/// Pause both shortcuts while the user presses keys for a new one (R11).
#[tauri::command]
#[specta::specta]
pub async fn suspend_shortcuts(app: AppHandle) -> Result<(), String> {
    blocking(move || {
        shortcut::suspend(&app);
        Ok(())
    })
    .await
}

/// Register the saved chords again after capture ends.
#[tauri::command]
#[specta::specta]
pub async fn resume_shortcuts(app: AppHandle) -> Result<(), String> {
    blocking(move || {
        shortcut::resume(&app);
        Ok(())
    })
    .await
}
