//! Thin IPC over the processing manager.

use std::sync::Arc;

use tauri::State;

use crate::managers::processing::ProcessingManager;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ReprocessPreview {
    pub execution: models::recognition::ExecutionSpec,
    pub requires_transcription: bool,
    pub explanation: String,
    pub manifest: String,
}

#[tauri::command]
#[specta::specta]
pub fn preview_reprocess(
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
    config: models::recognition::RecognitionConfig,
) -> Result<ReprocessPreview, String> {
    let preview = processing.preview_reprocess(&session_id, &config)?;
    Ok(ReprocessPreview {
        execution: preview.execution,
        requires_transcription: preview.requires_transcription,
        explanation: preview.explanation,
        manifest: preview.manifest,
    })
}

#[tauri::command]
#[specta::specta]
pub fn reprocess_session(
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
    preview: ReprocessPreview,
    allow_transcription: bool,
) -> Result<(), String> {
    processing.reprocess(
        &session_id,
        &pipeline::reprocess::Preview {
            execution: preview.execution,
            requires_transcription: preview.requires_transcription,
            explanation: preview.explanation,
            manifest: preview.manifest,
        },
        allow_transcription,
    )
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

/// Transcribe a saved session again with the model selected now (R15). The
/// session is queued; its transcript and summary change when it saves.
#[tauri::command]
#[specta::specta]
pub fn transcribe_again(
    app: tauri::AppHandle,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<(), String> {
    let model = crate::managers::settings::get_settings(&app).asr_model;
    processing.transcribe_again(&session_id, &model)
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
