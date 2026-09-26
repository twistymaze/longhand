//! Whether the shortcut is armed, and whether processing will be able to run
//! (R1a, R1c).

use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, State};

use crate::managers::destination::{self, DestinationStatus};
use crate::managers::models::ModelsManager;
use crate::managers::secrets::SecretsManager;
use crate::managers::settings;
use crate::shortcut::{ShortcutKind, ShortcutRegistration, ShortcutStatus};

/// Ready to record: the microphone authorized, a destination chosen, and the
/// recording chord either registered or cleared. No chord is a choice; a
/// chord that failed to register is a fault (KTD7, R13).
fn armed(microphone: bool, destination: bool, shortcut: &ShortcutStatus) -> bool {
    microphone && destination && shortcut.is_ok()
}

#[derive(Serialize, Type)]
pub struct Readiness {
    pub microphone_authorized: bool,
    pub destination: DestinationStatus,
    /// Microphone authorized, a destination chosen, and the chord registered
    /// or cleared (R1c, R13). A destination that is unreachable right now can be changed
    /// before the session is saved (R23), so it is shown rather than blocking.
    pub armed: bool,
    pub shortcut: ShortcutStatus,
    /// Catalog ids of required weights not yet on disk.
    pub missing_models: Vec<String>,
    /// A Base URL and a model are set, and the request gate lets summary
    /// processing use them (KTD3). A locked Keychain does not clear it.
    pub endpoint_configured: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn readiness(
    app: AppHandle,
    models: State<'_, Arc<ModelsManager>>,
    registration: State<'_, ShortcutRegistration>,
    secrets: State<'_, SecretsManager>,
) -> Result<Readiness, String> {
    let shortcut = registration.status(ShortcutKind::Recording);
    let settings = settings::get_settings(&app);
    let microphone_authorized = tauri_plugin_macos_permissions::check_microphone_permission().await;
    let current = settings.current_destination.clone();
    let destination =
        tauri::async_runtime::spawn_blocking(move || destination::status(current.as_deref()))
            .await
            .map_err(|e| e.to_string())?;
    // Read the key only when a Base URL and a model are set. Without them
    // the endpoint is not configured, whatever the key state is.
    let endpoint_configured = if settings.endpoint_configured() {
        let base_url = settings.effective_endpoint_base_url().to_string();
        let read = secrets
            .run(move |store| store.endpoint_credential(&base_url))
            .await;
        settings::endpoint_ready(&settings, settings::key_state(&read))
    } else {
        false
    };
    Ok(Readiness {
        microphone_authorized,
        armed: armed(
            microphone_authorized,
            settings.current_destination.is_some(),
            &shortcut,
        ),
        shortcut,
        destination,
        missing_models: models.missing(&settings.asr_model),
        endpoint_configured,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(chord: Option<&str>, registered: bool) -> ShortcutStatus {
        ShortcutStatus {
            chord: chord.map(str::to_string),
            registered,
            error: (!registered && chord.is_some()).then(|| "refused".to_string()),
        }
    }

    #[test]
    fn a_cleared_shortcut_is_ready_to_record() {
        assert!(armed(true, true, &status(None, false)));
    }

    #[test]
    fn a_failed_shortcut_is_not_ready() {
        assert!(!armed(true, true, &status(Some("Super+KeyR"), false)));
    }

    #[test]
    fn a_registered_shortcut_is_ready() {
        assert!(armed(true, true, &status(Some("Super+KeyR"), true)));
        assert!(!armed(false, true, &status(Some("Super+KeyR"), true)));
        assert!(!armed(true, false, &status(None, false)));
    }
}
