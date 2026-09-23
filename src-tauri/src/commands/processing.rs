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
