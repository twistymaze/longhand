//! The alert path's setup and pre-check (R5a, KTD14).

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::managers::alerts::{AlertCheck, AlertsManager};
use crate::managers::settings::{self, Settings};

/// Whether an alert would reach the user now. Run as recording starts and
/// polled while it lasts; the output is probed again when it changes.
#[tauri::command]
#[specta::specta]
pub async fn alert_check(
    app: AppHandle,
    alerts: State<'_, Arc<AlertsManager>>,
    force: bool,
) -> Result<AlertCheck, String> {
    let alerts = Arc::clone(&alerts);
    let tone_heard = settings::get_settings(&app).alert_tone_heard;
    tauri::async_runtime::spawn_blocking(move || alerts.check(tone_heard, force))
        .await
        .map_err(|e| e.to_string())
}

/// Play the failed-capture cue once, for the user to confirm hearing it.
#[tauri::command]
#[specta::specta]
pub fn play_alert_sound() -> Result<(), String> {
    sfx::play_cue().map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn confirm_alert_sound(app: AppHandle, heard: bool) -> Result<Settings, String> {
    settings::update_settings(&app, |s| {
        s.alert_tone_heard = heard;
        Ok(())
    })
}
