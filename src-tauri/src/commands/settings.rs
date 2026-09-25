//! Thin IPC over the settings and secrets managers.
//!
//! No command returns a credential to the webview. The status reports only
//! presence, length, and a short fingerprint, which is enough to prove a
//! value survived a relaunch without moving the secret out of Rust.

use std::sync::Arc;

use serde::Serialize;
use sha2::{Digest, Sha256};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::managers::models::{self as model_manager, ModelsManager};
use crate::managers::processing::ProcessingManager;
use crate::managers::secrets::SecretsManager;
use crate::managers::settings::{self, Settings, SpeakerCountSetting, SummaryPromptList};

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

/// Pick the recognition model (R5, KD7). A pick that is not downloaded
/// starts its download. While the selected model is downloaded, it stays in
/// use until the pick's download is verified.
#[tauri::command]
#[specta::specta]
pub fn select_asr_model(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    model_id: String,
) -> Result<Settings, String> {
    match models::model(&model_id) {
        Some(m) if m.kind == models::ModelKind::Asr => {}
        _ => return Err(format!("'{model_id}' is not a recognition model")),
    }
    let mut pick = None;
    let settings = settings::update_settings(&app, |s| {
        pick = Some(model_manager::apply_pick(s, &model_id, |id| {
            manager.is_present(id)
        }));
        Ok(())
    })?;
    let pick = pick.expect("the settings change ran");
    if let Some(replaced) = pick.cancel {
        manager.cancel(&replaced);
    }
    if pick.download {
        manager.start_downloads(&app, vec![model_id])?;
    }
    Ok(settings)
}

/// Turn the live transcript of streaming models on or off (R18, KD9).
#[tauri::command]
#[specta::specta]
pub fn set_live_transcript(app: AppHandle, enabled: bool) -> Result<Settings, String> {
    settings::update_settings(&app, |s| {
        s.live_transcript = enabled;
        Ok(())
    })
}

/// The built-in summary prompts first, then the custom prompts (R1, R2).
#[tauri::command]
#[specta::specta]
pub fn summary_prompts(app: AppHandle) -> SummaryPromptList {
    settings::get_settings(&app).summary_prompt_list()
}

/// Add a summary prompt named "New prompt" with the Topic summary text (R3).
#[tauri::command]
#[specta::specta]
pub fn create_summary_prompt(app: AppHandle) -> Result<Settings, String> {
    settings::update_settings(&app, |s| {
        s.create_summary_prompt();
        Ok(())
    })
}

/// Add a copy of a summary prompt, named "X copy" (R3).
#[tauri::command]
#[specta::specta]
pub fn duplicate_summary_prompt(app: AppHandle, id: String) -> Result<Settings, String> {
    settings::update_settings(&app, |s| s.duplicate_summary_prompt(&id).map(drop))
}

#[tauri::command]
#[specta::specta]
pub fn rename_summary_prompt(app: AppHandle, id: String, name: String) -> Result<Settings, String> {
    settings::update_settings(&app, |s| s.rename_summary_prompt(&id, &name))
}

#[tauri::command]
#[specta::specta]
pub fn set_summary_prompt_text(
    app: AppHandle,
    id: String,
    text: String,
) -> Result<Settings, String> {
    settings::update_settings(&app, |s| s.set_summary_prompt_text(&id, &text))
}

/// Delete a custom summary prompt. The Topic summary becomes active when the
/// deleted prompt was active (R4).
#[tauri::command]
#[specta::specta]
pub fn delete_summary_prompt(app: AppHandle, id: String) -> Result<Settings, String> {
    settings::update_settings(&app, |s| s.delete_summary_prompt(&id))
}

/// Make a summary prompt the one that the next summary uses (R1).
#[tauri::command]
#[specta::specta]
pub fn activate_summary_prompt(app: AppHandle, id: String) -> Result<Settings, String> {
    settings::update_settings(&app, |s| s.activate_summary_prompt(&id))
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
            .set_title("Choose where sessions are saved")
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
