//! Thin IPC over the processing manager.

use std::sync::Arc;

use tauri::State;

use crate::managers::processing::ProcessingManager;

/// Summarize a landed session again, replacing its `summary.md` (R19,
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
