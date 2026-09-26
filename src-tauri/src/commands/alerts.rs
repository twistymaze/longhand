//! The alert path's pre-check and its sound (R5a, R18).

use std::sync::Arc;

use tauri::State;

use crate::managers::alerts::{AlertCheck, AlertsManager};

/// Whether an alert would reach the user now. Run as recording starts and
/// polled while it lasts; the output is probed again when it changes.
#[tauri::command]
#[specta::specta]
pub async fn alert_check(
    alerts: State<'_, Arc<AlertsManager>>,
    force: bool,
) -> Result<AlertCheck, String> {
    let alerts = Arc::clone(&alerts);
    tauri::async_runtime::spawn_blocking(move || alerts.check(force))
        .await
        .map_err(|e| e.to_string())
}

/// Play the failed-capture cue once, so the user knows the sound (R18).
#[tauri::command]
#[specta::specta]
pub fn play_alert_sound() -> Result<(), String> {
    sfx::play_cue().map_err(|e| e.to_string())
}
