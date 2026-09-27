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

use crate::managers::capture::CaptureManager;
use crate::managers::models::{self as model_manager, ModelsManager};
use crate::managers::processing::ProcessingManager;
use crate::managers::secrets::SecretsManager;
use crate::managers::settings::{
    self, MicrophoneChoice, Settings, SpeakerCountSetting, SummaryPromptList,
};

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

#[tauri::command]
#[specta::specta]
pub fn login_item_status(app: AppHandle) -> Result<crate::autostart::LoginItemStatus, String> {
    crate::autostart::status(&app)
}

#[tauri::command]
#[specta::specta]
pub fn set_login_item(
    app: AppHandle,
    enabled: bool,
) -> Result<crate::autostart::LoginItemStatus, String> {
    crate::autostart::set_enabled(&app, enabled)
}

#[tauri::command]
#[specta::specta]
pub fn open_login_item_settings() -> Result<(), String> {
    crate::autostart::open_settings()
}

/// Save the Base URL of the summarization endpoint (R32). An empty value
/// clears it and stays empty (R2). Any other value must be a URL with a
/// host, because the credential is keyed by that host (R30a). The Model does
/// not change (KTD5).
#[tauri::command]
#[specta::specta]
pub fn set_endpoint_base_url(app: AppHandle, base_url: String) -> Result<Settings, String> {
    settings::update_settings(&app, |s| s.set_endpoint_base_url(&base_url))
}

/// Save the Model of the summarization endpoint. The Base URL does not
/// change, so a new install keeps "never saved" (KTD5).
#[tauri::command]
#[specta::specta]
pub fn set_endpoint_model(app: AppHandle, model: String) -> Result<Settings, String> {
    settings::update_settings(&app, |s| {
        s.set_endpoint_model(&model);
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

/// One connected input, with cpal's stable ID (KTD1).
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct MicrophoneInput {
    pub id: String,
    pub name: String,
}

/// The connected inputs, and the ID of the system default among them.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct MicrophoneList {
    pub default_id: Option<String>,
    /// By name. Inputs with the same name keep the order of the device list.
    pub inputs: Vec<MicrophoneInput>,
}

impl From<audio_cpal::Inputs> for MicrophoneList {
    fn from(inputs: audio_cpal::Inputs) -> Self {
        Self {
            default_id: inputs.default.map(|d| d.id),
            inputs: inputs
                .all
                .into_iter()
                .map(|i| MicrophoneInput {
                    id: i.id,
                    name: i.name,
                })
                .collect(),
        }
    }
}

/// The connected inputs for the Microphone choice (R1). The device list can
/// block, so it runs on a blocking thread, not the command thread, as in
/// Handy.
#[tauri::command]
#[specta::specta]
pub async fn list_microphones() -> Result<MicrophoneList, String> {
    tauri::async_runtime::spawn_blocking(|| MicrophoneList::from(audio_cpal::CpalAudio::inputs()))
        .await
        .map_err(|e| e.to_string())
}

/// Choose the usual microphone, or `None` for "System default" (R1).
#[tauri::command]
#[specta::specta]
pub fn set_microphone(
    app: AppHandle,
    microphone: Option<MicrophoneChoice>,
) -> Result<Settings, String> {
    let updated = settings::update_settings(&app, |s| s.set_microphone(microphone))?;
    // Resolve the new choice off the start path, so the next note starts
    // at once (R3).
    if let Some(capture) = app.try_state::<Arc<CaptureManager>>() {
        let capture = Arc::clone(&capture);
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = capture.prepare(&app) {
                log::warn!("the chosen microphone was not prepared: {e}");
            }
        });
    }
    Ok(updated)
}

/// Choose whether a new note starts with system audio on (R6).
#[tauri::command]
#[specta::specta]
pub fn set_system_audio_default(app: AppHandle, enabled: bool) -> Result<Settings, String> {
    if enabled && !crate::managers::capture::SYSTEM_AUDIO_AVAILABLE {
        return Err(crate::commands::capture::SYSTEM_AUDIO_UNAVAILABLE.to_string());
    }
    settings::update_settings(&app, |s| {
        s.system_audio_default = enabled;
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

/// Sessions waiting for a destination try the new one (R22b), and the
/// library watches start again on the retained roots (KTD7).
fn destination_changed(app: &AppHandle) {
    if let Some(processing) = app.try_state::<std::sync::Arc<ProcessingManager>>() {
        processing.retry_awaiting();
        processing.roots_changed();
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

/// The result of a model list request, for the Summaries pane (KTD2). It
/// has every `ModelList` outcome, and `NotRequested` when the request gate
/// sends no request (KTD3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum EndpointModels {
    Listed { ids: Vec<String> },
    KeyNeeded,
    KeyRejected,
    KeyCannotList,
    NotListed,
    Redirected { host: Option<String> },
    ServerError { status: Option<u16> },
    Unreachable,
    NotRequested { reason: ListNotRequested },
}

/// Why Longhand sent no model list request (R4, R16, R17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum ListNotRequested {
    NoBaseUrl,
    KeychainLocked,
    KeychainFailed,
    KeyNeeded,
}

impl From<settings::NotRequestedReason> for ListNotRequested {
    fn from(reason: settings::NotRequestedReason) -> Self {
        use settings::NotRequestedReason as R;
        match reason {
            R::NoBaseUrl => Self::NoBaseUrl,
            R::KeychainLocked => Self::KeychainLocked,
            R::KeychainFailed => Self::KeychainFailed,
            R::KeyNeeded => Self::KeyNeeded,
        }
    }
}

impl From<summarize::ModelList> for EndpointModels {
    fn from(list: summarize::ModelList) -> Self {
        use summarize::ModelList as L;
        match list {
            L::Listed { ids } => Self::Listed { ids },
            L::KeyNeeded => Self::KeyNeeded,
            L::KeyRejected => Self::KeyRejected,
            L::KeyCannotList => Self::KeyCannotList,
            L::NotListed => Self::NotListed,
            L::Redirected { host } => Self::Redirected { host },
            L::ServerError { status } => Self::ServerError { status },
            L::Unreachable => Self::Unreachable,
        }
    }
}

/// What the list command does next: send a request to this endpoint, or
/// return an outcome with no request.
#[derive(Debug, PartialEq, Eq)]
enum ListStep {
    Request(summarize::Endpoint),
    Done(EndpointModels),
}

/// Apply the request gate (KTD3) to the settings and to the result of the
/// Keychain read.
fn list_step(s: &Settings, read: Result<Option<String>, secrets::SecretError>) -> ListStep {
    use settings::EndpointGate;
    let base_url = s.effective_endpoint_base_url();
    let key = settings::key_state(&read);
    let credential = match settings::endpoint_gate(base_url, s.endpoint_base_url_saved(), key) {
        EndpointGate::NotRequested(reason) => {
            return ListStep::Done(EndpointModels::NotRequested {
                reason: reason.into(),
            });
        }
        EndpointGate::RequestWithKey => read.ok().flatten(),
        EndpointGate::RequestWithoutKey => None,
    };
    ListStep::Request(summarize::Endpoint {
        base_url: base_url.to_string(),
        model: String::new(),
        credential,
    })
}

/// List the models of the effective endpoint (R5). The command reads the
/// key itself, so the webview never holds it. It sends no request when the
/// request gate refuses one (R4, R16, R17).
#[tauri::command]
#[specta::specta]
pub async fn list_endpoint_models(
    app: AppHandle,
    manager: State<'_, SecretsManager>,
) -> Result<EndpointModels, String> {
    let s = settings::get_settings(&app);
    let base_url = s.effective_endpoint_base_url().to_string();
    // An empty Base URL has no host to read a key for. The gate refuses it.
    let read = if base_url.is_empty() {
        Ok(None)
    } else {
        manager
            .run(move |store| store.endpoint_credential(&base_url))
            .await
    };
    let endpoint = match list_step(&s, read) {
        ListStep::Done(outcome) => return Ok(outcome),
        ListStep::Request(endpoint) => endpoint,
    };
    // A Base URL that is not a valid http or https URL cannot be reached.
    let Ok(client) = summarize::Client::new(endpoint) else {
        return Ok(EndpointModels::Unreachable);
    };
    Ok(summarize::list_models(&client).await.into())
}

fn fingerprint(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrets::SecretError;
    use summarize::ModelList;

    const LOCAL: &str = "http://localhost:11434/v1";

    #[test]
    fn the_microphone_list_keeps_each_id_and_names_the_default_by_id() {
        let input = |id: &str, name: &str| audio_cpal::Input {
            id: id.into(),
            name: name.into(),
        };
        let list = MicrophoneList::from(audio_cpal::Inputs {
            default: Some(input("usb-2", "USB Audio")),
            all: vec![input("usb-1", "USB Audio"), input("usb-2", "USB Audio")],
        });
        assert_eq!(list.default_id.as_deref(), Some("usb-2"));
        let ids: Vec<&str> = list.inputs.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["usb-1", "usb-2"]);
        assert_eq!(
            serde_json::to_value(&list).unwrap()["inputs"][0],
            serde_json::json!({ "id": "usb-1", "name": "USB Audio" })
        );

        let none = MicrophoneList::from(audio_cpal::Inputs::default());
        assert_eq!(none.default_id, None);
        assert!(none.inputs.is_empty());
    }

    fn saved(base_url: &str) -> Settings {
        let mut s = Settings::default();
        s.set_endpoint_base_url(base_url).unwrap();
        s
    }

    fn not_requested(reason: ListNotRequested) -> ListStep {
        ListStep::Done(EndpointModels::NotRequested { reason })
    }

    #[test]
    fn the_default_endpoint_with_no_key_is_not_requested() {
        // Covers AE1: a new install sends nothing to api.openai.com.
        assert_eq!(
            list_step(&Settings::default(), Ok(None)),
            not_requested(ListNotRequested::KeyNeeded)
        );
    }

    #[test]
    fn the_default_endpoint_with_a_key_is_requested_with_the_key() {
        assert_eq!(
            list_step(&Settings::default(), Ok(Some("sk-test".into()))),
            ListStep::Request(summarize::Endpoint {
                base_url: "https://api.openai.com/v1".into(),
                model: String::new(),
                credential: Some("sk-test".into()),
            })
        );
    }

    #[test]
    fn a_saved_local_endpoint_with_no_key_is_requested_with_no_key() {
        assert_eq!(
            list_step(&saved(LOCAL), Ok(None)),
            ListStep::Request(summarize::Endpoint {
                base_url: LOCAL.into(),
                model: String::new(),
                credential: None,
            })
        );
    }

    #[test]
    fn an_empty_base_url_is_not_requested() {
        assert_eq!(
            list_step(&saved(""), Ok(Some("sk-test".into()))),
            not_requested(ListNotRequested::NoBaseUrl)
        );
    }

    #[test]
    fn a_keychain_that_cannot_be_read_is_not_requested() {
        // Covers AE8. Locked and Unavailable map to locked, as processing
        // maps them.
        for error in [
            SecretError::Locked {
                message: "locked".into(),
            },
            SecretError::Unavailable {
                message: "none".into(),
            },
        ] {
            assert_eq!(
                list_step(&saved(LOCAL), Err(error)),
                not_requested(ListNotRequested::KeychainLocked)
            );
        }
        assert_eq!(
            list_step(
                &Settings::default(),
                Err(SecretError::Platform {
                    message: "broken".into(),
                })
            ),
            not_requested(ListNotRequested::KeychainFailed)
        );
    }

    #[test]
    fn every_list_outcome_maps_to_the_same_frontend_outcome() {
        let cases = [
            (
                ModelList::Listed {
                    ids: vec!["a".into()],
                },
                EndpointModels::Listed {
                    ids: vec!["a".into()],
                },
            ),
            (ModelList::KeyNeeded, EndpointModels::KeyNeeded),
            (ModelList::KeyRejected, EndpointModels::KeyRejected),
            (ModelList::KeyCannotList, EndpointModels::KeyCannotList),
            (ModelList::NotListed, EndpointModels::NotListed),
            (
                ModelList::Redirected {
                    host: Some("example.com".into()),
                },
                EndpointModels::Redirected {
                    host: Some("example.com".into()),
                },
            ),
            (
                ModelList::ServerError { status: Some(500) },
                EndpointModels::ServerError { status: Some(500) },
            ),
            (ModelList::Unreachable, EndpointModels::Unreachable),
        ];
        for (list, expected) in cases {
            assert_eq!(EndpointModels::from(list), expected);
        }
    }

    #[test]
    fn the_outcome_serializes_to_the_shape_that_the_bindings_declare() {
        let json = |value: EndpointModels| serde_json::to_value(value).unwrap();
        assert_eq!(
            json(EndpointModels::Listed {
                ids: vec!["gpt-4o".into()]
            }),
            serde_json::json!({ "kind": "listed", "ids": ["gpt-4o"] })
        );
        assert_eq!(
            json(EndpointModels::KeyCannotList),
            serde_json::json!({ "kind": "key-cannot-list" })
        );
        assert_eq!(
            json(EndpointModels::Redirected { host: None }),
            serde_json::json!({ "kind": "redirected", "host": null })
        );
        assert_eq!(
            json(EndpointModels::ServerError { status: Some(502) }),
            serde_json::json!({ "kind": "server-error", "status": 502 })
        );
        assert_eq!(
            json(EndpointModels::NotRequested {
                reason: ListNotRequested::KeychainLocked
            }),
            serde_json::json!({ "kind": "not-requested", "reason": "keychain-locked" })
        );

        let bindings = include_str!("../../../src/bindings.ts");
        for declaration in [
            "export type EndpointModels = { kind: \"listed\"; ids: string[] } | \
             { kind: \"key-needed\" } | { kind: \"key-rejected\" } | \
             { kind: \"key-cannot-list\" } | { kind: \"not-listed\" } | \
             { kind: \"redirected\"; host: string | null } | \
             { kind: \"server-error\"; status: number | null } | \
             { kind: \"unreachable\" } | \
             { kind: \"not-requested\"; reason: ListNotRequested }",
            "export type ListNotRequested = \"no-base-url\" | \"keychain-locked\" | \
             \"keychain-failed\" | \"key-needed\"",
            "async listEndpointModels() : Promise<Result<EndpointModels, string>>",
        ] {
            assert!(
                bindings.contains(declaration),
                "src/bindings.ts does not declare: {declaration}"
            );
        }
    }
}

/// Serialize zoom changes off the main thread, which native webviews use.
#[tauri::command]
#[specta::specta]
pub async fn change_interface_zoom(
    app: AppHandle,
    action: crate::interface_zoom::ZoomAction,
) -> Result<u16, String> {
    tauri::async_runtime::spawn_blocking(move || crate::interface_zoom::change(&app, action))
        .await
        .map_err(|e| e.to_string())?
}
