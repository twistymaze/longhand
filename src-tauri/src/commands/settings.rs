//! Thin IPC over the settings and secrets managers.
//!
//! No command returns a credential to the webview. The status reports only
//! presence, length, and a short fingerprint, which is enough to prove a
//! value survived a relaunch without moving the secret out of Rust.

use serde::Serialize;
use sha2::{Digest, Sha256};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::managers::processing::ProcessingManager;
use crate::managers::secrets::SecretsManager;
use crate::managers::settings::{self, Settings, SpeakerCountSetting};

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
pub fn get_settings(app: AppHandle) -> Settings {
    settings::get_settings(&app)
}

/// Set the summarization endpoint (R32). An empty base URL clears it; any
/// other value must be a URL with a host, since the credential is keyed by
/// that host (R30a).
#[tauri::command]
#[specta::specta]
pub fn set_endpoint(app: AppHandle, base_url: String, model: String) -> Result<Settings, String> {
    let base_url = base_url.trim().to_string();
    if !base_url.is_empty() {
        secrets::endpoint_key(&base_url).map_err(|e| e.to_string())?;
    }
    settings::update_settings(&app, |s| {
        s.endpoint_base_url = base_url;
        s.endpoint_model = model.trim().to_string();
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_context_limit(app: AppHandle, limit: Option<u32>) -> Result<Settings, String> {
    if limit == Some(0) {
        return Err("a context limit must be above zero".into());
    }
    settings::update_settings(&app, |s| {
        s.context_limit_tokens = limit;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_default_speaker_count(
    app: AppHandle,
    count: Option<SpeakerCountSetting>,
) -> Result<Settings, String> {
    if let Some(count) = &count {
        count.validate()?;
    }
    settings::update_settings(&app, |s| {
        s.default_speaker_count = count;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn set_asr_model(app: AppHandle, model_id: String) -> Result<Settings, String> {
    match models::model(&model_id) {
        Some(m) if m.kind == models::ModelKind::Asr => {}
        _ => return Err(format!("'{model_id}' is not a recognition model")),
    }
    settings::update_settings(&app, |s| {
        s.asr_model = model_id;
        Ok(())
    })
}

/// Pick a folder with the system dialog, make it current, and retain it as a
/// root (R23, R25a). `None` when the user closes the dialog.
#[tauri::command]
#[specta::specta]
pub async fn choose_destination(app: AppHandle) -> Result<Option<Settings>, String> {
    let dialog_app = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .set_title("Choose where sessions land")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let folder = path.to_string_lossy().into_owned();
    let settings = settings::update_settings(&app, |s| {
        s.choose_destination(&folder);
        Ok(())
    })?;
    destination_changed(&app);
    Ok(Some(settings))
}

/// Sessions waiting for a destination try the new one (R22b).
fn destination_changed(app: &AppHandle) {
    if let Some(processing) = app.try_state::<std::sync::Arc<ProcessingManager>>() {
        processing.retry_awaiting();
    }
}

/// Switch to a folder chosen earlier.
#[tauri::command]
#[specta::specta]
pub fn set_current_destination(app: AppHandle, folder: String) -> Result<Settings, String> {
    settings::update_settings(&app, |s| {
        if !s.destination_roots.contains(&folder) {
            return Err("that folder has not been chosen as a destination".into());
        }
        s.current_destination = Some(folder);
        Ok(())
    })
    .inspect(|_| destination_changed(&app))
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
