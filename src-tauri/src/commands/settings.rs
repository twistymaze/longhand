//! Thin IPC over the settings and secrets managers.
//!
//! No command returns a credential to the webview. The status reports only
//! presence, length, and a short fingerprint, which is enough to prove a
//! value survived a relaunch without moving the secret out of Rust.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::managers::audio_removal::{self, AudioRemovalManager};
use crate::managers::capture::CaptureManager;
use crate::managers::models::ModelsManager;
use crate::managers::processing::{MoveStatus, ProcessingManager};
use crate::managers::secrets::SecretsManager;
use crate::managers::settings::{
    self, MicrophoneChoice, Settings, SpeakerCountSetting, SummaryPromptList,
};
use crate::managers::shared::SharedSettingsManager;

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

/// Change the chosen recognition configuration with `change`, then compare
/// it with the configuration in use (R18, KTD14). The configuration in use
/// changes at once when its models are on disk; otherwise their downloads
/// start.
fn choose_recognition(
    app: &AppHandle,
    manager: &Arc<ModelsManager>,
    change: impl FnOnce(&mut settings::RecognitionChoice),
) -> Result<Settings, String> {
    settings::update_settings(app, |s| {
        change(&mut s.chosen_recognition);
        Ok(())
    })?;
    manager.follow_choice(app);
    Ok(settings::get_settings(app))
}

#[tauri::command(async)]
#[specta::specta]
pub fn set_default_speaker_count(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    count: Option<SpeakerCountSetting>,
) -> Result<Settings, String> {
    if let Some(count) = &count {
        count.validate()?;
    }
    choose_recognition(&app, &manager, |c| c.default_speaker_count = count)
}

/// Choose the recognition model (R5, R18). The model in use stays until
/// every file of the chosen configuration is on disk.
#[tauri::command(async)]
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
    choose_recognition(&app, &manager, |c| c.asr_model = model_id)
}

/// Turn the live transcript of streaming models on or off (R18, KD9).
#[tauri::command(async)]
#[specta::specta]
pub fn set_live_transcript(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    enabled: bool,
) -> Result<Settings, String> {
    choose_recognition(&app, &manager, |c| c.live_transcript = enabled)
}

/// Start the failed downloads of the chosen recognition configuration
/// again (R19).
#[tauri::command(async)]
#[specta::specta]
pub fn retry_recognition_downloads(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
) -> Settings {
    manager.retry_choice();
    manager.follow_choice(&app);
    // The failure is gone in every window.
    settings::emit_settings_changed(&app);
    settings::get_settings(&app)
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

/// Choose what New Note and the recording shortcut do (R8): start a note
/// that records, or open a note that does not record.
#[tauri::command]
#[specta::specta]
pub fn set_new_note_records(app: AppHandle, enabled: bool) -> Result<Settings, String> {
    settings::update_settings(&app, |s| {
        s.new_note_records = enabled;
        Ok(())
    })
}

/// Choose whether a recording that stops and cannot restart plays the
/// alert sound (R9). The next alert reads the new value.
#[tauri::command]
#[specta::specta]
pub fn set_alert_sound(app: AppHandle, enabled: bool) -> Result<Settings, String> {
    settings::update_settings(&app, |s| {
        s.alert_sound = enabled;
        Ok(())
    })
}

/// Choose whether the app checks for updates by itself (R10). The next
/// scheduled check reads the new value.
#[tauri::command(async)]
#[specta::specta]
pub fn set_check_updates_automatically(app: AppHandle, enabled: bool) -> Result<Settings, String> {
    settings::update_settings(&app, |s| {
        s.check_updates_automatically = enabled;
        Ok(())
    })
}

/// Choose when the audio of a saved note goes, or turn automatic removal
/// off (R1, R3). The value goes into the shared settings file, so it
/// applies on every computer that uses the storage folder. A number of
/// days outside the range is refused and changes nothing. After a saved
/// change that turns the setting on or shortens the delay, a removal pass
/// starts (R17, KTD6). The Settings window asks for the confirmation first.
#[tauri::command(async)]
#[specta::specta]
pub fn set_audio_removal(
    app: AppHandle,
    removal: settings::AudioRemovalSetting,
) -> Result<Settings, String> {
    removal.validate()?;
    // The comparison starts from the file, not from the cache: the cache can
    // be older than a change that another computer made.
    let before = read_removal_delay(&app);
    let updated = settings::update_settings(&app, |s| {
        s.audio_removal = removal;
        Ok(())
    })?;
    if audio_removal::removes_more(before, shared_removal_delay(&app))
        && let Some(pass) = app.try_state::<Arc<AudioRemovalManager>>()
    {
        pass.run_now();
    }
    Ok(updated)
}

/// The delay of the audio removal setting in the shared settings now, or
/// `None` when the setting is off, is not known, or the shared settings are
/// not available (KTD3).
fn shared_removal_delay(app: &AppHandle) -> Option<u32> {
    app.try_state::<Arc<SharedSettingsManager>>()
        .and_then(|shared| shared.snapshot().audio_removal_days())
}

/// The delay of the audio removal setting after a read of the shared
/// settings file, as [`shared_removal_delay`] gives it. The read only
/// updates this computer's cache and writes nothing (rule 1). When the file
/// has a change from another computer, the app does what the folder watcher
/// does for that change. The read can wait for an online-only file, so call
/// this off the main thread.
fn read_removal_delay(app: &AppHandle) -> Option<u32> {
    let shared = app.try_state::<Arc<SharedSettingsManager>>()?;
    let (changed, days) = reread_removal_delay(&shared);
    if changed {
        log::info!("shared settings: another computer changed them");
        settings::emit_settings_changed(app);
        if let Some(models) = app.try_state::<Arc<ModelsManager>>() {
            models.follow_choice(app);
        }
        // The change can turn the audio removal on or shorten its delay. It
        // needs no confirmation (KTD6).
        if let Some(pass) = app.try_state::<Arc<AudioRemovalManager>>() {
            pass.run_now();
        }
    }
    days
}

/// Read the shared settings file into the cache of `shared`. Returns
/// whether the read changed the shared settings, and the delay of the audio
/// removal setting after the read.
fn reread_removal_delay(shared: &SharedSettingsManager) -> (bool, Option<u32>) {
    let changed = shared.reread();
    (changed, shared.snapshot().audio_removal_days())
}

/// The delay to count with for the change from `before` to `proposed`, or
/// `None` when the change does not remove more audio at once (R17): the
/// change turns the setting off, keeps the delay, or makes it longer.
fn count_delay(before: Option<u32>, proposed: settings::AudioRemovalSetting) -> Option<u32> {
    let settings::AudioRemovalSetting::AfterDays { days } = proposed else {
        return None;
    };
    audio_removal::removes_more(before, Some(days)).then_some(days)
}

/// How many notes a pass would remove the audio of now if the setting
/// changed to `removal` (R17, KTD11). The count is 0 when the change does
/// not remove more audio at once. The count uses the same per-note check as
/// the pass and reads file metadata only. The pass checks each note again
/// after the confirmation. A number of days outside the range is refused.
#[tauri::command]
#[specta::specta]
pub async fn audio_removal_count(
    app: AppHandle,
    removal: settings::AudioRemovalSetting,
) -> Result<u32, String> {
    removal.validate()?;
    // The count reads the shared settings file and folders that can be on a
    // slow volume. The count starts from the file, not from the cache: the
    // cache can be older than a change that another computer made, and the
    // save compares with the file.
    let count = tauri::async_runtime::spawn_blocking(move || {
        let Some(days) = count_delay(read_removal_delay(&app), removal) else {
            return 0;
        };
        app.try_state::<Arc<AudioRemovalManager>>()
            .map_or(0, |pass| pass.count_eligible(days))
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

/// Choose whether the app has no Dock icon while no window is visible
/// (R2, R4). Off applies at once: the Dock icon comes back. On applies at
/// the next close of the last visible window, which reads the new value
/// (KTD3). The Dock is macOS only, so other platforms refuse.
#[tauri::command]
#[specta::specta]
pub fn set_hide_from_dock(app: AppHandle, enabled: bool) -> Result<Settings, String> {
    if !cfg!(target_os = "macos") {
        return Err("Hide from Dock is available only on macOS.".to_string());
    }
    let updated = settings::update_settings(&app, |s| {
        s.hide_from_dock = enabled;
        Ok(())
    })?;
    #[cfg(target_os = "macos")]
    if !enabled {
        crate::window::set_dock_policy(&app, crate::window::DockPolicy::Regular);
    }
    Ok(updated)
}

/// The built-in summary prompts first, then the custom prompts (R1, R2).
#[tauri::command]
#[specta::specta]
pub fn summary_prompts(app: AppHandle) -> SummaryPromptList {
    settings::get_settings(&app).summary_prompt_list()
}

// The prompt commands change the files of the storage folder through the
// shared settings manager (R14). They run off the main thread, because a
// read of a synced folder can wait for the sync service.

/// Add a summary prompt named "New prompt" with the Topic summary text (R3).
#[tauri::command(async)]
#[specta::specta]
pub fn create_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
) -> Result<Settings, String> {
    shared.create_prompt()?;
    Ok(settings::get_settings(&app))
}

/// Add a copy of a summary prompt, named "X copy" (R3).
#[tauri::command(async)]
#[specta::specta]
pub fn duplicate_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
) -> Result<Settings, String> {
    shared.duplicate_prompt(&id)?;
    Ok(settings::get_settings(&app))
}

#[tauri::command(async)]
#[specta::specta]
pub fn rename_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
    name: String,
) -> Result<Settings, String> {
    shared.rename_prompt(&id, &name)?;
    Ok(settings::get_settings(&app))
}

#[tauri::command(async)]
#[specta::specta]
pub fn set_summary_prompt_text(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
    text: String,
) -> Result<Settings, String> {
    shared.set_prompt_text(&id, &text)?;
    Ok(settings::get_settings(&app))
}

/// Save the title text of a custom summary prompt. It can be empty (R4).
#[tauri::command(async)]
#[specta::specta]
pub fn set_summary_prompt_title_text(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
    text: String,
) -> Result<Settings, String> {
    shared.set_prompt_title_text(&id, &text)?;
    Ok(settings::get_settings(&app))
}

/// Delete a custom summary prompt. The Topic summary becomes active when the
/// deleted prompt was active (R4).
#[tauri::command(async)]
#[specta::specta]
pub fn delete_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
) -> Result<Settings, String> {
    shared.delete_prompt(&id)?;
    Ok(settings::get_settings(&app))
}

/// Make a summary prompt the one that the next summary uses (R1).
#[tauri::command(async)]
#[specta::specta]
pub fn activate_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
) -> Result<Settings, String> {
    shared.activate_prompt(&id)?;
    Ok(settings::get_settings(&app))
}

/// Save the Background text that each summary uses (R4, R5). An empty
/// text is a valid Background.
#[tauri::command(async)]
#[specta::specta]
pub fn set_background(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    text: String,
) -> Result<Settings, String> {
    shared.set_background(&text)?;
    Ok(settings::get_settings(&app))
}

/// Pick a folder with the system dialog and return its path. Nothing
/// changes: the surface then asks what to do with the notes in the old
/// folder, and calls [`change_storage_folder`] (R4). `None` when the user
/// closes the dialog.
#[tauri::command]
#[specta::specta]
pub async fn pick_storage_folder(app: AppHandle) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose where notes are saved")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// What a change of storage folder does with the notes in the old folder
/// (R4).
#[derive(Deserialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OldNotes {
    /// Move every note into the new folder (R5).
    Move,
    /// Keep the notes in the old folder. The library forgets them (R7).
    Leave,
}

/// The checks before a change of storage folder from `current` to `folder`
/// (KTD11): no Move runs or is paused, and the new folder can be the
/// storage folder. The error is the cause, as Settings shows it. Returns
/// the new folder.
fn check_change(current: &str, folder: &str, move_open: bool) -> Result<PathBuf, String> {
    if move_open {
        return Err(crate::managers::processing::MOVE_OPEN.into());
    }
    let folder = PathBuf::from(folder);
    crate::managers::destination::check_new_folder(Path::new(current), &folder)?;
    Ok(folder)
}

/// Make `folder` the storage folder (R4, R21). The shared settings of the
/// new folder apply, and this computer's custom prompts merge into them,
/// or this computer's shared settings go into a folder that has none
/// (KTD8). Notes that wait for a folder try the new one, and the library
/// watches the new folder. With Move, the Move journal opens before the
/// folder changes, and the Move puts each note in the new folder while the
/// library lists it from the old folder until it moves (R5, R10). With
/// Leave them, the library forgets the notes of the old folder and keeps
/// their files; a busy note is forgotten when it is free (R7). A note that
/// records during the change saves to the new folder.
#[tauri::command]
#[specta::specta]
pub async fn change_storage_folder(
    app: AppHandle,
    folder: String,
    old_notes: OldNotes,
) -> Result<Settings, String> {
    let worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || change_folder(&worker, &folder, old_notes))
        .await
        .map_err(|e| e.to_string())??;
    Ok(settings::get_settings(&app))
}

fn change_folder(app: &AppHandle, folder: &str, old_notes: OldNotes) -> Result<(), String> {
    // One change at a time, so two changes never join two folders.
    static CHANGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = CHANGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let processing = app.try_state::<Arc<ProcessingManager>>();
    let current = settings::get_settings(app).storage_folder;
    let move_open = processing.as_ref().is_some_and(|p| p.move_open());
    let new = check_change(&current, folder, move_open)?;
    let shared = app.try_state::<Arc<SharedSettingsManager>>();
    if let Some(shared) = &shared {
        shared.change_folder(Some(new.clone()));
    }
    // Give the shared settings back to the storage folder that the
    // settings hold.
    let roll_back_shared = || {
        if let Some(shared) = &shared {
            shared.change_folder(settings::get_settings(app).storage_folder());
        }
    };
    // The journal opens before the folder changes, so the library keeps
    // the notes of the old folder in the meantime (KTD10).
    let moving = old_notes == OldNotes::Move;
    if moving && let Some(processing) = &processing {
        let opened = processing.open_move(Path::new(&current), &new);
        if let Err(e) = opened {
            roll_back_shared();
            return Err(e);
        }
    }
    let set = settings::update_settings(app, |s| {
        s.storage_folder = folder.to_string();
        Ok(())
    });
    if let Err(e) = set {
        // The shared settings stay with the storage folder, and no note
        // has moved.
        roll_back_shared();
        if moving && let Some(processing) = &processing {
            processing.close_move();
        }
        return Err(e);
    }
    log::info!("the storage folder changed");
    if let Some(processing) = &processing {
        processing.retry_awaiting();
        processing.forget_outside_roots();
        processing.roots_changed();
        if moving {
            processing.start_move();
        }
    }
    // The shared settings of the new folder can choose another recognition
    // configuration (R18).
    if let Some(models) = app.try_state::<Arc<ModelsManager>>() {
        models.follow_choice(app);
    }
    settings::emit_settings_changed(app);
    Ok(())
}

/// The Move of the notes to the storage folder, or `None` when no Move
/// runs or is paused.
#[tauri::command]
#[specta::specta]
pub fn move_status(processing: State<'_, Arc<ProcessingManager>>) -> Option<MoveStatus> {
    processing.move_status()
}

/// The cause that Show in Finder gives when the storage folder is not
/// available.
const STORAGE_NOT_AVAILABLE: &str = "The storage folder is not available.";

/// The storage folder that Show in Finder opens, or the cause when it is
/// not a folder on disk now.
fn folder_to_show(folder: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(folder);
    if folder.is_empty() || !path.is_dir() {
        return Err(STORAGE_NOT_AVAILABLE.into());
    }
    Ok(path)
}

/// Open the storage folder in Finder, or in Explorer on Windows (R3,
/// KTD17). The command opens the folder from Rust, so the Settings window
/// gets no file path permission.
#[tauri::command]
#[specta::specta]
pub fn show_storage_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let folder = folder_to_show(&settings::get_settings(&app).storage_folder)?;
    app.opener()
        .open_path(folder.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("The storage folder could not open: {e}"))
}

/// Continue a paused Move (R10, AE13).
#[tauri::command]
#[specta::specta]
pub fn retry_move(processing: State<'_, Arc<ProcessingManager>>) -> Result<(), String> {
    processing.retry_move()
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

    /// R17: only a change that removes more audio at once gets a count.
    #[test]
    fn the_count_uses_the_new_delay_only_when_the_change_removes_more() {
        use settings::AudioRemovalSetting::{AfterDays, Off};
        // A shorter delay, and the setting on again.
        assert_eq!(count_delay(Some(90), AfterDays { days: 30 }), Some(30));
        assert_eq!(count_delay(None, AfterDays { days: 30 }), Some(30));
        // A longer delay, the same delay, and the setting off.
        assert_eq!(count_delay(Some(30), AfterDays { days: 90 }), None);
        assert_eq!(count_delay(Some(30), AfterDays { days: 30 }), None);
        assert_eq!(count_delay(Some(30), Off), None);
        assert_eq!(count_delay(None, Off), None);
    }

    /// R17: the count starts from the file, not from an older cache. The
    /// cache of computer A says 30 days, but computer B changed the file to
    /// 90 days. A change to 60 days then shortens the delay and gets a
    /// count.
    #[test]
    fn the_count_starts_from_the_file_when_the_cache_is_older() {
        use shared_settings::file::{AudioRemoval, SharedValue, settings_path};
        let folder = tempfile::tempdir().unwrap();
        let (cache_a, cache_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = Some(folder.path().to_path_buf());
        let a = SharedSettingsManager::new(cache_a.path().to_path_buf(), root.clone());
        a.join_at_launch();
        let b = SharedSettingsManager::new(cache_b.path().to_path_buf(), root);
        b.join_at_launch();
        b.save(&[SharedValue::AudioRemoval(AudioRemoval::AfterDays(90))])
            .unwrap();
        assert_eq!(a.snapshot().audio_removal_days(), Some(30));
        let proposed = settings::AudioRemovalSetting::AfterDays { days: 60 };
        assert_eq!(
            count_delay(a.snapshot().audio_removal_days(), proposed),
            None
        );

        let file = std::fs::read(settings_path(folder.path())).unwrap();
        assert_eq!(reread_removal_delay(&a), (true, Some(90)));
        assert_eq!(count_delay(Some(90), proposed), Some(60));
        // The read wrote nothing (rule 1), and a second read has no change.
        assert_eq!(std::fs::read(settings_path(folder.path())).unwrap(), file);
        assert_eq!(reread_removal_delay(&a), (false, Some(90)));
    }

    /// KTD11: Change is not available while a Move runs or is paused.
    #[test]
    fn a_change_of_folder_is_refused_while_a_move_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let (current, new) = (dir.path().join("Notes"), dir.path().join("Disk"));
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        let (current, new) = (current.to_str().unwrap(), new.to_str().unwrap());
        assert_eq!(
            check_change(current, new, true),
            Err(crate::managers::processing::MOVE_OPEN.to_string())
        );
        assert_eq!(check_change(current, new, false), Ok(PathBuf::from(new)));
    }

    /// KTD17: Show in Finder opens only a storage folder that exists.
    #[test]
    fn show_in_finder_needs_a_storage_folder_that_is_available() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Notes");
        let not_available = Err(STORAGE_NOT_AVAILABLE.to_string());
        assert_eq!(folder_to_show(""), not_available);
        assert_eq!(folder_to_show(folder.to_str().unwrap()), not_available);
        std::fs::write(&folder, b"a file").unwrap();
        assert_eq!(folder_to_show(folder.to_str().unwrap()), not_available);
        std::fs::remove_file(&folder).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(folder_to_show(folder.to_str().unwrap()), Ok(folder));
    }

    #[test]
    fn a_change_of_folder_gives_the_cause_of_a_refused_folder() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("Notes");
        std::fs::create_dir_all(&current).unwrap();
        let current = current.to_str().unwrap();
        let cause = check_change(current, current, false).unwrap_err();
        assert!(cause.contains("already the storage folder"), "{cause}");
    }

    #[test]
    fn old_notes_read_as_snake_case() {
        assert_eq!(
            serde_json::from_value::<OldNotes>(serde_json::json!("leave")).unwrap(),
            OldNotes::Leave
        );
        assert_eq!(
            serde_json::from_value::<OldNotes>(serde_json::json!("move")).unwrap(),
            OldNotes::Move
        );
    }

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

/// Choose the complete recognition configuration (R18). The configuration
/// in use stays until every file of the chosen configuration is on disk.
#[tauri::command(async)]
#[specta::specta]
pub fn set_recognition_defaults(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    config: models::recognition::RecognitionConfig,
) -> Result<Settings, String> {
    models::recognition::resolve(&config).map_err(|e| e.to_string())?;
    let choice = settings::RecognitionChoice::from_config(&config);
    let result = choose_recognition(&app, &manager, |c| *c = choice)?;
    // The files of the chosen configuration download first. The rest of
    // the bundle follows (KTD5).
    manager.start_bundle_downloads(&app, &config.asr_model, &[])?;
    Ok(result)
}
