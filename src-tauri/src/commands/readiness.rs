//! Whether the shortcut is armed, and whether processing will be able to run
//! (R1a, R1c).

use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, State};

use crate::managers::destination::{self, DestinationStatus};
use crate::managers::models::ModelsManager;
use crate::managers::settings;
use crate::shortcut::{ShortcutRegistration, ShortcutStatus};

#[derive(Serialize, Type)]
pub struct Readiness {
    pub microphone_authorized: bool,
    pub destination: DestinationStatus,
    /// Microphone authorized, a destination chosen, and the chord registered
    /// (R1c). A destination that is unreachable right now can be changed
    /// before the session is saved (R23), so it is shown rather than blocking.
    pub armed: bool,
    pub shortcut: ShortcutStatus,
    /// Catalog ids of required weights not yet on disk.
    pub missing_models: Vec<String>,
    pub endpoint_configured: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn readiness(
    app: AppHandle,
    models: State<'_, Arc<ModelsManager>>,
    registration: State<'_, ShortcutRegistration>,
) -> Result<Readiness, String> {
    let shortcut = registration.status();
    let settings = settings::get_settings(&app);
    let microphone_authorized = tauri_plugin_macos_permissions::check_microphone_permission().await;
    let current = settings.current_destination.clone();
    let destination =
        tauri::async_runtime::spawn_blocking(move || destination::status(current.as_deref()))
            .await
            .map_err(|e| e.to_string())?;
    Ok(Readiness {
        microphone_authorized,
        armed: microphone_authorized
            && settings.current_destination.is_some()
            && shortcut.registered,
        shortcut,
        destination,
        missing_models: models.missing(&settings.asr_model),
        endpoint_configured: settings.endpoint_configured(),
    })
}
