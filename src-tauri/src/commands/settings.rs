//! Thin IPC over the settings and secrets managers.
//!
//! No command returns a credential to the webview. The status reports only
//! presence, length, and a short fingerprint, which is enough to prove a
//! value survived a relaunch without moving the secret out of Rust.

use serde::Serialize;
use sha2::{Digest, Sha256};
use specta::Type;
use tauri::{AppHandle, State};

use crate::managers::secrets::SecretsManager;
use crate::managers::settings;

#[derive(Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialStatus {
    Present {
        host: String,
        length: u32,
        fingerprint: String,
    },
    Absent {
        host: String,
    },
    /// Locked or refused: the user can recover it (R20b).
    NeedsUnlock {
        message: String,
    },
    Failed {
        message: String,
    },
}

fn status_from_error(error: secrets::SecretError) -> CredentialStatus {
    if error.needs_unlock() {
        CredentialStatus::NeedsUnlock {
            message: error.to_string(),
        }
    } else {
        CredentialStatus::Failed {
            message: error.to_string(),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_probe_setting(app: AppHandle) -> String {
    settings::get_settings(&app).probe_value
}

#[tauri::command]
#[specta::specta]
pub fn set_probe_setting(app: AppHandle, value: String) -> Result<(), String> {
    let mut current = settings::get_settings(&app);
    current.probe_value = value;
    settings::write_settings(&app, &current)
}

#[tauri::command]
#[specta::specta]
pub async fn credential_status(
    manager: State<'_, SecretsManager>,
    base_url: String,
) -> Result<CredentialStatus, String> {
    let host = match secrets::endpoint_key(&base_url) {
        Ok(host) => host,
        Err(e) => {
            return Ok(CredentialStatus::Failed {
                message: e.to_string(),
            });
        }
    };
    Ok(
        match manager
            .run(move |store| store.endpoint_credential(&base_url))
            .await
        {
            Ok(Some(secret)) => CredentialStatus::Present {
                host,
                length: secret.chars().count() as u32,
                fingerprint: fingerprint(&secret),
            },
            Ok(None) => CredentialStatus::Absent { host },
            Err(e) => status_from_error(e),
        },
    )
}

#[tauri::command]
#[specta::specta]
pub async fn set_credential(
    manager: State<'_, SecretsManager>,
    base_url: String,
    value: String,
) -> Result<(), String> {
    manager
        .run(move |store| store.set_endpoint_credential(&base_url, &value))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_credential(
    manager: State<'_, SecretsManager>,
    base_url: String,
) -> Result<(), String> {
    manager
        .run(move |store| store.delete_endpoint_credential(&base_url))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn repair_keychain_access() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(secrets::repair_keychain_access)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

fn fingerprint(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}
