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

use super::blocking;
use crate::managers::agent_block::AgentBlockStatus;
use crate::managers::audio_removal::{self, AudioRemovalManager};
use crate::managers::capture::CaptureManager;
use crate::managers::models::ModelsManager;
use crate::managers::processing::{MoveStatus, ProcessingManager};
use crate::managers::secrets::SecretsManager;
use crate::managers::settings::{
    self, KeyLocation, ListedApp, MicrophoneChoice, Settings, SpeakerCountSetting,
    SummaryPromptList,
};
use crate::managers::shared::{NoteTypeList, SharedSettingsManager, StorageFolders};
use crate::managers::{destination, watched_folder};
use summarize::Provider;

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

/// Make `provider` the active AI provider, or choose none (R7). The entry
/// of each provider stays, so a switch back restores it (R10).
#[tauri::command]
#[specta::specta]
pub fn choose_provider(app: AppHandle, provider: Option<Provider>) -> Result<Settings, String> {
    settings::update_settings(&app, |s| {
        s.choose_provider(provider);
        Ok(())
    })
}

/// Save the address of a local provider or of Custom (R8, R9). An empty
/// value gives a local provider its usual address again, and gives Custom
/// no address. Any other value must be a URL with a host, because the key
/// is kept for that host (R10).
#[tauri::command]
#[specta::specta]
pub fn set_provider_address(
    app: AppHandle,
    provider: Provider,
    address: String,
) -> Result<Settings, String> {
    settings::update_settings(&app, |s| s.set_provider_address(provider, &address))
}

/// Save the model of `provider`.
#[tauri::command]
#[specta::specta]
pub fn set_provider_model(
    app: AppHandle,
    provider: Provider,
    model: String,
) -> Result<Settings, String> {
    settings::update_settings(&app, |s| {
        s.set_provider_model(provider, &model);
        Ok(())
    })
}

/// Save the context limit that the user entered for `provider`, or `None`
/// to use the limit that the provider reports (R15).
#[tauri::command]
#[specta::specta]
pub fn set_provider_context_limit(
    app: AppHandle,
    provider: Provider,
    limit: Option<u32>,
) -> Result<Settings, String> {
    settings::update_settings(&app, |s| s.set_provider_context_limit(provider, limit))
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

#[tauri::command]
#[specta::specta]
pub async fn set_default_speaker_count(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    count: Option<SpeakerCountSetting>,
) -> Result<Settings, String> {
    if let Some(count) = &count {
        count.validate()?;
    }
    let manager = Arc::clone(&manager);
    blocking(move || choose_recognition(&app, &manager, |c| c.default_speaker_count = count)).await
}

/// Choose the recognition model (R5, R18). The model in use stays until
/// every file of the chosen configuration is on disk.
#[tauri::command]
#[specta::specta]
pub async fn select_asr_model(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    model_id: String,
) -> Result<Settings, String> {
    match models::model(&model_id) {
        Some(m) if m.kind == models::ModelKind::Asr => {}
        _ => return Err(format!("'{model_id}' is not a recognition model")),
    }
    let manager = Arc::clone(&manager);
    blocking(move || choose_recognition(&app, &manager, |c| c.asr_model = model_id)).await
}

/// Turn the live transcript of streaming models on or off (R18, KD9).
#[tauri::command]
#[specta::specta]
pub async fn set_live_transcript(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    enabled: bool,
) -> Result<Settings, String> {
    let manager = Arc::clone(&manager);
    blocking(move || choose_recognition(&app, &manager, |c| c.live_transcript = enabled)).await
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

/// Why the app list cannot change on this computer (R17).
pub const APP_AUDIO_UNAVAILABLE: &str = "App audio needs Windows 11.";

fn app_audio_supported(supported: bool) -> Result<(), String> {
    if supported {
        Ok(())
    } else {
        Err(APP_AUDIO_UNAVAILABLE.to_string())
    }
}

/// Whether this computer can record app audio (R17): macOS, or Windows 11.
#[tauri::command]
#[specta::specta]
pub async fn app_audio_available() -> Result<bool, String> {
    blocking(|| Ok(audio_system::apps::supported())).await
}

/// One running app with a window, for the add menu in Settings and the
/// App Audio submenu of a note (R2, R3, R10).
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct RunningApp {
    pub id: String,
    pub name: String,
    pub path: String,
    /// "Add Call Apps" adds this app when it is not on the list (R3).
    pub call_app: bool,
}

/// The running apps with a window, by name (R2, R10, KTD12). With `all`
/// false, the apps on the app list are left out. The list is read again
/// at each call.
#[tauri::command]
#[specta::specta]
pub async fn list_running_apps(app: AppHandle, all: bool) -> Result<Vec<RunningApp>, String> {
    blocking(move || {
        let listed = if all {
            Vec::new()
        } else {
            settings::get_settings(&app).apps
        };
        Ok(not_listed(audio_system::apps::running_apps(), &listed))
    })
    .await
}

fn not_listed(running: Vec<audio_system::apps::App>, listed: &[ListedApp]) -> Vec<RunningApp> {
    running
        .into_iter()
        .filter(|r| !listed.iter().any(|l| l.id == r.id))
        .map(|r| RunningApp {
            call_app: audio_system::apps::is_call_app(&r.id),
            id: r.id,
            name: r.name,
            path: r.path,
        })
        .collect()
}

/// Add one app to the app list (R2). On macOS, the first add asks for the
/// system audio permission (R15, KTD11).
#[tauri::command]
#[specta::specta]
pub async fn add_app(app: AppHandle, entry: ListedApp) -> Result<Settings, String> {
    blocking(move || {
        app_audio_supported(audio_system::apps::supported())?;
        let updated = settings::update_settings(&app, |s| s.add_app(entry))?;
        request_permission_if_not_asked();
        Ok(updated)
    })
    .await
}

/// Add the common call apps that run now and are not on the app list (R3,
/// KTD13). It adds nothing when none runs, and asks for the permission
/// once.
#[tauri::command]
#[specta::specta]
pub async fn add_call_apps(app: AppHandle) -> Result<Settings, String> {
    blocking(move || {
        app_audio_supported(audio_system::apps::supported())?;
        let running = audio_system::apps::running_apps();
        let updated = settings::update_settings(&app, |s| {
            let listed: Vec<audio_system::apps::App> = s.apps.iter().map(Into::into).collect();
            for found in audio_system::apps::call_apps_to_add(&running, &listed) {
                s.add_app(found.into())?;
            }
            Ok(())
        })?;
        request_permission_if_not_asked();
        Ok(updated)
    })
    .await
}

/// Remove an app from the app list (R4). The app set of an open note does
/// not change (R11).
#[tauri::command]
#[specta::specta]
pub async fn remove_app(app: AppHandle, id: String) -> Result<Settings, String> {
    blocking(move || {
        settings::update_settings(&app, |s| {
            s.remove_app(&id);
            Ok(())
        })
    })
    .await
}

/// The icon of the app at `path` as a PNG data URL, or `None` when it
/// cannot be read. The UI then shows a generic icon (R5).
#[tauri::command]
#[specta::specta]
pub async fn app_icon(path: String) -> Result<Option<String>, String> {
    use base64::Engine as _;
    blocking(move || {
        Ok(audio_system::apps::icon_png(&path).map(|png| {
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(png)
            )
        }))
    })
    .await
}

/// On macOS, start the permission request when macOS has not asked the
/// user yet (R15, KTD11). The request runs on its own thread, so the add
/// returns at once.
pub fn request_permission_if_not_asked() {
    #[cfg(target_os = "macos")]
    std::thread::spawn(|| {
        if audio_system::permission::preflight() == audio_system::Permission::NotAsked {
            log::info!("the first app is on the list; asking for the system audio permission");
            audio_system::permission::request();
        }
    });
}

/// Choose what New Note and the recording shortcut do (R8): start a note
/// that records, or open a note that does not record. The value goes into
/// the shared settings file.
#[tauri::command]
#[specta::specta]
pub async fn set_new_note_records(app: AppHandle, enabled: bool) -> Result<Settings, String> {
    blocking(move || {
        settings::update_settings(&app, |s| {
            s.new_note_records = enabled;
            Ok(())
        })
    })
    .await
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

/// Choose whether a new note shows the recording widget (R11). The next
/// note reads the new value (R14). The value goes into the shared settings
/// file.
#[tauri::command]
#[specta::specta]
pub async fn set_show_recording_widget(app: AppHandle, enabled: bool) -> Result<Settings, String> {
    blocking(move || {
        settings::update_settings(&app, |s| {
            s.show_recording_widget = enabled;
            Ok(())
        })
    })
    .await
}

/// Choose whether the app checks for updates by itself (R10). The next
/// scheduled check reads the new value.
#[tauri::command]
#[specta::specta]
pub async fn set_check_updates_automatically(
    app: AppHandle,
    enabled: bool,
) -> Result<Settings, String> {
    blocking(move || {
        settings::update_settings(&app, |s| {
            s.check_updates_automatically = enabled;
            Ok(())
        })
    })
    .await
}

/// Choose when the audio of a saved note goes, or turn automatic removal
/// off (R1, R3). The value goes into the shared settings file, so it
/// applies on every computer that uses the storage folder. A number of
/// days outside the range is refused and changes nothing. After a saved
/// change that turns the setting on or shortens the delay, a removal pass
/// starts (R17, KTD6). The Settings window asks for the confirmation first.
#[tauri::command]
#[specta::specta]
pub async fn set_audio_removal(
    app: AppHandle,
    removal: settings::AudioRemovalSetting,
) -> Result<Settings, String> {
    removal.validate()?;
    blocking(move || {
        // The comparison starts from the file, not from the cache: the cache
        // can be older than a change that another computer made.
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
    })
    .await
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

/// Run one change of the shared settings and give the settings after it.
/// The prompt commands change the files of the storage folder through the
/// shared settings manager (R14), so the change runs on a blocking thread:
/// a read of a synced folder can wait for the sync service.
async fn shared_change<T>(
    app: AppHandle,
    shared: &Arc<SharedSettingsManager>,
    change: impl FnOnce(&SharedSettingsManager) -> Result<T, String> + Send + 'static,
) -> Result<Settings, String> {
    let shared = Arc::clone(shared);
    blocking(move || {
        change(&shared)?;
        Ok(settings::get_settings(&app))
    })
    .await
}

/// Add a summary prompt named "New prompt" with the Topic summary text (R3).
#[tauri::command]
#[specta::specta]
pub async fn create_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
) -> Result<Settings, String> {
    shared_change(app, &shared, move |shared| shared.create_prompt()).await
}

/// The prompt that `restore_summary_prompt` added, and the settings after it.
#[derive(Serialize, Type)]
pub struct RestoredPrompt {
    pub id: String,
    pub settings: Settings,
}

/// Add a summary prompt with a name and texts, in one write. The Settings
/// window keeps the unsaved texts of a prompt that another computer deleted
/// with it.
#[tauri::command]
#[specta::specta]
pub async fn restore_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    name: String,
    text: String,
    title_text: String,
) -> Result<RestoredPrompt, String> {
    let shared = Arc::clone(&shared);
    blocking(move || {
        let id = shared.restore_prompt(&name, &text, &title_text)?;
        Ok(RestoredPrompt {
            id,
            settings: settings::get_settings(&app),
        })
    })
    .await
}

/// Add a copy of a summary prompt, named "X copy" (R3).
#[tauri::command]
#[specta::specta]
pub async fn duplicate_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
) -> Result<Settings, String> {
    shared_change(app, &shared, move |shared| shared.duplicate_prompt(&id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
    name: String,
) -> Result<Settings, String> {
    shared_change(app, &shared, move |shared| shared.rename_prompt(&id, &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_summary_prompt_text(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
    text: String,
) -> Result<Settings, String> {
    shared_change(app, &shared, move |shared| {
        shared.set_prompt_text(&id, &text)
    })
    .await
}

/// Save the title text of a custom summary prompt. It can be empty (R4).
#[tauri::command]
#[specta::specta]
pub async fn set_summary_prompt_title_text(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
    text: String,
) -> Result<Settings, String> {
    shared_change(app, &shared, move |shared| {
        shared.set_prompt_title_text(&id, &text)
    })
    .await
}

/// Delete a custom summary prompt. The Topic summary becomes active when the
/// deleted prompt was active (R4).
#[tauri::command]
#[specta::specta]
pub async fn delete_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
) -> Result<Settings, String> {
    shared_change(app, &shared, move |shared| shared.delete_prompt(&id)).await
}

/// Make a summary prompt the one that the next summary uses (R1).
#[tauri::command]
#[specta::specta]
pub async fn activate_summary_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    id: String,
) -> Result<Settings, String> {
    shared_change(app, &shared, move |shared| shared.activate_prompt(&id)).await
}

/// Save the Background text that each summary uses (R4, R5). An empty
/// text is a valid Background.
#[tauri::command]
#[specta::specta]
pub async fn set_background(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    text: String,
) -> Result<Settings, String> {
    shared_change(app, &shared, move |shared| shared.set_background(&text)).await
}

/// The state of the agent instructions in `AGENTS.md` and `CLAUDE.md` of
/// the storage folder (R29). This writes nothing.
#[tauri::command]
#[specta::specta]
pub async fn agent_instructions_status(app: AppHandle) -> AgentBlockStatus {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<SharedSettingsManager>>()
            .agent_block_status()
    })
    .await
    .unwrap_or_else(|e| AgentBlockStatus::Unavailable {
        reason: e.to_string(),
    })
}

/// Add the agent instructions to `AGENTS.md` and `CLAUDE.md` of the storage
/// folder (R29). The error names the file that Longhand could not write.
#[tauri::command]
#[specta::specta]
pub async fn add_agent_instructions(
    shared: State<'_, Arc<SharedSettingsManager>>,
) -> Result<AgentBlockStatus, String> {
    let shared = Arc::clone(&shared);
    blocking(move || shared.add_agent_block()).await
}

/// The type folders and the home folders of the storage folder, from the
/// store rows that the library scan keeps (KTD4, KTD9). The old folder of a
/// type rename, merge or delete that is not done is leaving (KTD7). This
/// reads no folder.
pub(crate) fn storage_folders(app: &AppHandle) -> Result<StorageFolders, String> {
    let Some(root) = settings::get_settings(app).storage_folder() else {
        return Ok(StorageFolders::default());
    };
    let Some(capture) = app.try_state::<Arc<CaptureManager>>() else {
        return Ok(StorageFolders::default());
    };
    let store = capture.shared_store();
    let store = pipeline::lock(&store);
    let records = store.sessions().map_err(|e| e.to_string())?;
    drop(store);
    let mut folders = StorageFolders::from_records(&root, &records);
    if let Some(processing) = app.try_state::<Arc<ProcessingManager>>() {
        folders.mark_leaving(processing.leaving_types().iter().map(String::as_str));
    }
    Ok(folders)
}

/// The note types and the default type (R2, R7, R34, KTD9): the shared
/// list plus each type folder that holds a note. This never writes.
#[tauri::command]
#[specta::specta]
pub async fn note_types(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
) -> Result<NoteTypeList, String> {
    let shared = Arc::clone(&shared);
    blocking(move || Ok(shared.note_types(&storage_folders(&app)?))).await
}

/// Add a note type to the shared list (R3). The name goes through the type
/// name check (R28), and a name that the list has is refused.
#[tauri::command]
#[specta::specta]
pub async fn add_note_type(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    name: String,
) -> Result<NoteTypeList, String> {
    let shared = Arc::clone(&shared);
    blocking(move || {
        let folders = storage_folders(&app)?;
        shared.add_note_type(&name, &folders)?;
        settings::emit_settings_changed(&app);
        Ok(shared.note_types(&folders))
    })
    .await
}

/// Make a note type the default type, or General for `None` (R7, R23).
#[tauri::command]
#[specta::specta]
pub async fn set_default_note_type(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    name: Option<String>,
) -> Result<NoteTypeList, String> {
    let shared = Arc::clone(&shared);
    blocking(move || {
        let folders = storage_folders(&app)?;
        shared.set_default_note_type(name.as_deref(), &folders)?;
        settings::emit_settings_changed(&app);
        Ok(shared.note_types(&folders))
    })
    .await
}

/// Choose the summary prompt of a note type, or "Default" for `None` (R1,
/// R2). The name "General" is General.
#[tauri::command]
#[specta::specta]
pub async fn set_note_type_prompt(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    name: String,
    prompt_id: Option<String>,
) -> Result<NoteTypeList, String> {
    let shared = Arc::clone(&shared);
    blocking(move || {
        let folders = storage_folders(&app)?;
        let list = shared.set_note_type_prompt(&name, prompt_id.as_deref(), &folders)?;
        settings::emit_settings_changed(&app);
        Ok(list)
    })
    .await
}

/// What a type rename, merge or delete did (R6, R31).
#[derive(Serialize, Type)]
pub struct NoteTypeEdit {
    /// The number of notes that changed their type.
    pub changed: u32,
    /// The note types and the default type after the edit.
    pub list: NoteTypeList,
}

/// Rename a note type (R6). A new name of another type merges the two
/// types: the notes go into the folder of that type. The default type
/// follows the rename. The name goes through the type name check (R28).
/// The folder of each note moves now, or later when a computer works on
/// the note.
#[tauri::command]
#[specta::specta]
pub async fn rename_note_type(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    name: String,
    new_name: String,
) -> Result<NoteTypeEdit, String> {
    let (shared, processing) = (Arc::clone(&shared), Arc::clone(&processing));
    tauri::async_runtime::spawn_blocking(move || {
        let folders = storage_folders(&app)?;
        let changed = processing.rename_note_type(&shared, &folders, &name, &new_name)?;
        Ok(NoteTypeEdit {
            changed,
            list: shared.note_types(&storage_folders(&app)?),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Delete a note type (R6): its notes get the type General, and their
/// folders go directly in the storage folder. When it is the default type,
/// the default becomes General (R7). The surface asks for a confirmation
/// with the number of notes first (R31).
#[tauri::command]
#[specta::specta]
pub async fn delete_note_type(
    app: AppHandle,
    shared: State<'_, Arc<SharedSettingsManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    name: String,
) -> Result<NoteTypeEdit, String> {
    let (shared, processing) = (Arc::clone(&shared), Arc::clone(&processing));
    tauri::async_runtime::spawn_blocking(move || {
        let folders = storage_folders(&app)?;
        let changed = processing.delete_note_type(&shared, &folders, &name)?;
        Ok(NoteTypeEdit {
            changed,
            list: shared.note_types(&storage_folders(&app)?),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What a tag rename or delete did (R13, R31).
#[derive(Serialize, Type)]
pub struct TagEdit {
    /// The number of notes that had the tag.
    pub changed: u32,
    /// The tag list after the edit.
    pub tags: Vec<crate::commands::session::TagEntry>,
}

/// Rename the tag `name` to `new_name` on every note (R13), also on a note
/// that another computer records or processes. A new name with the key of
/// another tag merges the two tags, and a note with both keeps one. A note
/// whose folder is not available now changes later.
#[tauri::command]
#[specta::specta]
pub async fn rename_tag(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    name: String,
    new_name: String,
) -> Result<TagEdit, String> {
    let (store, processing) = (capture.shared_store(), Arc::clone(&processing));
    tauri::async_runtime::spawn_blocking(move || {
        let changed = processing.rename_tag(&name, &new_name)?;
        Ok(TagEdit {
            changed,
            tags: crate::commands::session::tag_entries(&store)?,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Delete the tag `name` from every note (R13). The surface asks for a
/// confirmation with the number of notes first (R31): the count of the tag
/// in the tag list.
#[tauri::command]
#[specta::specta]
pub async fn delete_tag(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    name: String,
) -> Result<TagEdit, String> {
    let (store, processing) = (capture.shared_store(), Arc::clone(&processing));
    tauri::async_runtime::spawn_blocking(move || {
        let changed = processing.delete_tag(&name)?;
        Ok(TagEdit {
            changed,
            tags: crate::commands::session::tag_entries(&store)?,
        })
    })
    .await
    .map_err(|e| e.to_string())?
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
/// the new folder. A new folder that touches the watched folder is not
/// refused: the change turns the watched folder off (R9, U7).
fn check_change(current: &str, folder: &str, move_open: bool) -> Result<PathBuf, String> {
    if move_open {
        return Err(crate::managers::processing::MOVE_OPEN.into());
    }
    let folder = PathBuf::from(folder);
    crate::managers::destination::check_new_folder(Path::new(current), &folder)?;
    Ok(folder)
}

/// Make `folder` the storage folder in `s`. When the watched folder and
/// the new storage folder touch (R9), the watched folder turns off in the
/// same write, and the result is the reason, as Settings shows it (U7).
fn change_settings(s: &mut Settings, folder: &str) -> Option<String> {
    s.storage_folder = folder.to_string();
    let off = s.watched_folder.as_deref().and_then(|watched| {
        destination::watched_folder_turned_off(Path::new(folder), Path::new(watched))
    });
    if off.is_some() {
        s.watched_folder = None;
    }
    off
}

/// The question after Leave Them (R6, U7): the count and the size of the
/// audio files in `watched` that the import record of `storage` does not
/// hold by name and size, and the files to hold from import until the
/// answer. `None` when no file needs the question, or when the folder
/// cannot be listed now (the log names the cause).
fn files_to_ask(
    watched: &Path,
    storage: &Path,
) -> Option<(
    watched_folder::WatchedFolderPreview,
    Vec<watched_folder::Held>,
)> {
    let listed = match watched_folder::list(watched) {
        Ok(listed) => listed,
        Err(e) => {
            log::warn!("cannot list the watched folder after the storage folder change: {e}");
            return None;
        }
    };
    let preview = watched_folder::preview(&listed, Some(storage));
    (preview.count > 0).then(|| {
        (
            preview,
            listed.iter().map(watched_folder::Held::from).collect(),
        )
    })
}

/// What a change of storage folder did (R4, U7).
#[derive(Serialize, Type, Debug, Clone)]
pub struct StorageFolderChange {
    pub settings: Settings,
    /// The watched folder touched the new storage folder, so the change
    /// turned it off (R9). The reason, as Settings shows it.
    pub watched_folder_off: Option<String>,
    /// Leave Them: the watched folder holds audio files that the import
    /// record of the new storage folder does not hold. The watcher holds
    /// them until the surface asks the question of R6 and calls
    /// [`set_watched_folder`] with the answer.
    pub watched_folder_files: Option<watched_folder::WatchedFolderPreview>,
}

/// The cause that Settings shows when a change with Leave them waits for a
/// note that is saved now.
const NOTE_SAVING: &str =
    "A note is being saved to the current storage folder. Try again in a moment.";

/// The check before a change with Leave them from the storage folder
/// `current`: no early write of a note that is not saved yet is between
/// its staging folder and its rename in `current`, and no first Save into
/// `current` runs now (`running`). These steps end in a few seconds. After
/// the change, a note that has had no first save gets a new home folder in
/// the new storage folder.
///
/// A first Save that began and does not run now (it stopped, failed or
/// waits for its folder) does not stop the change. When it continues and
/// its publication has not begun, the note gets a new home folder in the
/// new storage folder. Known limit: a publication that has begun finishes
/// in its home folder in `current`.
fn check_leave(
    store: &pipeline::SharedStore,
    current: &str,
    running: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    match pipeline::early_home::saving_into(store, Path::new(current), running) {
        Ok(false) => Ok(()),
        Ok(true) => Err(NOTE_SAVING.into()),
        Err(e) => Err(e.to_string()),
    }
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
///
/// The watcher of the watched folder pauses during the change (U7). With
/// Move, it stays paused until the Move has taken the import record into
/// the new folder. A watched folder that touches the new storage folder
/// turns off (R9). With Leave Them, the watcher holds the audio files that
/// the new folder's import record does not hold until the R6 answer.
#[tauri::command]
#[specta::specta]
pub async fn change_storage_folder(
    app: AppHandle,
    folder: String,
    old_notes: OldNotes,
) -> Result<StorageFolderChange, String> {
    tauri::async_runtime::spawn_blocking(move || change_folder(&app, &folder, old_notes))
        .await
        .map_err(|e| e.to_string())?
}

fn change_folder(
    app: &AppHandle,
    folder: &str,
    old_notes: OldNotes,
) -> Result<StorageFolderChange, String> {
    // One change at a time, so two changes never join two folders.
    static CHANGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = CHANGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let processing = app.try_state::<Arc<ProcessingManager>>();
    let before = settings::get_settings(app);
    let current = before.storage_folder;
    let move_open = processing.as_ref().is_some_and(|p| p.move_open());
    let new = check_change(&current, folder, move_open)?;
    let moving = old_notes == OldNotes::Move;
    if !moving && let Some(processing) = &processing {
        check_leave(processing.shared_store(), &current, &|id| {
            processing.runs(id)
        })?;
    }
    // No import reads the import record while the storage folder changes.
    if let Some(processing) = &processing {
        processing.pause_watched_folder();
    }
    let changed = apply_change(app, processing.as_deref(), &current, folder, &new, moving);
    // R6 after Leave Them: the new folder's import record can hold none of
    // the files in the watched folder.
    let mut asked = None;
    if matches!(changed, Ok(None))
        && !moving
        && let Some(watched) = settings::get_settings(app).watched_folder
    {
        asked = files_to_ask(Path::new(&watched), &new);
    }
    let (preview, held) = asked.unzip();
    if let Some(processing) = &processing {
        processing.watched_folder_change_done(held);
    }
    Ok(StorageFolderChange {
        watched_folder_off: changed?,
        settings: settings::get_settings(app),
        watched_folder_files: preview,
    })
}

/// The change of [`change_folder`] after its checks, while the watcher is
/// paused. Returns the reason when the change turned the watched folder
/// off (R9).
fn apply_change(
    app: &AppHandle,
    processing: Option<&Arc<ProcessingManager>>,
    current: &str,
    folder: &str,
    new: &Path,
    moving: bool,
) -> Result<Option<String>, String> {
    let shared = app.try_state::<Arc<SharedSettingsManager>>();
    if let Some(shared) = &shared {
        shared.change_folder(Some(new.to_path_buf()));
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
    if moving && let Some(processing) = processing {
        let opened = processing.open_move(Path::new(current), new);
        if let Err(e) = opened {
            roll_back_shared();
            return Err(e);
        }
    }
    let mut off = None;
    let set = settings::update_settings(app, |s| {
        off = change_settings(s, folder);
        Ok(())
    });
    if let Err(e) = set {
        // The shared settings stay with the storage folder, and no note
        // has moved.
        roll_back_shared();
        if moving && let Some(processing) = processing {
            processing.close_move();
        }
        return Err(e);
    }
    log::info!("the storage folder changed");
    if off.is_some() {
        log::info!("the watched folder touches the new storage folder, so it is off");
    }
    if let Some(processing) = processing {
        if off.is_some() {
            processing.watched_folder_changed(Vec::new());
        }
        // Before the forget: the retry first gives each note that has had
        // no first save a new home folder in the new storage folder, so its
        // first Save publishes there and the library keeps it. With Move,
        // the open Move handles these notes.
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
    Ok(off)
}

/// Pick the watched folder with the system dialog and return its path
/// (R4). Nothing changes: the surface reads the preview, asks about the
/// files in the folder, and calls [`set_watched_folder`] (F3). `None` when
/// the user closes the dialog.
#[tauri::command]
#[specta::specta]
pub async fn pick_watched_folder(app: AppHandle) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a folder for new recordings")
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

/// Whether `folder` can be the watched folder of this computer (R9). The
/// error is the cause, as Settings shows it.
fn check_watched(app: &AppHandle, folder: &Path) -> Result<(), String> {
    let storage = settings::get_settings(app).storage_folder;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let home = app.path().home_dir().ok();
    destination::check_watched_folder(
        folder,
        &destination::WatchLimits {
            storage: Path::new(&storage),
            app_data: &app_data,
            home: home.as_deref(),
        },
    )
}

/// The audio files of `folder`, or the cause, as Settings shows it, when
/// the folder cannot be read.
fn list_watched(folder: &Path) -> Result<Vec<watched_folder::Listed>, String> {
    watched_folder::list(folder).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => {
            "Longhand doesn’t have permission to read this folder.".to_string()
        }
        _ => format!("Longhand can’t read this folder: {e}"),
    })
}

/// The count and the total size of the audio files in `folder` that the
/// import record does not hold by name and size (R6, KTD12). It reads only
/// the folder list, so no file downloads. Refused for a folder that cannot
/// be the watched folder (R9).
#[tauri::command]
#[specta::specta]
pub async fn watched_folder_preview(
    app: AppHandle,
    folder: String,
) -> Result<watched_folder::WatchedFolderPreview, String> {
    blocking(move || {
        let folder = PathBuf::from(folder);
        check_watched(&app, &folder)?;
        let listed = list_watched(&folder)?;
        let storage = settings::get_settings(&app).storage_folder();
        Ok(watched_folder::preview(&listed, storage.as_deref()))
    })
    .await
}

/// What the watched folder does with the audio files that are in it when
/// the user chooses it (R6).
#[derive(Deserialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExistingFiles {
    /// Each file that the import record does not hold becomes a note.
    Import,
    /// The files go into the import record as skipped. Only files that
    /// arrive later become notes.
    Skip,
}

/// Make `folder` the watched folder of this computer, or turn the watched
/// folder off with `None` (R4, R10). With [`ExistingFiles::Skip`], the
/// files in the folder now are held from import and recorded as skipped
/// in the background (R6, KTD12). Turning off makes no change to the notes
/// that the folder made.
#[tauri::command]
#[specta::specta]
pub async fn set_watched_folder(
    app: AppHandle,
    folder: Option<String>,
    existing: ExistingFiles,
) -> Result<Settings, String> {
    blocking(move || {
        let skip = match &folder {
            Some(folder) => {
                let path = PathBuf::from(folder);
                check_watched(&app, &path)?;
                match existing {
                    ExistingFiles::Skip => list_watched(&path)?
                        .iter()
                        .map(watched_folder::Held::from)
                        .collect(),
                    ExistingFiles::Import => Vec::new(),
                }
            }
            None => Vec::new(),
        };
        let updated = settings::update_settings(&app, |s| {
            s.watched_folder = folder.clone();
            Ok(())
        })?;
        if let Some(processing) = app.try_state::<Arc<ProcessingManager>>() {
            processing.watched_folder_changed(skip);
        }
        Ok(updated)
    })
    .await
}

/// The state of the watched folder: off, watched, or not available after
/// several scans (R4).
#[tauri::command]
#[specta::specta]
pub async fn watched_folder_status(
    app: AppHandle,
) -> Result<watched_folder::WatchedFolderStatus, String> {
    blocking(move || {
        Ok(match app.try_state::<Arc<ProcessingManager>>() {
            Some(processing) => processing.watched_folder_status(),
            None => match settings::get_settings(&app).watched_folder {
                Some(path) => watched_folder::WatchedFolderStatus::Watching { path },
                None => watched_folder::WatchedFolderStatus::Off,
            },
        })
    })
    .await
}

/// The Move of the notes to the storage folder, or `None` when no Move
/// runs or is paused.
#[tauri::command]
#[specta::specta]
pub async fn move_status(app: AppHandle) -> Option<MoveStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<ProcessingManager>>().move_status()
    })
    .await
    .unwrap_or_else(|e| {
        log::error!("could not read the Move status: {e}");
        None
    })
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
pub async fn show_storage_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    blocking(move || {
        let folder = folder_to_show(&settings::get_settings(&app).storage_folder)?;
        app.opener()
            .open_path(folder.to_string_lossy(), None::<&str>)
            .map_err(|e| format!("The storage folder could not open: {e}"))
    })
    .await
}

/// Continue a paused Move (R10, AE13).
#[tauri::command]
#[specta::specta]
pub async fn retry_move(processing: State<'_, Arc<ProcessingManager>>) -> Result<(), String> {
    let processing = Arc::clone(&processing);
    blocking(move || processing.retry_move()).await
}

/// Where the Keychain holds the key of `provider` with the settings now,
/// or why it has none: a local provider takes no key (R8), and Custom
/// needs an address first.
fn key_location(app: &AppHandle, provider: Provider) -> Result<KeyLocation, String> {
    settings::get_settings(app)
        .key_location(provider)
        .ok_or_else(|| match provider.takes_key() {
            true => "Enter an address before you save an API key.".to_string(),
            false => "This provider takes no API key.".to_string(),
        })
}

/// The state of the saved key of `provider` for its host (R18). The read
/// runs off the settings lock, on the blocking pool.
#[tauri::command]
#[specta::specta]
pub async fn provider_credential_status(
    app: AppHandle,
    manager: State<'_, SecretsManager>,
    provider: Provider,
) -> Result<CredentialStatus, String> {
    let location = match key_location(&app, provider) {
        Ok(location) => location,
        Err(message) => return Ok(CredentialStatus::Failed { message }),
    };
    let host = location.host();
    Ok(match manager.run(move |store| location.read(store)).await {
        Ok(Some(secret)) => CredentialStatus::Present {
            host,
            length: secret.chars().count() as u32,
            fingerprint: fingerprint(&secret),
        },
        Ok(None) => CredentialStatus::Absent { host },
        Err(e) => status_from_error(e),
    })
}

/// Save the key of `provider` for its host (R10).
#[tauri::command]
#[specta::specta]
pub async fn set_provider_credential(
    app: AppHandle,
    manager: State<'_, SecretsManager>,
    provider: Provider,
    value: String,
) -> Result<(), String> {
    let location = key_location(&app, provider)?;
    manager
        .run(move |store| location.save(store, &value))
        .await
        .map_err(|e| e.to_string())
}

/// Delete the key of `provider` for its host, and an old host-only entry
/// that the migration left for it (KTD8).
#[tauri::command]
#[specta::specta]
pub async fn delete_provider_credential(
    app: AppHandle,
    manager: State<'_, SecretsManager>,
    provider: Provider,
) -> Result<(), String> {
    let location = key_location(&app, provider)?;
    manager
        .run(move |store| location.delete(store))
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
    // The server has no model list, but its chat route answers. The Model
    // row is then a text field (R17).
    ChatOnly,
    Redirected { host: Option<String> },
    ServerError { status: Option<u16> },
    Unreachable,
    NotRequested { reason: ListNotRequested },
}

/// The result of the connection check (KTD9): the model list outcome, and
/// the address that worked. For Custom, the address can have `/v1` added
/// (R14). `None` when no address worked or no request was sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct EndpointCheck {
    pub models: EndpointModels,
    pub address: Option<String>,
}

impl EndpointCheck {
    fn without_address(models: EndpointModels) -> Self {
        Self {
            models,
            address: None,
        }
    }
}

/// Why Longhand sent no model list request (R12, R17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum ListNotRequested {
    NoProvider,
    NoAddress,
    NotChosen,
    KeychainLocked,
    KeychainFailed,
    KeyNeeded,
}

impl From<settings::NotRequestedReason> for ListNotRequested {
    fn from(reason: settings::NotRequestedReason) -> Self {
        use settings::NotRequestedReason as R;
        match reason {
            R::NoProvider => Self::NoProvider,
            R::NoAddress => Self::NoAddress,
            R::NotChosen => Self::NotChosen,
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
            L::ChatOnly => Self::ChatOnly,
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

/// Apply the request gate (R12) for `provider` to the settings and to the
/// result of the Keychain read. A local provider or Custom with no key is
/// requested only when it is the active provider, which the user chose.
fn list_step(
    s: &Settings,
    provider: Provider,
    read: Result<Option<String>, secrets::SecretError>,
) -> ListStep {
    use settings::EndpointGate;
    let address = s.provider_address(provider);
    let key = settings::key_state(&read);
    let chosen = s.active_provider == Some(provider);
    let credential = match settings::provider_gate(Some(provider), chosen, &address, key) {
        EndpointGate::NotRequested(reason) => {
            return ListStep::Done(EndpointModels::NotRequested {
                reason: reason.into(),
            });
        }
        EndpointGate::RequestWithKey => read.ok().flatten(),
        EndpointGate::RequestWithoutKey => None,
    };
    ListStep::Request(summarize::Endpoint {
        provider,
        base_url: address,
        model: s.providers.get(provider).model.clone(),
        credential,
    })
}

/// Check the connection to `provider` and list its models (R17, KTD9).
/// The command reads the key itself, so the webview never holds it. It
/// sends no request when the request gate refuses one (R12). For Custom,
/// it saves the address that worked when that address has `/v1` added
/// (R14), but only while the saved address is still the one that the
/// check used, so a slow reply never saves over a newer address. The
/// network and Keychain work runs off the settings lock.
#[tauri::command]
#[specta::specta]
pub async fn check_provider(
    app: AppHandle,
    manager: State<'_, SecretsManager>,
    provider: Provider,
) -> Result<EndpointCheck, String> {
    let s = settings::get_settings(&app);
    let read = match s.key_location(provider) {
        Some(location) => manager.run(move |store| location.read(store)).await,
        None => Ok(None),
    };
    let endpoint = match list_step(&s, provider, read) {
        ListStep::Done(outcome) => return Ok(EndpointCheck::without_address(outcome)),
        ListStep::Request(endpoint) => endpoint,
    };
    let started = endpoint.base_url.clone();
    let Ok(client) = summarize::Client::new(endpoint) else {
        return Ok(EndpointCheck::without_address(EndpointModels::Unreachable));
    };
    let checked = summarize::check(&client).await;
    if let Some(resolved) = checked.address.clone()
        && provider == Provider::Custom
        && resolved != started
    {
        blocking(move || {
            settings::update_settings(&app, |s| {
                s.save_checked_address(provider, &started, &resolved);
                Ok(())
            })
        })
        .await?;
    }
    Ok(EndpointCheck {
        models: checked.list.into(),
        address: checked.address,
    })
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

    /// Leave them waits while a note that is not saved yet
    /// is written into the current storage folder.
    #[test]
    fn leave_them_is_refused_while_a_note_is_saved_to_the_current_folder() {
        use pipeline::early_home;
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("Notes");
        std::fs::create_dir_all(&current).unwrap();
        let store: pipeline::SharedStore = Arc::new(std::sync::Mutex::new(
            session_core::Store::open_in_memory().unwrap(),
        ));
        {
            let mut s = store.lock().unwrap();
            s.create_session("s", chrono::Local::now().fixed_offset(), None)
                .unwrap();
            early_home::begin(&s, "s").unwrap();
        }
        let check = || check_leave(&store, current.to_str().unwrap(), &|_| false);
        assert_eq!(check(), Ok(()));

        // The early write is between its staging folder and its rename.
        let next = early_home::step(
            &store,
            "s",
            &early_home::EarlyState::Begun,
            Some(&current),
            "run",
            &session_save::RealFs,
        )
        .unwrap();
        early_home::record(&store, "s", &next).unwrap();

        assert_eq!(check(), Err(NOTE_SAVING.to_string()));
        early_home::write(&store, "s", Some(&current), "run", &session_save::RealFs).unwrap();
        assert_eq!(check(), Ok(()));
    }

    /// Leave them waits for a first Save into the current storage folder
    /// that runs now, and not for one that stopped.
    #[test]
    fn leave_them_waits_only_for_a_first_save_that_runs_now() {
        use pipeline::early_home;
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("Notes");
        std::fs::create_dir_all(&current).unwrap();
        let store: pipeline::SharedStore = Arc::new(std::sync::Mutex::new(
            session_core::Store::open_in_memory().unwrap(),
        ));
        {
            let mut s = store.lock().unwrap();
            s.create_session("s", chrono::Local::now().fixed_offset(), None)
                .unwrap();
            early_home::begin(&s, "s").unwrap();
        }
        early_home::write(&store, "s", Some(&current), "run", &session_save::RealFs).unwrap();
        // The first Save began to publish into the home folder.
        store
            .lock()
            .unwrap()
            .put_checkpoint(
                "s",
                session_core::Stage::Save,
                pipeline::save::STATE,
                &serde_json::to_string(&session_save::SaveState::Working).unwrap(),
            )
            .unwrap();
        let current = current.to_str().unwrap();

        // The Save stopped: it does not run now.
        assert_eq!(check_leave(&store, current, &|_| false), Ok(()));
        // The Save runs now.
        assert_eq!(
            check_leave(&store, current, &|id| id == "s"),
            Err(NOTE_SAVING.to_string())
        );
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

    // R9 at a change of the storage folder (U7): a new storage folder that
    // contains the watched folder is accepted, and the same settings write
    // turns the watched folder off and gives the reason. A folder apart
    // keeps the watched folder.
    #[test]
    fn a_new_storage_folder_that_contains_the_watched_folder_turns_it_off() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("Notes");
        let containing = dir.path().join("Disk");
        let watched = containing.join("Recordings");
        let other = dir.path().join("Other");
        for folder in [&current, &watched, &other] {
            std::fs::create_dir_all(folder).unwrap();
        }
        let (current, watched) = (current.to_str().unwrap(), watched.to_str().unwrap());
        let new = containing.to_str().unwrap();
        assert_eq!(check_change(current, new, false), Ok(containing.clone()));
        let mut s = Settings {
            storage_folder: current.into(),
            watched_folder: Some(watched.into()),
            ..Settings::default()
        };
        let off = change_settings(&mut s, new);
        assert_eq!(s.storage_folder, new);
        assert_eq!(s.watched_folder, None);
        assert!(off.unwrap().contains("inside the new storage folder"));
        let mut s = Settings {
            storage_folder: current.into(),
            watched_folder: Some(watched.into()),
            ..Settings::default()
        };
        assert_eq!(change_settings(&mut s, other.to_str().unwrap()), None);
        assert_eq!(s.watched_folder.as_deref(), Some(watched));
    }

    // R6 after Leave Them (U7): the question counts the audio files of the
    // watched folder against the new storage folder's empty import record,
    // and the files to hold until the answer are the same files.
    #[test]
    fn leave_them_with_5_audio_files_asks_about_5_files() {
        let dir = tempfile::tempdir().unwrap();
        let (watched, new) = (dir.path().join("Recordings"), dir.path().join("Disk"));
        std::fs::create_dir_all(&new).unwrap();
        std::fs::create_dir_all(&watched).unwrap();
        for i in 0..5 {
            std::fs::write(watched.join(format!("Memo {i}.m4a")), b"memo").unwrap();
        }
        std::fs::write(watched.join("Agenda.txt"), b"text").unwrap();
        let (preview, held) = files_to_ask(&watched, &new).unwrap();
        assert_eq!(preview.count, 5);
        assert_eq!(preview.bytes, 20);
        assert_eq!(held.len(), 5);
        // A folder with no audio file that the record does not hold: no
        // question.
        let empty = dir.path().join("Empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(files_to_ask(&empty, &new).is_none());
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
    fn the_add_list_leaves_out_the_apps_on_the_list() {
        let app = |id: &str| audio_system::apps::App {
            id: id.into(),
            name: id.into(),
            path: format!("/Applications/{id}.app"),
        };
        let listed = vec![ListedApp::from(app("us.zoom.xos"))];
        let ids: Vec<String> =
            not_listed(vec![app("com.hnc.Discord"), app("us.zoom.xos")], &listed)
                .into_iter()
                .map(|a| a.id)
                .collect();
        assert_eq!(ids, vec!["com.hnc.Discord"]);
    }

    #[test]
    fn the_note_menu_list_keeps_the_apps_on_the_list_and_marks_call_apps() {
        let app = |id: &str| audio_system::apps::App {
            id: id.into(),
            name: id.into(),
            path: format!("/Applications/{id}.app"),
        };
        let zoom = if cfg!(target_os = "windows") {
            "zoom"
        } else {
            "us.zoom.xos"
        };
        let all: Vec<(String, bool)> = not_listed(vec![app("com.google.Chrome"), app(zoom)], &[])
            .into_iter()
            .map(|a| (a.id, a.call_app))
            .collect();
        assert_eq!(
            all,
            vec![
                ("com.google.Chrome".to_owned(), false),
                (zoom.to_owned(), true)
            ]
        );
    }

    #[test]
    fn an_old_windows_refuses_the_add_commands() {
        assert_eq!(
            app_audio_supported(false),
            Err(APP_AUDIO_UNAVAILABLE.to_string())
        );
        assert_eq!(app_audio_supported(true), Ok(()));
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

    /// Settings with `provider` active and, when given, the address
    /// `address`.
    fn active(provider: Provider, address: Option<&str>) -> Settings {
        let mut s = Settings::default();
        s.choose_provider(Some(provider));
        if let Some(address) = address {
            s.set_provider_address(provider, address).unwrap();
        }
        s
    }

    fn not_requested(reason: ListNotRequested) -> ListStep {
        ListStep::Done(EndpointModels::NotRequested { reason })
    }

    #[test]
    fn a_cloud_provider_with_no_key_is_not_requested() {
        for s in [Settings::default(), active(Provider::OpenAi, None)] {
            assert_eq!(
                list_step(&s, Provider::OpenAi, Ok(None)),
                not_requested(ListNotRequested::KeyNeeded)
            );
        }
    }

    #[test]
    fn a_cloud_provider_with_a_key_is_requested_at_its_address_with_the_key() {
        let mut s = Settings::default();
        s.set_provider_model(Provider::Gemini, "gemini-3-flash");
        assert_eq!(
            list_step(&s, Provider::Gemini, Ok(Some("sk-test".into()))),
            ListStep::Request(summarize::Endpoint {
                provider: Provider::Gemini,
                base_url: "https://generativelanguage.googleapis.com".into(),
                model: "gemini-3-flash".into(),
                credential: Some("sk-test".into()),
            })
        );
    }

    #[test]
    fn a_chosen_local_provider_is_requested_with_no_key() {
        assert_eq!(
            list_step(
                &active(Provider::Ollama, Some(LOCAL)),
                Provider::Ollama,
                Ok(None)
            ),
            ListStep::Request(summarize::Endpoint {
                provider: Provider::Ollama,
                base_url: LOCAL.into(),
                model: String::new(),
                credential: None,
            })
        );
    }

    #[test]
    fn a_local_provider_that_the_user_did_not_choose_is_not_requested() {
        assert_eq!(
            list_step(&active(Provider::OpenAi, None), Provider::Ollama, Ok(None)),
            not_requested(ListNotRequested::NotChosen)
        );
    }

    #[test]
    fn custom_with_no_address_is_not_requested() {
        assert_eq!(
            list_step(
                &active(Provider::Custom, None),
                Provider::Custom,
                Ok(Some("sk-test".into()))
            ),
            not_requested(ListNotRequested::NoAddress)
        );
    }

    #[test]
    fn a_keychain_that_cannot_be_read_is_not_requested() {
        let custom = active(Provider::Custom, Some(LOCAL));
        for error in [
            SecretError::Locked {
                message: "locked".into(),
            },
            SecretError::Unavailable {
                message: "none".into(),
            },
        ] {
            assert_eq!(
                list_step(&custom, Provider::Custom, Err(error)),
                not_requested(ListNotRequested::KeychainLocked)
            );
        }
        assert_eq!(
            list_step(
                &Settings::default(),
                Provider::OpenAi,
                Err(SecretError::Platform {
                    message: "broken".into(),
                })
            ),
            not_requested(ListNotRequested::KeychainFailed)
        );
    }

    #[test]
    fn every_gate_reason_has_its_own_frontend_reason() {
        use settings::NotRequestedReason as R;
        let reasons = [
            (R::NoProvider, ListNotRequested::NoProvider),
            (R::NoAddress, ListNotRequested::NoAddress),
            (R::NotChosen, ListNotRequested::NotChosen),
            (R::KeychainLocked, ListNotRequested::KeychainLocked),
            (R::KeychainFailed, ListNotRequested::KeychainFailed),
            (R::KeyNeeded, ListNotRequested::KeyNeeded),
        ];
        for (reason, expected) in reasons {
            assert_eq!(ListNotRequested::from(reason), expected);
        }
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
            (ModelList::ChatOnly, EndpointModels::ChatOnly),
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
        assert_eq!(
            serde_json::to_value(EndpointCheck {
                models: EndpointModels::ChatOnly,
                address: Some("http://127.0.0.1:4000/v1".into()),
            })
            .unwrap(),
            serde_json::json!({
                "models": { "kind": "chat-only" },
                "address": "http://127.0.0.1:4000/v1"
            })
        );

        let bindings = include_str!("../../../src/bindings.ts");
        for declaration in [
            "export type EndpointModels = { kind: \"listed\"; ids: string[] } | \
             { kind: \"key-needed\" } | { kind: \"key-rejected\" } | \
             { kind: \"key-cannot-list\" } | { kind: \"not-listed\" } | \
             { kind: \"chat-only\" } | { kind: \"redirected\"; host: string | null } | \
             { kind: \"server-error\"; status: number | null } | \
             { kind: \"unreachable\" } | \
             { kind: \"not-requested\"; reason: ListNotRequested }",
            "export type ListNotRequested = \"no-provider\" | \"no-address\" | \
             \"not-chosen\" | \"keychain-locked\" | \"keychain-failed\" | \"key-needed\"",
            "export type EndpointCheck = { models: EndpointModels; address: string | null }",
            "async checkProvider(provider: Provider) : Promise<Result<EndpointCheck, string>>",
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
#[tauri::command]
#[specta::specta]
pub async fn set_recognition_defaults(
    app: AppHandle,
    manager: State<'_, Arc<ModelsManager>>,
    config: models::recognition::RecognitionConfig,
) -> Result<Settings, String> {
    models::recognition::resolve(&config).map_err(|e| e.to_string())?;
    let manager = Arc::clone(&manager);
    blocking(move || {
        let choice = settings::RecognitionChoice::from_config(&config);
        let result = choose_recognition(&app, &manager, |c| *c = choice)?;
        // The files of the chosen configuration download first. The rest
        // of the bundle follows (KTD5).
        manager.start_bundle_downloads(&app, &config.asr_model, &[])?;
        Ok(result)
    })
    .await
}
