//! Thin IPC over the processing manager.

use std::sync::Arc;

use tauri::State;

use crate::managers::processing::ProcessingManager;

/// The transcription model of a Transcribe Again run, for the dialog.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct TranscribeAgainCheck {
    pub model_id: String,
    pub model_name: String,
}

/// Why Transcribe Again cannot start. `in_settings` is true when the user
/// can fix the cause in Settings (KTD4).
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct TranscribeAgainRefusal {
    pub message: String,
    pub in_settings: bool,
}

impl From<pipeline::reprocess::Refusal> for TranscribeAgainRefusal {
    fn from(refusal: pipeline::reprocess::Refusal) -> Self {
        Self {
            message: refusal.message,
            in_settings: refusal.in_settings,
        }
    }
}

fn stopped(e: tauri::Error) -> TranscribeAgainRefusal {
    TranscribeAgainRefusal {
        message: e.to_string(),
        in_settings: false,
    }
}

/// Check if a saved note can be transcribed again with the recognition
/// settings now (R7, KTD2). This is read-only. The dialog opens only when
/// this gives the model of the run.
#[tauri::command]
#[specta::specta]
pub async fn check_transcribe_again(
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<TranscribeAgainCheck, TranscribeAgainRefusal> {
    let processing = Arc::clone(&processing);
    let model_id = tauri::async_runtime::spawn_blocking(move || {
        processing.check_transcribe_again(&session_id)
    })
    .await
    .map_err(stopped)??;
    Ok(TranscribeAgainCheck {
        model_name: pipeline::reprocess::model_name(&model_id),
        model_id,
    })
}

/// Transcribe a saved note again with the recognition settings (R15). Every
/// check runs again, and the command refuses when the settings no longer
/// select `model_id`, the model that the dialog showed. The note is queued;
/// its transcript and summary change when it saves. The row follows a
/// folder that can be synced, so this runs off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn transcribe_again(
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
    model_id: String,
) -> Result<(), TranscribeAgainRefusal> {
    let processing = Arc::clone(&processing);
    tauri::async_runtime::spawn_blocking(move || {
        processing.transcribe_again(&session_id, &model_id)
    })
    .await
    .map_err(stopped)??;
    Ok(())
}

/// Summarize a saved session again, replacing its `summary.md` (R19,
/// AE17). Returns the summary's path.
#[tauri::command]
#[specta::specta]
pub async fn regenerate_summary(
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<String, String> {
    let processing = Arc::clone(&processing);
    processing.regenerate_summary(&session_id).await
}

/// Retry a failed session, or one waiting for its destination, from its
/// checkpoints.
#[tauri::command]
#[specta::specta]
pub fn retry_session(
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<(), String> {
    processing.retry(&session_id)
}

/// Try a blocked session's secret store again. Reading a locked keychain
/// item is what asks macOS to unlock it (R20b).
#[tauri::command]
#[specta::specta]
pub fn unblock_session(
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<(), String> {
    processing.unblock(&session_id)
}
