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

/// Ready to record: the microphone authorized, and the recording chord
/// either registered or cleared. No chord is a choice; a chord that failed
/// to register is a fault (KTD7, R13). The storage folder does not count: a
/// new install has the default folder, and a note waits to save while the
/// folder is not available (AE1).
fn armed(microphone: bool, shortcut: &ShortcutStatus) -> bool {
    microphone && shortcut.is_ok()
}

/// The system audio permission, as the page reads it (R9, R10, KTD12).
#[derive(Serialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemAudioPermission {
    Allowed,
    Denied,
    /// macOS has not asked yet, or its prompt waits for an answer.
    NotAsked,
    /// The private check is missing on this macOS.
    Unknown,
}

impl From<audio_system::Permission> for SystemAudioPermission {
    fn from(permission: audio_system::Permission) -> Self {
        match permission {
            audio_system::Permission::Allowed => Self::Allowed,
            audio_system::Permission::Denied => Self::Denied,
            audio_system::Permission::NotAsked => Self::NotAsked,
            audio_system::Permission::Unknown => Self::Unknown,
        }
    }
}

/// The Screen & System Audio Recording page of System Settings.
#[cfg(target_os = "macos")]
const SYSTEM_AUDIO_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// Check the system audio permission. It asks the user nothing.
async fn check_system_audio() -> Result<SystemAudioPermission, String> {
    tauri::async_runtime::spawn_blocking(audio_system::permission::preflight)
        .await
        .map(SystemAudioPermission::from)
        .map_err(|e| e.to_string())
}

/// The system audio permission now. The General pane polls it after
/// "Allow…", while the macOS prompt waits for an answer.
#[tauri::command]
#[specta::specta]
pub async fn system_audio_permission() -> Result<SystemAudioPermission, String> {
    check_system_audio().await
}

/// Ask for the system audio permission: a short tap makes macOS show its
/// prompt (KTD12). It runs away from the command thread and returns the
/// state after the tap, which is "not asked" while the prompt waits.
#[tauri::command]
#[specta::specta]
pub async fn request_system_audio_permission() -> Result<SystemAudioPermission, String> {
    tauri::async_runtime::spawn_blocking(audio_system::permission::request)
        .await
        .map(SystemAudioPermission::from)
        .map_err(|e| e.to_string())
}

/// Open the Screen & System Audio Recording page of System Settings, where
/// the user allows a denied permission.
#[tauri::command]
#[specta::specta]
pub async fn open_system_audio_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(SYSTEM_AUDIO_SETTINGS)
            .status()
            .map_err(|e| e.to_string())
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("System Settings did not open ({status})"))
                }
            })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("this platform has no system audio permission".to_string())
    }
}

#[derive(Serialize, Type)]
pub struct Readiness {
    pub microphone_authorized: bool,
    /// The system audio permission (R9). It does not change `armed`: the
    /// microphone alone is enough to record.
    pub system_audio: SystemAudioPermission,
    pub destination: DestinationStatus,
    /// Microphone authorized, and the chord registered or cleared (R1c,
    /// R13). A storage folder that is not available now is shown, and does
    /// not block a recording: the note waits to save (AE1).
    pub armed: bool,
    /// A new note can start, with or without a recording: the storage
    /// folder is available (R7). It does not change `armed`.
    pub can_start_note: bool,
    /// Why a new note cannot start, while `can_start_note` is false.
    pub note_start_refusal: Option<String>,
    pub shortcut: ShortcutStatus,
    /// Catalog ids of the weights that the next note needs and that are
    /// not on disk yet. They do not block a recording: processing waits
    /// for them (KTD16).
    pub missing_models: Vec<String>,
    /// A provider with an address and a model is active, and the request
    /// gate lets summary processing use it (R12). A locked Keychain does
    /// not clear it.
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
    let system_audio = check_system_audio().await?;
    let folder = settings.storage_folder.clone();
    let destination = tauri::async_runtime::spawn_blocking(move || destination::status(&folder))
        .await
        .map_err(|e| e.to_string())?;
    // A local provider takes no key (R8).
    let endpoint_configured = if settings.provider_configured() {
        let location = settings
            .active_provider
            .and_then(|provider| settings.key_location(provider));
        let read = match location {
            Some(location) => secrets.run(move |store| location.read(store)).await,
            None => Ok(None),
        };
        settings::provider_ready(&settings, settings::key_state(&read))
    } else {
        false
    };
    let note_start_refusal = destination::note_start_refusal(&destination);
    Ok(Readiness {
        microphone_authorized,
        system_audio,
        armed: armed(microphone_authorized, &shortcut),
        can_start_note: note_start_refusal.is_none(),
        note_start_refusal: note_start_refusal.map(str::to_string),
        shortcut,
        destination,
        missing_models: models.missing_spec(
            &crate::managers::capture::recording_spec(
                settings.recognition_config(),
                settings.chosen_config(),
                |spec| models.missing_spec(spec).is_empty(),
            )
            .map_err(|e| e.to_string())?,
        ),
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
        assert!(armed(true, &status(None, false)));
    }

    #[test]
    fn a_failed_shortcut_is_not_ready() {
        assert!(!armed(true, &status(Some("Super+KeyR"), false)));
    }

    #[test]
    fn each_permission_state_maps_to_its_page_value() {
        use audio_system::Permission;
        assert_eq!(
            SystemAudioPermission::from(Permission::Allowed),
            SystemAudioPermission::Allowed
        );
        assert_eq!(
            SystemAudioPermission::from(Permission::Denied),
            SystemAudioPermission::Denied
        );
        assert_eq!(
            SystemAudioPermission::from(Permission::NotAsked),
            SystemAudioPermission::NotAsked
        );
        assert_eq!(
            SystemAudioPermission::from(Permission::Unknown),
            SystemAudioPermission::Unknown
        );
    }

    /// AE6: the page reads a denied permission from Readiness, and the
    /// microphone alone is enough to record, so `armed` does not change.
    #[test]
    fn a_denied_permission_is_reported_and_does_not_disarm() {
        let value = serde_json::to_value(SystemAudioPermission::from(
            audio_system::Permission::Denied,
        ))
        .unwrap();
        assert_eq!(value, serde_json::json!("denied"));
        assert_eq!(
            serde_json::to_value(SystemAudioPermission::NotAsked).unwrap(),
            serde_json::json!("not_asked")
        );
        assert!(armed(true, &status(None, false)));
    }

    #[test]
    fn a_registered_shortcut_is_ready() {
        assert!(armed(true, &status(Some("Super+KeyR"), true)));
        assert!(!armed(false, &status(Some("Super+KeyR"), true)));
    }
}
