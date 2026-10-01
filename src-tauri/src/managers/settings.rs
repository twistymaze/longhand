//! Settings in the application's own configuration store (R33, KTD15), and
//! the settings that the app uses: the local settings and the shared view
//! (KTD7).
//!
//! Shape follows Handy's `settings.rs` (one `#[serde(default)]` struct, a
//! schema-version integer, migrations keyed on the stored version, salvage of
//! individually valid fields); see src-tauri/PROVENANCE.md. Portable mode is
//! omitted. Credentials are not settings: they live in the OS secret store,
//! keyed by endpoint host (see `managers::secrets`).
//!
//! The local store holds only the settings that stay on this computer
//! (R13). The shared settings and the custom prompts come from the shared
//! settings manager (`managers::shared`). [`Settings`] joins both.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared_settings::file::{AudioRemoval, Field, SharedValue, is_catalog_asr_model};
use shared_settings::view::ActivePrompt;
use specta::Type;
use summarize::prompts::TOPIC_SUMMARY_ID;
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;
use tauri_specta::Event;

use crate::managers::shared::{SharedSettingsManager, SharedSnapshot, SharedState};

/// Relative, so `tauri-plugin-store` resolves it under the app data directory.
pub const SETTINGS_STORE_PATH: &str = "settings.json";
const SETTINGS_KEY: &str = "settings";

pub const CURRENT_SETTINGS_SCHEMA_VERSION: u32 = 6;

/// The local store version that moved the shared settings to the storage
/// folder (R12, KTD7). A store below it holds the old shared values.
const SHARED_SETTINGS_VERSION: u64 = 6;

/// The Base URL that Longhand uses when the user never saved one (R1, KD3).
pub const DEFAULT_ENDPOINT_BASE_URL: &str = "https://api.openai.com/v1";

/// The recognition model that versions 1 to 3 selected by default. A store
/// of those versions with no `asr_model` read as this model, so the
/// migration keeps it (KTD11).
const PREVIOUS_DEFAULT_ASR_MODEL: &str = "whisper-large-v3-turbo-q5km";

/// The chord a fresh install records with. Obscure enough to collide with
/// nothing common; the Phase 2a probe proved it registers on macOS.
pub const DEFAULT_SHORTCUT: &str = "CommandOrControl+Alt+Shift+L";

/// The keys of the shared settings in a local store from before version 6.
const LEGACY_SUMMARY_PROMPTS: &str = "summary_prompts";
const LEGACY_SHARED_KEYS: [&str; 9] = [
    shared_settings::file::KEY_ACTIVE_SUMMARY_PROMPT,
    shared_settings::file::KEY_DEFAULT_SPEAKER_COUNT,
    shared_settings::file::KEY_ASR_MODEL,
    shared_settings::file::KEY_LIVE_TRANSCRIPT,
    shared_settings::file::KEY_SPEAKER_SOURCE,
    shared_settings::file::KEY_SPEAKER_PROCESSING_MODE,
    shared_settings::file::KEY_RECOGNITION_LANGUAGE,
    shared_settings::file::KEY_CHECK_UPDATES_AUTOMATICALLY,
    LEGACY_SUMMARY_PROMPTS,
];

/// An optional bound on how many people are speaking (R36).
#[derive(Serialize, Deserialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpeakerCountSetting {
    Exact { count: u32 },
    Range { min: u32, max: u32 },
}

impl SpeakerCountSetting {
    pub fn validate(&self) -> Result<(), String> {
        match *self {
            Self::Exact { count } if count >= 1 => Ok(()),
            Self::Range { min, max } if min >= 1 && min <= max => Ok(()),
            Self::Exact { .. } => Err("a speaker count must be at least 1".into()),
            Self::Range { .. } => {
                Err("a speaker range must start at 1 or more and not run backwards".into())
            }
        }
    }
}

impl From<SpeakerCountSetting> for models::recognition::SpeakerCount {
    fn from(count: SpeakerCountSetting) -> Self {
        match count {
            SpeakerCountSetting::Exact { count } => Self::Exact { count },
            SpeakerCountSetting::Range { min, max } => Self::Range { min, max },
        }
    }
}

impl From<models::recognition::SpeakerCount> for SpeakerCountSetting {
    fn from(count: models::recognition::SpeakerCount) -> Self {
        use models::recognition::SpeakerCount;
        match count {
            SpeakerCount::Exact { count } => Self::Exact { count },
            SpeakerCount::Range { min, max } => Self::Range { min, max },
        }
    }
}

/// A shared setting: when Longhand removes the audio of a saved note (R1,
/// R3). The file form is [`AudioRemoval`]; this is the form of the
/// Settings window.
#[derive(Serialize, Deserialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioRemovalSetting {
    Off,
    /// The audio goes this many whole days after the note last recorded.
    AfterDays {
        days: u32,
    },
}

impl AudioRemovalSetting {
    /// Refuse a number of days that the shared settings file cannot hold.
    pub fn validate(&self) -> Result<(), String> {
        if AudioRemoval::from(*self).valid() {
            Ok(())
        } else {
            Err(format!(
                "Choose from {} to {} days.",
                shared_settings::file::AUDIO_REMOVAL_MIN_DAYS,
                shared_settings::file::AUDIO_REMOVAL_MAX_DAYS
            ))
        }
    }
}

impl Default for AudioRemovalSetting {
    fn default() -> Self {
        AudioRemoval::default().into()
    }
}

impl From<AudioRemovalSetting> for AudioRemoval {
    fn from(setting: AudioRemovalSetting) -> Self {
        match setting {
            AudioRemovalSetting::Off => AudioRemoval::Off,
            AudioRemovalSetting::AfterDays { days } => AudioRemoval::AfterDays(days),
        }
    }
}

impl From<AudioRemoval> for AudioRemovalSetting {
    fn from(removal: AudioRemoval) -> Self {
        match removal {
            AudioRemoval::Off => AudioRemovalSetting::Off,
            AudioRemoval::AfterDays(days) => AudioRemovalSetting::AfterDays { days },
        }
    }
}

/// The usual microphone (R1, KTD1): cpal's stable device ID, with the device
/// name as its label, so Settings can name a microphone that is not
/// connected. Lookup is by ID first, then by the exact label.
#[derive(Serialize, Deserialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct MicrophoneChoice {
    pub id: String,
    pub label: String,
}

/// A summary prompt that the user made (R1, KTD6). The built-in prompts are
/// never saved: their text lives in `summarize::prompts`.
#[derive(Serialize, Deserialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct SummaryPrompt {
    pub id: String,
    pub name: String,
    pub text: String,
    /// Empty for a prompt that the old local store holds: that store has
    /// no title texts (KTD6).
    #[serde(default)]
    pub title_text: String,
}

impl From<&summarize::Prompt> for SummaryPrompt {
    fn from(prompt: &summarize::Prompt) -> Self {
        Self {
            id: prompt.id.clone(),
            name: prompt.name.clone(),
            text: prompt.text.clone(),
            title_text: prompt.title_text.clone(),
        }
    }
}

impl From<&SummaryPrompt> for summarize::Prompt {
    fn from(prompt: &SummaryPrompt) -> Self {
        Self {
            id: prompt.id.clone(),
            name: prompt.name.clone(),
            text: prompt.text.clone(),
            title_text: prompt.title_text.clone(),
        }
    }
}

/// One row of the prompt list that the Settings window shows.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct SummaryPromptEntry {
    pub id: String,
    pub name: String,
    pub text: String,
    pub title_text: String,
    /// A built-in prompt cannot be renamed, edited or deleted (R2).
    pub built_in: bool,
}

/// The built-in prompts first, then the custom prompts, and the id of the
/// prompt that the next summary uses.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct SummaryPromptList {
    pub prompts: Vec<SummaryPromptEntry>,
    pub active: String,
    /// The most characters a prompt text can have.
    pub text_limit: u32,
    /// The most characters a title text can have.
    pub title_text_limit: u32,
}

/// The settings that stay on this computer (R13), as the local store holds
/// them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct LocalSettings {
    pub settings_schema_version: u32,
    pub interface_zoom_percent: u16,
    pub shortcut: Option<String>,
    pub show_window_shortcut: Option<String>,
    pub storage_folder: String,
    pub endpoint_base_url: Option<String>,
    pub endpoint_model: String,
    pub context_limit_tokens: Option<u32>,
    /// The recognition configuration in use on this computer (R18, KTD14).
    /// It is never written to the shared settings.
    pub asr_model: String,
    pub live_transcript: bool,
    pub speaker_source: models::recognition::SpeakerSource,
    pub speaker_processing_mode: models::recognition::ProcessingMode,
    pub recognition_language: Option<String>,
    pub default_speaker_count: Option<SpeakerCountSetting>,
    pub microphone: Option<MicrophoneChoice>,
    pub system_audio_default: bool,
    pub hide_from_dock: bool,
    pub learned_context_limits: std::collections::BTreeMap<String, u32>,
}

impl Default for LocalSettings {
    fn default() -> Self {
        Settings::default().local()
    }
}

impl LocalSettings {
    /// The storage folder, or `None` before the launch sets one.
    pub fn storage_folder(&self) -> Option<PathBuf> {
        folder_path(&self.storage_folder)
    }

    /// The recognition configuration in use on this computer.
    pub fn in_use(&self) -> RecognitionChoice {
        RecognitionChoice {
            asr_model: self.asr_model.clone(),
            live_transcript: self.live_transcript,
            speaker_source: self.speaker_source,
            speaker_processing_mode: self.speaker_processing_mode,
            recognition_language: self.recognition_language.clone(),
            default_speaker_count: self.default_speaker_count,
        }
    }
}

/// The six values of a recognition configuration, as the settings hold
/// them (R18): the chosen configuration in the shared settings, or the
/// configuration in use on this computer.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct RecognitionChoice {
    /// Catalog id of the transcription model (KTD6).
    pub asr_model: String,
    /// A streaming model transcribes during recording (R18, KD9).
    pub live_transcript: bool,
    pub speaker_source: models::recognition::SpeakerSource,
    pub speaker_processing_mode: models::recognition::ProcessingMode,
    pub recognition_language: Option<String>,
    pub default_speaker_count: Option<SpeakerCountSetting>,
}

impl Default for RecognitionChoice {
    fn default() -> Self {
        Self {
            asr_model: models::DEFAULT_ASR_MODEL.to_string(),
            live_transcript: true,
            speaker_source: Default::default(),
            speaker_processing_mode: Default::default(),
            recognition_language: None,
            default_speaker_count: None,
        }
    }
}

impl RecognitionChoice {
    /// The choice of a configuration that the Settings window sends.
    pub fn from_config(config: &models::recognition::RecognitionConfig) -> Self {
        use models::recognition::ProcessingMode;
        Self {
            asr_model: config.asr_model.clone(),
            live_transcript: config.processing_mode == ProcessingMode::DuringRecording,
            speaker_source: config.speaker_source,
            speaker_processing_mode: config.speaker_processing_mode,
            recognition_language: config.language.clone(),
            default_speaker_count: config.speaker_count.map(Into::into),
        }
    }

    /// The configuration that recognition resolves. Live transcript
    /// applies only to a model that streams.
    pub fn config(&self) -> models::recognition::RecognitionConfig {
        use models::recognition::{ProcessingMode, RecognitionConfig};
        let live_supported = models::model(&self.asr_model).is_some_and(|m| m.streaming);
        RecognitionConfig {
            asr_model: self.asr_model.clone(),
            speaker_source: self.speaker_source,
            processing_mode: if self.live_transcript && live_supported {
                ProcessingMode::DuringRecording
            } else {
                ProcessingMode::AfterStop
            },
            speaker_count: self.default_speaker_count.map(Into::into),
            language: self.recognition_language.clone(),
            speaker_processing_mode: self.speaker_processing_mode,
        }
    }

    /// The chosen configuration in the shared fields. A field that the
    /// file does not have reads as its default. A field with a value that
    /// this version does not know reads as the value of `in_use`, and the
    /// second value is true (R17).
    fn from_shared(
        fields: &shared_settings::file::SharedFields,
        in_use: &RecognitionChoice,
    ) -> (Self, bool) {
        let defaults = RecognitionChoice::default();
        let mut not_known = false;
        let choice = Self {
            asr_model: pick(
                &fields.asr_model,
                defaults.asr_model,
                in_use.asr_model.clone(),
                &mut not_known,
            ),
            live_transcript: pick(
                &fields.live_transcript,
                defaults.live_transcript,
                in_use.live_transcript,
                &mut not_known,
            ),
            speaker_source: pick(
                &fields.speaker_source,
                defaults.speaker_source,
                in_use.speaker_source,
                &mut not_known,
            ),
            speaker_processing_mode: pick(
                &fields.speaker_processing_mode,
                defaults.speaker_processing_mode,
                in_use.speaker_processing_mode,
                &mut not_known,
            ),
            recognition_language: pick(
                &fields.recognition_language,
                defaults.recognition_language,
                in_use.recognition_language.clone(),
                &mut not_known,
            ),
            default_speaker_count: match &fields.default_speaker_count {
                Field::Known(count) => count.map(SpeakerCountSetting::from),
                Field::Absent => defaults.default_speaker_count,
                Field::Unknown(_) => {
                    not_known = true;
                    in_use.default_speaker_count
                }
            },
        };
        (choice, not_known)
    }
}

/// The value of `field`: its default when the file does not have it, and
/// `current` when this version does not know its value.
fn pick<T: Clone>(field: &Field<T>, default: T, current: T, not_known: &mut bool) -> T {
    match field {
        Field::Known(value) => value.clone(),
        Field::Absent => default,
        Field::Unknown(_) => {
            *not_known = true;
            current
        }
    }
}

/// A failed download of a file that the chosen configuration needs (R19).
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct RecognitionFailure {
    /// The catalog id of the file.
    pub model_id: String,
    /// The cause, as Settings shows it.
    pub message: String,
}

/// The chosen recognition configuration compared with the configuration in
/// use, for the Settings window (R18, R19).
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq, Default)]
pub struct RecognitionState {
    /// The chosen configuration differs from the configuration in use.
    pub differs: bool,
    /// The chosen configuration has a value that this version does not
    /// know, for example a model from a newer version. The configuration in
    /// use stays, and no download starts.
    pub chosen_not_known: bool,
    /// The catalog ids of the files that the chosen configuration waits
    /// for. `ModelDownloadEvent` gives the progress of each.
    pub waiting_for: Vec<String>,
    /// A download of the chosen configuration failed. It stays failed
    /// until the user selects Retry.
    pub failure: Option<RecognitionFailure>,
}

fn folder_path(folder: &str) -> Option<PathBuf> {
    (!folder.is_empty()).then(|| PathBuf::from(folder))
}

/// The settings that the app uses: the local settings of this computer and
/// the shared settings of the storage folder (KTD7). The local store holds
/// only the local fields. Each shared field comes from the shared view; a
/// value that this version does not know reads as the default here, and
/// `shared.unknown_values` names it.
#[derive(Serialize, Type, Debug, Clone, PartialEq)]
pub struct Settings {
    pub settings_schema_version: u32,
    pub interface_zoom_percent: u16,
    /// The chord that starts and stops a recording. `None` when the user
    /// cleared it (R3). A stored string reads as `Some`, so no migration is
    /// needed (KTD3).
    pub shortcut: Option<String>,
    /// The chord that shows or hides the main window. Off by default (R5).
    pub show_window_shortcut: Option<String>,
    /// The one storage folder (R1). The library lists the notes in it, and
    /// the next note saves to it. Empty only until the launch sets the
    /// default folder (R2, KTD12).
    pub storage_folder: String,
    /// OpenAI-compatible base URL, e.g. `https://api.openai.com/v1` (R32).
    /// `None` means a new install that has no settings yet, and
    /// [`DEFAULT_ENDPOINT_BASE_URL`] applies. An empty value means that the
    /// user cleared it, or that an existing install never set it (R2, KTD4).
    pub endpoint_base_url: Option<String>,
    pub endpoint_model: String,
    /// Overrides the probed context budget when set (KTD16).
    pub context_limit_tokens: Option<u32>,
    /// This field and the five after it are the recognition configuration
    /// in use on this computer (R18, KTD14). Recording, processing and
    /// Transcribe Again read them. They change to `chosen_recognition` as
    /// one unit when every model that it needs is on disk.
    pub default_speaker_count: Option<SpeakerCountSetting>,
    /// Catalog id of the recognition model (KTD6).
    pub asr_model: String,
    /// A streaming model transcribes during recording (R18, KD9).
    pub live_transcript: bool,
    pub speaker_source: models::recognition::SpeakerSource,
    pub speaker_processing_mode: models::recognition::ProcessingMode,
    pub recognition_language: Option<String>,
    /// A shared setting. The chosen recognition configuration (R18).
    pub chosen_recognition: RecognitionChoice,
    /// The chosen configuration compared with the configuration in use,
    /// and the downloads that it waits for (R19).
    pub recognition: RecognitionState,
    /// The microphone a new note records from. `None` is "System default"
    /// (R1, KTD17).
    pub microphone: Option<MicrophoneChoice>,
    /// Whether a new note starts with system audio on (R6, KD13).
    pub system_audio_default: bool,
    /// macOS only: the app has no Dock icon while no window is visible
    /// (R2, R3). Off by default: the Dock icon stays, as is usual on macOS.
    pub hide_from_dock: bool,
    /// A shared setting. The app checks for a newer build of its track at
    /// launch, each hour and at each wake (R9, R10). On by default. "Check
    /// for Updates…" works when it is off (R11).
    pub check_updates_automatically: bool,
    /// A shared setting. When the audio of a saved note goes (R1, R2). On
    /// by default, with a delay of 30 days. A value that this version does
    /// not know shows as the default here and in `shared.unknown_values`.
    /// The removal pass does not read this field: it reads
    /// `SharedSnapshot::audio_removal_days`, which gives no delay for such a
    /// value or while the shared settings are read-only.
    pub audio_removal: AudioRemovalSetting,
    /// Context limits endpoints have shown by rejecting longer prompts,
    /// keyed by [`learned_limit_key`] (KTD16).
    pub learned_context_limits: std::collections::BTreeMap<String, u32>,
    /// A shared setting. The custom summary prompts, in the order that
    /// Settings shows them (KTD6).
    pub summary_prompts: Vec<SummaryPrompt>,
    /// A shared setting. The id of the prompt that the next summary uses.
    /// An id that matches no prompt resolves to the Topic summary.
    pub active_summary_prompt: String,
    /// Whether this computer can change the shared settings now, and the
    /// values that this version does not know (R16, R17).
    pub shared: SharedState,
    /// The last copy of the active custom prompt that this computer read.
    /// A summary uses it while the prompt's file is missing or has a
    /// conflict copy (R14).
    #[serde(skip)]
    pub cached_active_prompt: Option<summarize::Prompt>,
}

/// The settings changed without a command from the surface, for example
/// when a picked model becomes the selected model after its download.
#[derive(Serialize, Type, Debug, Clone, tauri_specta::Event)]
pub struct SettingsChanged {
    pub settings: Settings,
}

/// Tell the surface that the settings changed.
pub fn emit_settings_changed(app: &AppHandle) {
    let settings = get_settings(app);
    if let Err(e) = (SettingsChanged { settings }).emit(app) {
        log::warn!("failed to emit the settings change: {e}");
    }
}

/// One endpoint and model: a limit learned for one says nothing of another.
pub fn learned_limit_key(base_url: &str, model: &str) -> String {
    format!("{} {}", base_url.trim(), model.trim())
}

/// The state of the saved key for the host of the effective Base URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Saved,
    Absent,
    /// Keychain is locked or refused access. The user can repair it (R16).
    Locked,
    /// Keychain cannot be read for another reason.
    Failed,
}

/// Why Longhand sends no request to the endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRequestedReason {
    /// The user cleared the Base URL (R17).
    NoBaseUrl,
    KeychainLocked,
    KeychainFailed,
    /// The Base URL is the built-in default and no key is saved for its
    /// host (R4, KD7).
    KeyNeeded,
}

/// Whether Longhand can send a request to the endpoint, and with which key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointGate {
    RequestWithKey,
    RequestWithoutKey,
    NotRequested(NotRequestedReason),
}

/// The key state from the result of a Keychain read. The list command,
/// summary processing and readiness all call it, so they map Keychain errors
/// the same way. A key that is empty after trim counts as no key: it must
/// not pass the gate or go out as a blank Bearer header.
pub fn key_state(read: &Result<Option<String>, secrets::SecretError>) -> KeyState {
    use secrets::SecretError;
    match read {
        Ok(Some(key)) if !key.trim().is_empty() => KeyState::Saved,
        Ok(_) => KeyState::Absent,
        Err(SecretError::Locked { .. } | SecretError::Unavailable { .. }) => KeyState::Locked,
        Err(_) => KeyState::Failed,
    }
}

/// The request gate for the endpoint (KTD3). Longhand contacts only an
/// endpoint that the user acted on: a key is saved for its host, or the user
/// saved the Base URL. The list command and summary processing both call it.
pub fn endpoint_gate(base_url: &str, saved_by_user: bool, key: KeyState) -> EndpointGate {
    if base_url.trim().is_empty() {
        return EndpointGate::NotRequested(NotRequestedReason::NoBaseUrl);
    }
    match key {
        KeyState::Locked => EndpointGate::NotRequested(NotRequestedReason::KeychainLocked),
        KeyState::Failed => EndpointGate::NotRequested(NotRequestedReason::KeychainFailed),
        KeyState::Saved => EndpointGate::RequestWithKey,
        KeyState::Absent if saved_by_user => EndpointGate::RequestWithoutKey,
        KeyState::Absent => EndpointGate::NotRequested(NotRequestedReason::KeyNeeded),
    }
}

/// Whether summary processing can use the endpoint in `s`, given the state
/// of the key for its host. Readiness reports this value, so it applies the
/// same gate as summary processing. A locked or failed Keychain read counts
/// as configured: the Keychain problem has its own message.
pub fn endpoint_ready(s: &Settings, key: KeyState) -> bool {
    s.endpoint_configured()
        && !matches!(
            endpoint_gate(
                s.effective_endpoint_base_url(),
                s.endpoint_base_url_saved(),
                key
            ),
            EndpointGate::NotRequested(
                NotRequestedReason::KeyNeeded | NotRequestedReason::NoBaseUrl
            )
        )
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            settings_schema_version: CURRENT_SETTINGS_SCHEMA_VERSION,
            interface_zoom_percent: 100,
            shortcut: Some(DEFAULT_SHORTCUT.to_string()),
            show_window_shortcut: None,
            storage_folder: String::new(),
            endpoint_base_url: None,
            endpoint_model: String::new(),
            context_limit_tokens: None,
            default_speaker_count: None,
            asr_model: models::DEFAULT_ASR_MODEL.to_string(),
            live_transcript: true,
            speaker_source: Default::default(),
            speaker_processing_mode: Default::default(),
            recognition_language: None,
            chosen_recognition: RecognitionChoice::default(),
            recognition: RecognitionState::default(),
            microphone: None,
            system_audio_default: false,
            hide_from_dock: false,
            check_updates_automatically: true,
            audio_removal: AudioRemovalSetting::default(),
            learned_context_limits: Default::default(),
            summary_prompts: Vec::new(),
            active_summary_prompt: TOPIC_SUMMARY_ID.to_string(),
            shared: SharedState::default(),
            cached_active_prompt: None,
        }
    }
}

impl Settings {
    /// The settings from the local settings and the shared view. A shared
    /// field that the file does not have, or that has a value that this
    /// version does not know, reads as its default.
    pub fn from_parts(local: LocalSettings, shared: &SharedSnapshot) -> Self {
        let defaults = Settings::default();
        let fields = &shared.view.fields;
        let in_use = local.in_use();
        let (chosen_recognition, chosen_not_known) =
            RecognitionChoice::from_shared(fields, &in_use);
        Self {
            settings_schema_version: local.settings_schema_version,
            interface_zoom_percent: local.interface_zoom_percent,
            shortcut: local.shortcut,
            show_window_shortcut: local.show_window_shortcut,
            storage_folder: local.storage_folder,
            endpoint_base_url: local.endpoint_base_url,
            endpoint_model: local.endpoint_model,
            context_limit_tokens: local.context_limit_tokens,
            default_speaker_count: local.default_speaker_count,
            asr_model: local.asr_model,
            live_transcript: local.live_transcript,
            speaker_source: local.speaker_source,
            speaker_processing_mode: local.speaker_processing_mode,
            recognition_language: local.recognition_language,
            recognition: RecognitionState {
                differs: chosen_recognition != in_use,
                chosen_not_known,
                ..RecognitionState::default()
            },
            chosen_recognition,
            microphone: local.microphone,
            system_audio_default: local.system_audio_default,
            hide_from_dock: local.hide_from_dock,
            check_updates_automatically: known_or(
                &fields.check_updates_automatically,
                defaults.check_updates_automatically,
            ),
            audio_removal: known_or(&fields.audio_removal, AudioRemoval::default()).into(),
            learned_context_limits: local.learned_context_limits,
            summary_prompts: shared
                .view
                .prompts
                .iter()
                .map(SummaryPrompt::from)
                .collect(),
            active_summary_prompt: known_or(
                &fields.active_summary_prompt,
                defaults.active_summary_prompt,
            ),
            shared: shared.state(),
            cached_active_prompt: match &shared.view.active {
                ActivePrompt::Custom {
                    prompt,
                    from_cache: true,
                } => Some(prompt.clone()),
                _ => None,
            },
        }
    }

    /// The local fields, as the local store holds them.
    pub fn local(&self) -> LocalSettings {
        LocalSettings {
            settings_schema_version: self.settings_schema_version,
            interface_zoom_percent: self.interface_zoom_percent,
            shortcut: self.shortcut.clone(),
            show_window_shortcut: self.show_window_shortcut.clone(),
            storage_folder: self.storage_folder.clone(),
            endpoint_base_url: self.endpoint_base_url.clone(),
            endpoint_model: self.endpoint_model.clone(),
            context_limit_tokens: self.context_limit_tokens,
            asr_model: self.asr_model.clone(),
            live_transcript: self.live_transcript,
            speaker_source: self.speaker_source,
            speaker_processing_mode: self.speaker_processing_mode,
            recognition_language: self.recognition_language.clone(),
            default_speaker_count: self.default_speaker_count,
            microphone: self.microphone.clone(),
            system_audio_default: self.system_audio_default,
            hide_from_dock: self.hide_from_dock,
            learned_context_limits: self.learned_context_limits.clone(),
        }
    }

    /// The recognition configuration in use on this computer.
    pub fn in_use(&self) -> RecognitionChoice {
        self.local().in_use()
    }

    /// The configuration in use, as recognition resolves it.
    pub fn recognition_config(&self) -> models::recognition::RecognitionConfig {
        self.in_use().config()
    }

    /// The chosen configuration, as recognition resolves it. `None` when it
    /// has a value that this version does not know.
    pub fn chosen_config(&self) -> Option<models::recognition::RecognitionConfig> {
        (!self.recognition.chosen_not_known).then(|| self.chosen_recognition.config())
    }

    /// Make `choice` the configuration in use. Only the models manager
    /// calls this, when every model of the chosen configuration is on disk.
    pub fn set_in_use(&mut self, choice: &RecognitionChoice) {
        self.asr_model = choice.asr_model.clone();
        self.live_transcript = choice.live_transcript;
        self.speaker_source = choice.speaker_source;
        self.speaker_processing_mode = choice.speaker_processing_mode;
        self.recognition_language = choice.recognition_language.clone();
        self.default_speaker_count = choice.default_speaker_count;
        self.recognition.differs = self.chosen_recognition != *choice;
    }

    /// Choose the usual microphone, or `None` for "System default" (R1).
    /// A choice with a blank ID or label is refused and changes nothing.
    pub fn set_microphone(&mut self, choice: Option<MicrophoneChoice>) -> Result<(), String> {
        if let Some(choice) = &choice
            && (choice.id.trim().is_empty() || choice.label.trim().is_empty())
        {
            return Err("A microphone choice needs a device ID and a name.".into());
        }
        self.microphone = choice;
        Ok(())
    }

    /// The storage folder, or `None` before the launch sets one.
    pub fn storage_folder(&self) -> Option<PathBuf> {
        folder_path(&self.storage_folder)
    }

    pub fn endpoint_configured(&self) -> bool {
        !self.effective_endpoint_base_url().is_empty() && !self.endpoint_model.trim().is_empty()
    }

    /// The Base URL that requests use: the saved value, or the OpenAI
    /// default when the user never saved one (KTD4). Empty when the user
    /// cleared it.
    pub fn effective_endpoint_base_url(&self) -> &str {
        self.endpoint_base_url
            .as_deref()
            .unwrap_or(DEFAULT_ENDPOINT_BASE_URL)
            .trim()
    }

    /// Whether the user saved the Base URL, as opposed to the built-in
    /// default (R4).
    pub fn endpoint_base_url_saved(&self) -> bool {
        self.endpoint_base_url.is_some()
    }

    /// Save the Base URL (R32). An empty value clears it and stays empty
    /// (R2). Any other value must be a URL with a host, because the
    /// credential is keyed by that host (R30a). The Model does not change
    /// (KTD5).
    pub fn set_endpoint_base_url(&mut self, base_url: &str) -> Result<(), String> {
        let base_url = base_url.trim();
        if !base_url.is_empty() {
            secrets::endpoint_key(base_url).map_err(|e| e.to_string())?;
        }
        self.endpoint_base_url = Some(base_url.to_string());
        Ok(())
    }

    /// Save the Model. The Base URL does not change (KTD5).
    pub fn set_endpoint_model(&mut self, model: &str) {
        self.endpoint_model = model.trim().to_string();
    }

    /// The prompt that a summary run uses now (KTD4). An unknown id gives
    /// the Topic summary. While the file of the active prompt is missing or
    /// has a conflict copy, the last copy that this computer read applies
    /// (R14).
    pub fn active_summary_prompt_snapshot(&self) -> summarize::Prompt {
        let id = &self.active_summary_prompt;
        summarize::prompts::built_in(id)
            .or_else(|| {
                self.cached_active_prompt
                    .clone()
                    .filter(|prompt| prompt.id == *id)
            })
            .or_else(|| self.find_summary_prompt(id))
            .unwrap_or_else(summarize::prompts::topic_summary)
    }

    pub fn summary_prompt_list(&self) -> SummaryPromptList {
        let built_ins = summarize::prompts::built_ins()
            .into_iter()
            .map(|p| SummaryPromptEntry {
                id: p.id,
                name: p.name,
                text: p.text,
                title_text: p.title_text,
                built_in: true,
            });
        let custom = self.summary_prompts.iter().map(|p| SummaryPromptEntry {
            id: p.id.clone(),
            name: p.name.clone(),
            text: p.text.clone(),
            title_text: p.title_text.clone(),
            built_in: false,
        });
        SummaryPromptList {
            prompts: built_ins.chain(custom).collect(),
            active: self.active_summary_prompt_snapshot().id,
            text_limit: summarize::prompts::PROMPT_TEXT_LIMIT as u32,
            title_text_limit: summarize::prompts::TITLE_TEXT_LIMIT as u32,
        }
    }

    fn find_summary_prompt(&self, id: &str) -> Option<summarize::Prompt> {
        summarize::prompts::built_in(id).or_else(|| {
            self.summary_prompts
                .iter()
                .find(|p| p.id == id)
                .map(summarize::Prompt::from)
        })
    }
}

fn known_or<T: Clone>(field: &Field<T>, default: T) -> T {
    field.known().cloned().unwrap_or(default)
}

/// The shared settings that a change from `before` to `after` sets. Each
/// save writes only these values (R15). The prompts are files of their own
/// and have their own commands, so this ignores them. The recognition
/// configuration in use is local, so this ignores it too (R18).
pub fn shared_patch(before: &Settings, after: &Settings) -> Vec<SharedValue> {
    let mut patch = Vec::new();
    if before.active_summary_prompt != after.active_summary_prompt {
        patch.push(SharedValue::ActiveSummaryPrompt(
            after.active_summary_prompt.clone(),
        ));
    }
    let (chosen_before, chosen) = (&before.chosen_recognition, &after.chosen_recognition);
    if chosen_before.default_speaker_count != chosen.default_speaker_count {
        patch.push(SharedValue::DefaultSpeakerCount(
            chosen.default_speaker_count.map(Into::into),
        ));
    }
    if chosen_before.asr_model != chosen.asr_model {
        patch.push(SharedValue::AsrModel(chosen.asr_model.clone()));
    }
    if chosen_before.live_transcript != chosen.live_transcript {
        patch.push(SharedValue::LiveTranscript(chosen.live_transcript));
    }
    if chosen_before.speaker_source != chosen.speaker_source {
        patch.push(SharedValue::SpeakerSource(chosen.speaker_source));
    }
    if chosen_before.speaker_processing_mode != chosen.speaker_processing_mode {
        patch.push(SharedValue::SpeakerProcessingMode(
            chosen.speaker_processing_mode,
        ));
    }
    if chosen_before.recognition_language != chosen.recognition_language {
        patch.push(SharedValue::RecognitionLanguage(
            chosen.recognition_language.clone(),
        ));
    }
    if before.check_updates_automatically != after.check_updates_automatically {
        patch.push(SharedValue::CheckUpdatesAutomatically(
            after.check_updates_automatically,
        ));
    }
    if before.audio_removal != after.audio_removal {
        patch.push(SharedValue::AudioRemoval(after.audio_removal.into()));
    }
    patch
}

/// Parse a stored value into the local settings. Returns whether the stored
/// form differs from what would be written back, so the caller persists the
/// migration once and then converges.
pub fn settings_from_value(stored: &Value) -> (LocalSettings, bool) {
    let original = stored;
    let mut stored = stored.clone();
    let rewritten = rewrite_stale_recognition(&mut stored) | rewrite_destinations(&mut stored);
    let (mut settings, mut updated) = match LocalSettings::deserialize(&stored) {
        Ok(settings) => (settings, rewritten),
        Err(e) => {
            log::warn!("failed to parse stored settings ({e}); salvaging valid fields");
            (salvage_settings(&stored), true)
        }
    };
    if apply_settings_migrations(&mut settings, original) {
        updated = true;
    }
    (settings, updated)
}

/// Rewrite the stored recognition values in use that the catalog no longer
/// supports (R9, R10, KTD2). The rewrite runs before the parse, so the
/// parse and the salvage see only valid values. Returns whether a value
/// changed.
fn rewrite_stale_recognition(stored: &mut Value) -> bool {
    let Some(map) = stored.as_object_mut() else {
        return false;
    };
    let speaker_count = map
        .get("default_speaker_count")
        .and_then(|v| serde_json::from_value::<SpeakerCountSetting>(v.clone()).ok());
    let mut changed = false;
    if let Some(count) = speaker_count
        && count.validate().is_err()
    {
        map.remove("default_speaker_count");
        changed = true;
    }
    let speaker_count = speaker_count
        .filter(|c| c.validate().is_ok())
        .map(Into::into);
    changed | rewrite_recognition_fields(map, speaker_count)
}

/// Part of the version 6 migration: the current destination becomes the
/// one storage folder, and the other retained folders are forgotten (R8).
/// The rule looks at the fields, not at the version, because version 6
/// stores with the old fields exist on development builds. A storage folder
/// in the store stays. Returns whether a value changed.
fn rewrite_destinations(stored: &mut Value) -> bool {
    const ROOTS: &str = "destination_roots";
    const CURRENT: &str = "current_destination";
    let Some(map) = stored.as_object_mut() else {
        return false;
    };
    if !map.contains_key(ROOTS) && !map.contains_key(CURRENT) {
        return false;
    }
    map.remove(ROOTS);
    if let Some(current) = map.remove(CURRENT)
        && current.is_string()
        && !map.contains_key("storage_folder")
    {
        map.insert("storage_folder".into(), current);
    }
    true
}

/// Rewrite `asr_model` and `speaker_source` in old settings or in a stored
/// `RecognitionConfig`. Both use the same field names.
fn rewrite_recognition_fields(
    map: &mut serde_json::Map<String, Value>,
    speaker_count: Option<models::recognition::SpeakerCount>,
) -> bool {
    let mut changed = false;
    if map
        .get("asr_model")
        .and_then(Value::as_str)
        .is_some_and(|id| !is_catalog_asr_model(id))
    {
        map.insert("asr_model".into(), models::DEFAULT_ASR_MODEL.into());
        changed = true;
    }
    // "integrated" was the removed "Transcription model labels" source.
    if map.get("speaker_source").and_then(Value::as_str) == Some("integrated") {
        let asr_model = map
            .get("asr_model")
            .and_then(Value::as_str)
            .unwrap_or(models::DEFAULT_ASR_MODEL);
        let source = automatic_speaker_source(asr_model, speaker_count);
        map.insert(
            "speaker_source".into(),
            serde_json::to_value(source).expect("a speaker source serializes"),
        );
        changed = true;
    }
    changed
}

/// The automatic speaker choice (KTD5): the first after-Stop preference of
/// the model that resolves with the speaker count.
fn automatic_speaker_source(
    asr_model: &str,
    speaker_count: Option<models::recognition::SpeakerCount>,
) -> models::recognition::SpeakerSource {
    use models::recognition::{RecognitionConfig, SpeakerSource, resolve};
    models::model(asr_model)
        .and_then(|m| m.speaker_preferences.as_ref())
        .and_then(|preferences| {
            preferences.after_stop.iter().copied().find(|&source| {
                resolve(&RecognitionConfig {
                    asr_model: asr_model.into(),
                    speaker_source: source,
                    speaker_count,
                    ..Default::default()
                })
                .is_ok()
            })
        })
        .unwrap_or(SpeakerSource::Pyannote)
}

/// The stored version. An absent version is 0: a store written before
/// versioning existed.
fn stored_version(stored: &Value) -> u64 {
    stored
        .get("settings_schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn apply_settings_migrations(settings: &mut LocalSettings, stored: &Value) -> bool {
    let stored_version = stored_version(stored);
    let mut updated = false;
    // Versions 1 to 5 changed only the stored form or shared fields. The
    // rules for the shared fields apply in `legacy_shared` (R21). Version 6
    // moves the shared fields to the storage folder (R12, KTD7): this write
    // drops them from the local store, after `legacy_shared` put them into
    // the cache. The recognition values that the store used become the
    // configuration in use, with the same rules (R18).
    if stored_version < SHARED_SETTINGS_VERSION {
        if let Some(legacy) = legacy_shared(stored) {
            let fields = shared_settings::file::SharedFields::from_raw(&legacy.settings);
            let (in_use, _) =
                RecognitionChoice::from_shared(&fields, &RecognitionChoice::default());
            settings.asr_model = in_use.asr_model;
            settings.live_transcript = in_use.live_transcript;
            settings.speaker_source = in_use.speaker_source;
            settings.speaker_processing_mode = in_use.speaker_processing_mode;
            settings.recognition_language = in_use.recognition_language;
            settings.default_speaker_count = in_use.default_speaker_count;
        }
        updated = true;
    }
    let zoom = crate::interface_zoom::normalize(settings.interface_zoom_percent);
    if zoom != settings.interface_zoom_percent {
        settings.interface_zoom_percent = zoom;
        updated = true;
    }
    if updated {
        settings.settings_schema_version = CURRENT_SETTINGS_SCHEMA_VERSION;
    }
    updated
}

/// Rebuild settings from a stored value that failed to deserialize as a
/// whole, keeping every field that is individually valid so one bad value
/// never resets the rest.
fn salvage_settings(stored: &Value) -> LocalSettings {
    let Some(stored_map) = stored.as_object() else {
        log::warn!("stored settings are not a JSON object; falling back to defaults");
        return LocalSettings::default();
    };
    let mut merged = serde_json::to_value(LocalSettings::default())
        .expect("default settings serialize to an object");
    for (key, value) in stored_map {
        let map = merged
            .as_object_mut()
            .expect("merged settings stay an object");
        let previous = map.insert(key.clone(), value.clone());
        if serde_json::from_value::<LocalSettings>(merged.clone()).is_err() {
            // Log only the key: a value may be sensitive.
            log::warn!("dropping invalid settings field '{key}', keeping its default");
            let map = merged
                .as_object_mut()
                .expect("merged settings stay an object");
            match previous {
                Some(previous) => map.insert(key.clone(), previous),
                None => map.remove(key),
            };
        }
    }
    serde_json::from_value(merged).unwrap_or_default()
}

/// The shared settings that a local store from before version 6 holds
/// (R21, KTD7). They fill the cache when no cache exists, so the first
/// launch after the update writes them into the storage folder.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyShared {
    /// The shared settings file, with the defaults for each value that the
    /// store did not have.
    pub settings: serde_json::Map<String, Value>,
    pub prompts: Vec<summarize::Prompt>,
}

impl LegacyShared {
    /// The cache of these values. It has no source folder, so the first
    /// join treats the storage folder as a change of folder (R21).
    pub fn into_cache(self) -> shared_settings::cache::Cache {
        let active = self
            .settings
            .get(shared_settings::file::KEY_ACTIVE_SUMMARY_PROMPT)
            .and_then(Value::as_str);
        let active_prompt = self
            .prompts
            .iter()
            .find(|p| Some(p.id.as_str()) == active)
            .cloned();
        shared_settings::cache::Cache {
            source: PathBuf::new(),
            settings: self.settings,
            prompts: self.prompts,
            active_prompt,
        }
    }
}

/// Read the old shared values of a local store from before version 6. The
/// rules of versions 1 to 5 apply to them: each value parses on its own,
/// a model or speaker source that the catalog no longer has is rewritten,
/// a store below version 3 gets the Topic summary, and a store of versions
/// 1 to 3 with no model keeps the previous default model. `None` when the
/// store is current, or when it is a store with no version and no shared
/// values (a new install).
pub fn legacy_shared(stored: &Value) -> Option<LegacyShared> {
    let map = stored.as_object()?;
    let version = stored_version(stored);
    if version >= SHARED_SETTINGS_VERSION {
        return None;
    }
    if version == 0 && !LEGACY_SHARED_KEYS.iter().any(|key| map.contains_key(*key)) {
        return None;
    }
    let mut map = map.clone();
    let speaker_count = map
        .get("default_speaker_count")
        .and_then(|v| serde_json::from_value::<SpeakerCountSetting>(v.clone()).ok())
        .map(Into::into);
    rewrite_recognition_fields(&mut map, speaker_count);
    let value = |key: &str| map.get(key).cloned();
    fn parse<T: serde::de::DeserializeOwned>(value: Option<Value>) -> Option<T> {
        value.and_then(|v| serde_json::from_value(v).ok())
    }
    let mut patch = Vec::new();
    // Version 3 made the Topic summary active on every existing install
    // (KTD6).
    if version >= 3
        && let Some(id) = parse::<String>(value("active_summary_prompt"))
    {
        patch.push(SharedValue::ActiveSummaryPrompt(id));
    }
    match parse::<String>(value("asr_model")) {
        Some(id) if is_catalog_asr_model(&id) => patch.push(SharedValue::AsrModel(id)),
        // Version 4 changed the default model. An install that existed
        // before keeps the model that it used (KTD11).
        None if (1..4).contains(&version) && !map.contains_key("asr_model") => {
            patch.push(SharedValue::AsrModel(PREVIOUS_DEFAULT_ASR_MODEL.into()));
        }
        _ => {}
    }
    if let Some(count) = parse::<Option<SpeakerCountSetting>>(value("default_speaker_count"))
        && count.is_none_or(|c| c.validate().is_ok())
    {
        patch.push(SharedValue::DefaultSpeakerCount(count.map(Into::into)));
    }
    if let Some(on) = parse(value("live_transcript")) {
        patch.push(SharedValue::LiveTranscript(on));
    }
    if let Some(source) = parse(value("speaker_source")) {
        patch.push(SharedValue::SpeakerSource(source));
    }
    if let Some(mode) = parse(value("speaker_processing_mode")) {
        patch.push(SharedValue::SpeakerProcessingMode(mode));
    }
    if let Some(language) = parse(value("recognition_language")) {
        patch.push(SharedValue::RecognitionLanguage(language));
    }
    if let Some(on) = parse(value("check_updates_automatically")) {
        patch.push(SharedValue::CheckUpdatesAutomatically(on));
    }
    let settings =
        shared_settings::file::apply_patch(&shared_settings::file::default_settings(), &patch);
    Some(LegacyShared {
        settings,
        prompts: legacy_prompts(map.get(LEGACY_SUMMARY_PROMPTS)),
    })
}

/// The stored custom prompts that are each valid. A prompt whose id is a
/// built-in id or repeats an earlier id is dropped: it could never be
/// selected apart from the other one.
fn legacy_prompts(value: Option<&Value>) -> Vec<summarize::Prompt> {
    let Some(entries) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    entries
        .iter()
        .filter_map(|entry| {
            let prompt = serde_json::from_value::<SummaryPrompt>(entry.clone()).ok();
            if prompt.is_none() {
                log::warn!("dropping an invalid stored summary prompt");
            }
            prompt
        })
        .filter(|p| {
            let usable = summarize::prompts::built_in(&p.id).is_none() && seen.insert(p.id.clone());
            if !usable {
                log::warn!("dropping a stored summary prompt with a built-in or repeated id");
            }
            usable
        })
        .map(|p| summarize::Prompt::from(&p))
        .collect()
}

/// The folder of the local cache of the shared settings (KTD6).
pub fn shared_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

/// The settings that the app uses now.
pub fn get_settings(app: &AppHandle) -> Settings {
    with_downloads(
        app,
        Settings::from_parts(get_local_settings(app), &shared_snapshot(app)),
    )
}

/// `settings` with the downloads that the chosen recognition configuration
/// waits for (R19).
fn with_downloads(app: &AppHandle, mut settings: Settings) -> Settings {
    if let Some(models) = app.try_state::<Arc<crate::managers::models::ModelsManager>>() {
        models.report_choice(&mut settings);
    }
    settings
}

/// `local` with a storage folder: when it has none, the default folder
/// under `local_data` (R2, KTD12). The default folder is made when it does
/// not exist, also when the user removed it; a folder that the user chose
/// is never made. Returns whether the settings changed.
pub fn with_storage_folder(mut local: LocalSettings, local_data: &Path) -> (LocalSettings, bool) {
    let default = crate::managers::destination::default_storage_folder(local_data);
    let changed = local.storage_folder.is_empty();
    if changed {
        local.storage_folder = default.to_string_lossy().into_owned();
    }
    if local.storage_folder() == Some(default.clone())
        && let Err(e) = std::fs::create_dir_all(&default)
    {
        log::error!("could not make the default storage folder: {e}");
    }
    (local, changed)
}

/// At launch, before the shared settings join and the first library scan:
/// set and make the default storage folder when no storage folder is set
/// (R2, KTD12). Returns the local settings that the app uses.
pub fn ensure_storage_folder(app: &AppHandle, local: LocalSettings) -> LocalSettings {
    let local_data = match app.path().local_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("no local data folder for the default storage folder: {e}");
            return local;
        }
    };
    let (local, changed) = with_storage_folder(local, &local_data);
    if changed {
        log::info!("the storage folder is the default folder");
        let _guard = UPDATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = write_settings(app, &local) {
            log::error!("could not save the default storage folder: {e}");
        }
    }
    local
}

/// The shared view of the shared settings manager, or the defaults before
/// the manager exists.
fn shared_snapshot(app: &AppHandle) -> SharedSnapshot {
    app.try_state::<Arc<SharedSettingsManager>>()
        .map(|manager| manager.snapshot())
        .unwrap_or_default()
}

/// The local settings. A first run writes the defaults, and an old store
/// migrates. The migration of a store from before version 6 puts the old
/// shared values into the cache first, when no cache exists. When that
/// write fails, the store stays as it is, so the next read tries again.
pub fn get_local_settings(app: &AppHandle) -> LocalSettings {
    let store = match app.store(SETTINGS_STORE_PATH) {
        Ok(store) => store,
        Err(e) => {
            log::error!("failed to open settings store: {e}; using defaults");
            return LocalSettings::default();
        }
    };
    let stored = store.get(SETTINGS_KEY);
    let (settings, updated) = match &stored {
        Some(stored) => settings_from_value(stored),
        None => (LocalSettings::default(), true),
    };
    if updated {
        if let Some(legacy) = stored.as_ref().and_then(legacy_shared) {
            let seeded = shared_cache_dir(app).and_then(|dir| {
                crate::managers::shared::seed_cache(&dir, legacy).map_err(|e| e.to_string())
            });
            if let Err(e) = seeded {
                log::error!("could not keep the old shared settings in the cache: {e}");
                return settings;
            }
        }
        store.set(
            SETTINGS_KEY,
            serde_json::to_value(&settings).expect("settings serialize"),
        );
        if let Err(e) = store.save() {
            log::error!("failed to persist migrated settings: {e}");
        }
    }
    settings
}

/// Read the local settings and write nothing: a migrated value is not
/// saved. Launch reads the Dock policy with this before the single-instance
/// lock (U19), so a second copy that exits changes no file.
pub fn read_settings(app: &AppHandle) -> LocalSettings {
    app.store(SETTINGS_STORE_PATH)
        .ok()
        .and_then(|store| store.get(SETTINGS_KEY))
        .map(|stored| settings_from_value(&stored).0)
        .unwrap_or_default()
}

/// Serializes read-modify-write so two commands changing different fields
/// never lose one another's change.
static UPDATE_LOCK: Mutex<()> = Mutex::new(());

/// Apply `change` to the settings and save each store that the change
/// touches (R15, KTD7). The local fields go to the local store. The shared
/// fields that changed go to the shared settings file as a patch of only
/// those values. A store with no changed field is not written. Nothing is
/// written when `change` refuses, or when the change sets a shared field
/// while the shared settings are read-only. The prompts have their own
/// commands: a change of `summary_prompts` here is not saved.
pub fn update_settings(
    app: &AppHandle,
    change: impl FnOnce(&mut Settings) -> Result<(), String>,
) -> Result<Settings, String> {
    let _guard = UPDATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let local = get_local_settings(app);
    let mut shared = shared_snapshot(app);
    let before = Settings::from_parts(local.clone(), &shared);
    let mut after = before.clone();
    change(&mut after)?;
    let patch = shared_patch(&before, &after);
    if !patch.is_empty() {
        let manager = app
            .try_state::<Arc<SharedSettingsManager>>()
            .ok_or("The shared settings are not ready yet.")?;
        shared = manager.save(&patch)?;
    }
    let changed_local = after.local();
    if changed_local != local {
        write_settings(app, &changed_local)?;
    }
    Ok(with_downloads(
        app,
        Settings::from_parts(changed_local, &shared),
    ))
}

/// Write and flush immediately: the store's autosave is debounced, and a
/// setting changed just before quitting must survive the quit.
fn write_settings(app: &AppHandle, settings: &LocalSettings) -> Result<(), String> {
    let store = app.store(SETTINGS_STORE_PATH).map_err(|e| e.to_string())?;
    let previous = store.get(SETTINGS_KEY);
    store.set(
        SETTINGS_KEY,
        serde_json::to_value(settings).map_err(|e| e.to_string())?,
    );
    if let Err(error) = store.save() {
        match previous {
            Some(value) => store.set(SETTINGS_KEY, value),
            None => {
                store.delete(SETTINGS_KEY);
            }
        }
        return Err(error.to_string());
    }
    Ok(())
}

/// The settings that a local store gives after its first read: the local
/// fields, and the shared values that the migration puts into the cache.
#[cfg(test)]
pub(crate) fn migrated(stored: &Value) -> (Settings, bool) {
    use shared_settings::view::{self, Availability};
    let (local, updated) = settings_from_value(stored);
    let cache = legacy_shared(stored).map(LegacyShared::into_cache);
    let shared = SharedSnapshot {
        view: view::cached(cache.as_ref(), Availability::Available),
        ..SharedSnapshot::default()
    };
    (Settings::from_parts(local, &shared), updated)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use summarize::prompts::PLAY_BY_PLAY_ID;

    use super::*;

    /// The last version before the shared settings.
    const OLD: u64 = SHARED_SETTINGS_VERSION - 1;

    #[test]
    fn interface_zoom_defaults_and_invalid_fields_keep_other_settings() {
        for value in [
            serde_json::Value::Null,
            serde_json::json!(0),
            serde_json::json!(51),
            serde_json::json!(201),
            serde_json::json!(-10),
            serde_json::json!(125.5),
            serde_json::json!("bad"),
        ] {
            let (settings, _) = settings_from_value(
                &serde_json::json!({ "settings_schema_version": 4, "interface_zoom_percent": value, "endpoint_model": "kept" }),
            );
            assert_eq!(settings.interface_zoom_percent, 100);
            assert_eq!(settings.endpoint_model, "kept");
        }
        let (settings, _) = settings_from_value(
            &serde_json::json!({ "settings_schema_version": 4, "interface_zoom_percent": 160 }),
        );
        assert_eq!(settings.interface_zoom_percent, 160);
        assert_eq!(Settings::default().interface_zoom_percent, 100);
    }

    #[test]
    fn missing_fields_and_version_fill_defaults_and_migrate() {
        let (settings, updated) = settings_from_value(&json!({}));
        assert!(updated);
        assert_eq!(settings, LocalSettings::default());
        assert_eq!(
            settings.settings_schema_version,
            CURRENT_SETTINGS_SCHEMA_VERSION
        );
        // A store with no version and no shared values is a new install.
        assert_eq!(legacy_shared(&json!({})), None);

        let written = serde_json::to_value(&settings).unwrap();
        assert_eq!(settings_from_value(&written), (settings, false));
    }

    #[test]
    fn the_default_settings_are_the_default_local_settings_and_shared_view() {
        assert_eq!(
            Settings::from_parts(LocalSettings::default(), &SharedSnapshot::default()),
            Settings::default()
        );
    }

    #[test]
    fn a_phase_2a_store_migrates_onto_the_real_schema() {
        let stored = json!({ "settings_schema_version": 1, "probe_value": "Test" });
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        // The install existed before the default changed, so it keeps the
        // model it read before (KTD11), as the chosen model and as the
        // model in use.
        let whisper = RecognitionChoice {
            asr_model: PREVIOUS_DEFAULT_ASR_MODEL.to_string(),
            ..RecognitionChoice::default()
        };
        let mut expected = Settings {
            chosen_recognition: whisper.clone(),
            ..Settings::default()
        };
        expected.set_in_use(&whisper);
        assert_eq!(settings, expected);
        let written = serde_json::to_value(settings.local()).unwrap();
        assert!(written.get("probe_value").is_none());
        assert_eq!(
            written["settings_schema_version"],
            CURRENT_SETTINGS_SCHEMA_VERSION
        );
    }

    #[test]
    fn no_stored_settings_select_parakeet_and_the_live_transcript() {
        let settings = Settings::default();
        assert_eq!(settings.asr_model, "parakeet-unified-en-0.6b-q80");
        assert!(settings.live_transcript);
        assert!(!settings.recognition.differs);
        assert_eq!(migrated(&json!({})).0, settings);
    }

    #[test]
    fn a_version_2_store_keeps_its_whisper_model() {
        let stored = json!({
            "settings_schema_version": 2,
            "asr_model": "whisper-large-v3-turbo-q5km",
            "endpoint_model": "kept",
        });
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(
            settings.settings_schema_version,
            CURRENT_SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(settings.asr_model, "whisper-large-v3-turbo-q5km");
        assert_eq!(settings.endpoint_model, "kept");
        assert!(settings.live_transcript);
        assert!(!settings.recognition.differs);
        let written = serde_json::to_value(settings.local()).unwrap();
        assert_eq!(settings_from_value(&written), (settings.local(), false));
    }

    #[test]
    fn a_version_2_store_without_a_model_keeps_the_previous_default() {
        let stored = json!({ "settings_schema_version": 2, "endpoint_model": "x" });
        let (settings, _) = migrated(&stored);
        assert_eq!(settings.asr_model, PREVIOUS_DEFAULT_ASR_MODEL);
    }

    #[test]
    fn a_version_3_store_keeps_its_model_and_its_prompt() {
        // Version 3 stores come from two builds: the summary prompt build,
        // which writes `asr_model`, and an earlier build of the model picker,
        // which wrote `asr_model` and no prompt fields.
        let stored = json!({
            "settings_schema_version": 3,
            "asr_model": "parakeet-unified-en-0.6b-q80",
            "live_transcript": false,
            "active_summary_prompt": PLAY_BY_PLAY_ID,
        });
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(
            settings.settings_schema_version,
            CURRENT_SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(settings.asr_model, "parakeet-unified-en-0.6b-q80");
        assert!(!settings.live_transcript);
        assert_eq!(settings.active_summary_prompt, PLAY_BY_PLAY_ID);
        let picker_only = json!({
            "settings_schema_version": 3,
            "asr_model": "nemotron-speech-streaming-en-0.6b-q80",
        });
        let (settings, _) = migrated(&picker_only);
        assert_eq!(settings.asr_model, "nemotron-speech-streaming-en-0.6b-q80");
        assert_eq!(settings.active_summary_prompt, TOPIC_SUMMARY_ID);
    }

    #[test]
    fn a_store_from_each_version_before_shared_settings_moves_its_shared_values() {
        // Each installed build wrote a store of one of these versions. Each
        // store drops its shared values from the local store, and each value
        // goes into the cache (R21).
        let custom = json!([{ "id": "c1", "name": "Standup", "text": "Recap." }]);
        for version in 1..SHARED_SETTINGS_VERSION {
            let stored = json!({
                "settings_schema_version": version,
                "endpoint_model": "kept",
                "asr_model": "parakeet-tdt-0.6b-v3-q80",
                "live_transcript": false,
                "speaker_source": "sortformer",
                "speaker_processing_mode": "during_recording",
                "recognition_language": "de",
                "default_speaker_count": { "kind": "exact", "count": 3 },
                "check_updates_automatically": false,
                "summary_prompts": custom,
                "active_summary_prompt": "c1",
            });
            let (local, updated) = settings_from_value(&stored);
            assert!(updated, "version {version}");
            assert_eq!(local.endpoint_model, "kept");
            assert_eq!(
                local.settings_schema_version,
                CURRENT_SETTINGS_SCHEMA_VERSION
            );
            let written = serde_json::to_value(&local).unwrap();
            for key in [
                shared_settings::file::KEY_ACTIVE_SUMMARY_PROMPT,
                shared_settings::file::KEY_CHECK_UPDATES_AUTOMATICALLY,
                LEGACY_SUMMARY_PROMPTS,
            ] {
                assert!(written.get(key).is_none(), "version {version}: {key}");
            }
            // The recognition values that the store used are the
            // configuration in use (R18).
            assert_eq!(
                local.in_use(),
                RecognitionChoice {
                    asr_model: "parakeet-tdt-0.6b-v3-q80".into(),
                    live_transcript: false,
                    speaker_source: models::recognition::SpeakerSource::Sortformer,
                    speaker_processing_mode: models::recognition::ProcessingMode::DuringRecording,
                    recognition_language: Some("de".into()),
                    default_speaker_count: Some(SpeakerCountSetting::Exact { count: 3 }),
                },
                "version {version}"
            );

            let legacy = legacy_shared(&stored).expect("an old store has shared values");
            let fields = shared_settings::file::SharedFields::from_raw(&legacy.settings);
            assert_eq!(
                fields.asr_model,
                Field::Known("parakeet-tdt-0.6b-v3-q80".into())
            );
            assert_eq!(fields.live_transcript, Field::Known(false));
            assert_eq!(
                fields.speaker_source,
                Field::Known(models::recognition::SpeakerSource::Sortformer)
            );
            assert_eq!(
                fields.speaker_processing_mode,
                Field::Known(models::recognition::ProcessingMode::DuringRecording)
            );
            assert_eq!(fields.recognition_language, Field::Known(Some("de".into())));
            assert_eq!(
                fields.default_speaker_count,
                Field::Known(Some(models::recognition::SpeakerCount::Exact { count: 3 }))
            );
            assert_eq!(fields.check_updates_automatically, Field::Known(false));
            assert_eq!(
                fields.schema_version,
                Some(shared_settings::file::SCHEMA_VERSION)
            );
            // Version 3 made the Topic summary active on every install.
            let active = if version < 3 { TOPIC_SUMMARY_ID } else { "c1" };
            assert_eq!(fields.active_summary_prompt, Field::Known(active.into()));
            assert_eq!(legacy.prompts.len(), 1);
            assert_eq!(legacy.prompts[0].name, "Standup");
            let cache = legacy.into_cache();
            assert_eq!(cache.source, PathBuf::new());
            assert_eq!(
                cache.active_prompt.map(|p| p.id),
                (version >= 3).then(|| "c1".to_string())
            );
        }
    }

    #[test]
    fn a_current_store_has_no_shared_values_to_move() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "asr_model": "parakeet-tdt-0.6b-v3-q80",
            "active_summary_prompt": PLAY_BY_PLAY_ID,
        });
        assert_eq!(legacy_shared(&stored), None);
        // A shared key in a current store is not a local setting. The model
        // in a current store is the model in use.
        let (local, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(
            local,
            LocalSettings {
                asr_model: "parakeet-tdt-0.6b-v3-q80".into(),
                ..LocalSettings::default()
            }
        );
    }

    #[test]
    fn a_current_store_with_no_configuration_in_use_uses_the_defaults() {
        // A version 6 store of a development build has no configuration in
        // use. The chosen configuration then applies when its models are on
        // disk (R18).
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "endpoint_model": "kept",
        });
        let (local, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(local.endpoint_model, "kept");
        assert_eq!(local.in_use(), RecognitionChoice::default());
    }

    #[test]
    fn an_invalid_old_shared_value_takes_its_default_and_keeps_the_rest() {
        let stored = json!({
            "settings_schema_version": OLD,
            "live_transcript": "yes",
            "default_speaker_count": { "kind": "exact", "count": 0 },
            "recognition_language": "fr",
        });
        let (settings, _) = migrated(&stored);
        assert!(settings.live_transcript);
        assert_eq!(settings.default_speaker_count, None);
        assert_eq!(settings.recognition_language.as_deref(), Some("fr"));
    }

    #[test]
    fn a_stored_chord_reads_as_set_with_no_rewrite() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "shortcut": "CommandOrControl+Alt+Shift+L",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(
            settings.shortcut.as_deref(),
            Some("CommandOrControl+Alt+Shift+L")
        );
    }

    #[test]
    fn missing_chords_take_their_defaults() {
        let stored = json!({ "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION });
        let (settings, _) = settings_from_value(&stored);
        assert_eq!(settings.shortcut.as_deref(), Some(DEFAULT_SHORTCUT));
        assert_eq!(settings.show_window_shortcut, None);
    }

    #[test]
    fn a_stored_null_chord_reads_as_cleared() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "shortcut": null,
            "show_window_shortcut": "Super+Shift+KeyL",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(settings.shortcut, None);
        assert_eq!(
            settings.show_window_shortcut.as_deref(),
            Some("Super+Shift+KeyL")
        );
    }

    #[test]
    fn a_store_without_the_recording_keys_reads_as_system_default_and_no_system_audio() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "endpoint_model": "kept",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated, "the new keys need no migration (KTD17)");
        assert_eq!(settings.microphone, None);
        assert!(!settings.system_audio_default);
        assert_eq!(settings.endpoint_model, "kept");
        assert_eq!(Settings::default().microphone, None);
        assert!(!Settings::default().system_audio_default);
    }

    #[test]
    fn a_store_without_hide_from_dock_reads_as_off_and_needs_no_migration() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "endpoint_model": "kept",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated, "the new key needs no migration");
        assert!(!settings.hide_from_dock);
        assert_eq!(settings.endpoint_model, "kept");
        assert!(!Settings::default().hide_from_dock);
    }

    #[test]
    fn an_old_store_without_the_update_setting_reads_as_on() {
        let stored = json!({ "settings_schema_version": OLD, "endpoint_model": "kept" });
        let (settings, _) = migrated(&stored);
        assert!(settings.check_updates_automatically);
        assert_eq!(settings.endpoint_model, "kept");
        assert!(Settings::default().check_updates_automatically);
    }

    #[test]
    fn a_stored_update_setting_is_kept() {
        let stored = json!({
            "settings_schema_version": OLD,
            "check_updates_automatically": false,
        });
        let (settings, _) = migrated(&stored);
        assert!(!settings.check_updates_automatically);
    }

    #[test]
    fn a_stored_hide_from_dock_value_is_kept() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "hide_from_dock": true,
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert!(settings.hide_from_dock);
    }

    #[test]
    fn a_store_with_the_old_alert_test_key_loads_and_drops_it_on_the_next_write() {
        for heard in [false, true] {
            let stored = json!({
                "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
                "endpoint_model": "kept",
                "alert_tone_heard": heard,
            });
            let (settings, updated) = settings_from_value(&stored);
            assert!(!updated, "the removed key needs no migration (KTD16)");
            assert_eq!(settings.endpoint_model, "kept");
            let written = serde_json::to_value(&settings).unwrap();
            assert!(written.get("alert_tone_heard").is_none());
        }
    }

    fn usb_mic() -> MicrophoneChoice {
        MicrophoneChoice {
            id: "CoreAudio:AppleUSBAudioEngine:1".into(),
            label: "USB Audio".into(),
        }
    }

    #[test]
    fn a_chosen_microphone_and_system_default_round_trip_through_the_store() {
        let mut settings = Settings::default();
        settings.set_microphone(Some(usb_mic())).unwrap();
        let written = serde_json::to_value(settings.local()).unwrap();
        assert_eq!(
            written["microphone"],
            json!({ "id": "CoreAudio:AppleUSBAudioEngine:1", "label": "USB Audio" })
        );
        let (read, updated) = settings_from_value(&written);
        assert!(!updated);
        assert_eq!(read.microphone, Some(usb_mic()));

        settings.set_microphone(None).unwrap();
        let written = serde_json::to_value(settings.local()).unwrap();
        let (read, updated) = settings_from_value(&written);
        assert!(!updated);
        assert_eq!(read.microphone, None);
    }

    #[test]
    fn a_microphone_choice_needs_an_id_and_a_label() {
        let mut settings = Settings::default();
        settings.set_microphone(Some(usb_mic())).unwrap();
        for bad in [
            MicrophoneChoice {
                id: " ".into(),
                label: "USB Audio".into(),
            },
            MicrophoneChoice {
                id: "usb-1".into(),
                label: "".into(),
            },
        ] {
            assert!(settings.set_microphone(Some(bad)).is_err());
        }
        assert_eq!(
            settings.microphone,
            Some(usb_mic()),
            "a refusal changes nothing"
        );
    }

    #[test]
    fn a_malformed_microphone_is_dropped_and_the_rest_is_kept() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "microphone": "USB Audio",
            "system_audio_default": true,
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(settings.microphone, None);
        assert!(settings.system_audio_default);
    }

    #[test]
    fn present_fields_survive_a_version_migration() {
        let (settings, updated) = settings_from_value(
            &json!({ "endpoint_model": "gpt-5", "destination_roots": ["/a"], "current_destination": "/a" }),
        );
        assert!(updated);
        assert_eq!(settings.endpoint_model, "gpt-5");
        assert_eq!(settings.storage_folder, "/a");
    }

    /// R8: the current folder becomes the one storage folder, and the other
    /// folders are forgotten.
    #[test]
    fn a_store_with_three_earlier_folders_keeps_only_the_current_one() {
        for version in [OLD, u64::from(CURRENT_SETTINGS_SCHEMA_VERSION)] {
            let stored = json!({
                "settings_schema_version": version,
                "endpoint_model": "kept",
                "destination_roots": ["/vault/a", "/vault/b", "/vault/c", "/vault/d"],
                "current_destination": "/vault/c",
            });
            let (settings, updated) = settings_from_value(&stored);
            assert!(updated, "version {version}");
            assert_eq!(settings.storage_folder, "/vault/c");
            assert_eq!(settings.endpoint_model, "kept");
            let written = serde_json::to_value(&settings).unwrap();
            assert!(written.get("destination_roots").is_none());
            assert!(written.get("current_destination").is_none());
            assert_eq!(settings_from_value(&written), (settings, false));
        }
    }

    #[test]
    fn a_store_with_folders_and_no_current_one_has_no_storage_folder() {
        let (settings, updated) = settings_from_value(&json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "destination_roots": ["/vault/a"],
            "current_destination": null,
        }));
        assert!(updated);
        assert_eq!(settings.storage_folder, "");
        assert_eq!(settings.storage_folder(), None);
    }

    #[test]
    fn a_storage_folder_in_the_store_is_not_replaced_by_an_old_destination() {
        let (settings, updated) = settings_from_value(&json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "storage_folder": "/vault/new",
            "current_destination": "/vault/old",
        }));
        assert!(updated);
        assert_eq!(settings.storage_folder, "/vault/new");
    }

    #[test]
    fn a_launch_with_no_storage_folder_sets_and_makes_the_default_folder() {
        let local_data = tempfile::tempdir().unwrap();
        let (settings, changed) = with_storage_folder(LocalSettings::default(), local_data.path());
        assert!(changed);
        let folder = local_data.path().join("Longhand").join("Notes");
        assert_eq!(settings.storage_folder(), Some(folder.clone()));
        assert!(folder.is_dir());
        // A second launch changes nothing.
        let (again, changed) = with_storage_folder(settings.clone(), local_data.path());
        assert!(!changed);
        assert_eq!(again, settings);
    }

    #[test]
    fn a_launch_makes_the_default_folder_again_and_keeps_a_chosen_folder_as_it_is() {
        let local_data = tempfile::tempdir().unwrap();
        let (settings, _) = with_storage_folder(LocalSettings::default(), local_data.path());
        let folder = settings.storage_folder().unwrap();
        std::fs::remove_dir(&folder).unwrap();
        let (_, changed) = with_storage_folder(settings, local_data.path());
        assert!(!changed);
        assert!(folder.is_dir(), "the default folder is made again");

        let chosen = local_data.path().join("unplugged");
        let local = LocalSettings {
            storage_folder: chosen.to_string_lossy().into_owned(),
            ..LocalSettings::default()
        };
        let (kept, changed) = with_storage_folder(local.clone(), local_data.path());
        assert!(!changed);
        assert_eq!(kept, local);
        assert!(!chosen.exists(), "a chosen folder is never made");
    }

    /// The local store keeps only the fields that this version knows. The
    /// shared settings file keeps unknown fields (R17); the shared settings
    /// crate tests that rule.
    #[test]
    fn unknown_field_loads_and_is_dropped_on_the_next_write() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "endpoint_model": "x",
            "from_the_future": true,
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(settings.endpoint_model, "x");
        let written = serde_json::to_value(&settings).unwrap();
        assert!(written.get("from_the_future").is_none());
    }

    #[test]
    fn one_invalid_field_keeps_the_rest() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "context_limit_tokens": "lots",
            "endpoint_model": "kept",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(settings.endpoint_model, "kept");
        assert_eq!(settings.context_limit_tokens, None);
    }

    #[test]
    fn non_object_store_falls_back_to_defaults() {
        let (settings, updated) = settings_from_value(&json!("garbage"));
        assert!(updated);
        assert_eq!(settings, LocalSettings::default());
        assert_eq!(legacy_shared(&json!("garbage")), None);
    }

    #[test]
    fn speaker_counts_must_be_positive_and_ordered() {
        assert!(SpeakerCountSetting::Exact { count: 4 }.validate().is_ok());
        assert!(SpeakerCountSetting::Exact { count: 0 }.validate().is_err());
        assert!(
            SpeakerCountSetting::Range { min: 2, max: 2 }
                .validate()
                .is_ok()
        );
        assert!(
            SpeakerCountSetting::Range { min: 5, max: 3 }
                .validate()
                .is_err()
        );
        assert!(
            SpeakerCountSetting::Range { min: 0, max: 3 }
                .validate()
                .is_err()
        );
    }

    fn custom(id: &str, name: &str) -> SummaryPrompt {
        SummaryPrompt {
            id: id.into(),
            name: name.into(),
            text: "Write a recap.".into(),
            title_text: String::new(),
        }
    }

    fn names(settings: &Settings) -> Vec<&str> {
        settings
            .summary_prompts
            .iter()
            .map(|p| p.name.as_str())
            .collect()
    }

    #[test]
    fn an_empty_store_gets_the_current_version_and_topic_summary_active() {
        let (settings, _) = migrated(&json!({}));
        assert_eq!(
            settings.settings_schema_version,
            CURRENT_SETTINGS_SCHEMA_VERSION
        );
        assert!(settings.summary_prompts.is_empty());
        assert_eq!(settings.active_summary_prompt, TOPIC_SUMMARY_ID);
    }

    #[test]
    fn a_version_2_store_keeps_its_settings_and_gets_topic_summary_active() {
        let stored = json!({
            "settings_schema_version": 2,
            "endpoint_model": "gpt-5",
            "active_summary_prompt": PLAY_BY_PLAY_ID,
        });
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(settings.endpoint_model, "gpt-5");
        assert_eq!(settings.active_summary_prompt, TOPIC_SUMMARY_ID);
        assert_eq!(
            settings.settings_schema_version,
            CURRENT_SETTINGS_SCHEMA_VERSION
        );
    }

    #[test]
    fn an_old_store_keeps_its_active_prompt() {
        let stored = json!({
            "settings_schema_version": OLD,
            "active_summary_prompt": PLAY_BY_PLAY_ID,
        });
        let (settings, _) = migrated(&stored);
        assert_eq!(settings.active_summary_prompt, PLAY_BY_PLAY_ID);
    }

    #[test]
    fn one_malformed_prompt_entry_keeps_the_other_entries() {
        let stored = json!({
            "settings_schema_version": OLD,
            "endpoint_model": "kept",
            "summary_prompts": [
                { "id": "a", "name": "Team sync", "text": "Recap." },
                { "id": "b", "name": 7 },
                { "id": "c", "name": "One on one", "text": "Notes." },
            ],
        });
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(settings.endpoint_model, "kept");
        let mut shown = names(&settings);
        shown.sort();
        assert_eq!(shown, ["One on one", "Team sync"]);
    }

    #[test]
    fn a_stored_prompt_with_no_title_text_has_an_empty_title_text() {
        // KTD6: the old local store has no title texts. Its prompts are
        // read, not dropped.
        let stored = json!([{ "id": "a", "name": "Team sync", "text": "Recap." }]);
        assert_eq!(
            legacy_prompts(Some(&stored)),
            [summarize::Prompt {
                id: "a".into(),
                name: "Team sync".into(),
                text: "Recap.".into(),
                title_text: String::new(),
            }]
        );
    }

    #[test]
    fn the_prompt_list_shows_the_title_texts_and_the_title_limit() {
        let mut settings = Settings::default();
        settings.summary_prompts.push(SummaryPrompt {
            title_text: "Name the team.".into(),
            ..custom("g", "Team sync")
        });
        let list = settings.summary_prompt_list();
        assert_eq!(
            list.title_text_limit,
            summarize::prompts::TITLE_TEXT_LIMIT as u32
        );
        let topic = summarize::prompts::topic_summary();
        let title_text = |id: &str| {
            let entry = list.prompts.iter().find(|p| p.id == id).unwrap();
            entry.title_text.clone()
        };
        assert_eq!(title_text(TOPIC_SUMMARY_ID), topic.title_text);
        assert_eq!(title_text("g"), "Name the team.");
        // The prompt of a summary run carries the title text.
        settings.active_summary_prompt = "g".into();
        assert_eq!(
            settings.active_summary_prompt_snapshot().title_text,
            "Name the team."
        );
    }

    #[test]
    fn a_stored_prompt_with_a_built_in_or_repeated_id_is_dropped() {
        let stored = json!({
            "settings_schema_version": OLD,
            "summary_prompts": [
                { "id": TOPIC_SUMMARY_ID, "name": "Shadow", "text": "x" },
                { "id": "a", "name": "Team sync", "text": "Recap." },
                { "id": "a", "name": "Again", "text": "x" },
            ],
        });
        let (settings, _) = migrated(&stored);
        assert_eq!(names(&settings), ["Team sync"]);
    }

    #[test]
    fn a_learned_limit_changes_only_local_fields() {
        // `learn_limit` saves with this change. It sets no shared value, so
        // the save does not write the shared settings file (R15).
        let before = Settings::default();
        let mut after = before.clone();
        after.learned_context_limits.insert(
            learned_limit_key("http://localhost:11434/v1", "llama3"),
            8192,
        );
        assert!(shared_patch(&before, &after).is_empty());
        assert_ne!(after.local(), before.local());
    }

    #[test]
    fn a_change_of_a_shared_value_changes_only_the_shared_file() {
        let before = Settings::default();
        let mut after = before.clone();
        after.active_summary_prompt = PLAY_BY_PLAY_ID.into();
        assert_eq!(
            shared_patch(&before, &after),
            [SharedValue::ActiveSummaryPrompt(PLAY_BY_PLAY_ID.into())]
        );
        assert_eq!(after.local(), before.local());

        let mut after = before.clone();
        after.chosen_recognition.live_transcript = false;
        after.chosen_recognition.default_speaker_count =
            Some(SpeakerCountSetting::Range { min: 2, max: 4 });
        after.check_updates_automatically = false;
        after.audio_removal = AudioRemovalSetting::Off;
        assert_eq!(
            shared_patch(&before, &after),
            [
                SharedValue::DefaultSpeakerCount(Some(models::recognition::SpeakerCount::Range {
                    min: 2,
                    max: 4
                })),
                SharedValue::LiveTranscript(false),
                SharedValue::CheckUpdatesAutomatically(false),
                SharedValue::AudioRemoval(AudioRemoval::Off),
            ]
        );
        assert_eq!(after.local(), before.local());
    }

    #[test]
    fn the_audio_removal_setting_is_shared_and_its_range_is_checked() {
        // R1, R2, R3: on by default with 30 days, and only in the shared
        // settings file.
        let before = Settings::default();
        assert_eq!(
            before.audio_removal,
            AudioRemovalSetting::AfterDays { days: 30 }
        );
        let mut after = before.clone();
        after.audio_removal = AudioRemovalSetting::AfterDays { days: 3650 };
        assert_eq!(
            shared_patch(&before, &after),
            [SharedValue::AudioRemoval(AudioRemoval::AfterDays(3650))]
        );
        assert_eq!(after.local(), before.local());
        for days in [1, 3650] {
            assert!(AudioRemovalSetting::AfterDays { days }.validate().is_ok());
        }
        for days in [0, 3651] {
            assert!(AudioRemovalSetting::AfterDays { days }.validate().is_err());
        }
        assert!(AudioRemovalSetting::Off.validate().is_ok());
    }

    #[test]
    fn an_unknown_audio_removal_value_shows_as_the_default_and_as_unknown() {
        use shared_settings::view::{self, Availability};
        let mut raw = shared_settings::file::default_settings();
        raw.insert("audio_removal".into(), json!("never"));
        let cache = shared_settings::cache::Cache {
            source: PathBuf::from("/Volumes/Notes"),
            settings: raw,
            prompts: Vec::new(),
            active_prompt: None,
        };
        let shared = SharedSnapshot {
            view: view::cached(Some(&cache), Availability::Available),
            availability: crate::managers::shared::SharedAvailability::Available,
        };
        let settings = Settings::from_parts(LocalSettings::default(), &shared);
        assert_eq!(settings.audio_removal, AudioRemovalSetting::default());
        assert!(
            settings
                .shared
                .unknown_values
                .iter()
                .any(|v| v.key == "audio_removal" && v.value == "\"never\"")
        );
        assert_eq!(shared.audio_removal_days(), None);
    }

    #[test]
    fn a_change_of_the_recognition_patches_each_changed_value() {
        use models::recognition::{ProcessingMode, RecognitionConfig, SpeakerSource};
        let before = Settings::default();
        let mut after = before.clone();
        after.chosen_recognition = RecognitionChoice::from_config(&RecognitionConfig {
            asr_model: "whisper-large-v3-turbo-q5km".into(),
            speaker_source: SpeakerSource::Sortformer,
            processing_mode: ProcessingMode::AfterStop,
            speaker_count: None,
            language: Some("de".into()),
            speaker_processing_mode: ProcessingMode::AfterStop,
        });
        let keys: Vec<&str> = shared_patch(&before, &after)
            .iter()
            .map(SharedValue::key)
            .collect();
        assert_eq!(
            keys,
            [
                "asr_model",
                "live_transcript",
                "speaker_source",
                "recognition_language"
            ]
        );
        // The chosen configuration is shared: the local store keeps the
        // configuration in use.
        assert_eq!(after.local(), before.local());
    }

    /// R18: the configuration in use stays on this computer. No change of
    /// it is in a shared patch, and the local store keeps it.
    #[test]
    fn the_configuration_in_use_is_never_in_a_shared_patch() {
        use models::recognition::{ProcessingMode, SpeakerSource};
        let before = Settings::default();
        let mut after = before.clone();
        after.set_in_use(&RecognitionChoice {
            asr_model: "whisper-large-v3-turbo-q5km".into(),
            live_transcript: false,
            speaker_source: SpeakerSource::Off,
            speaker_processing_mode: ProcessingMode::AfterStop,
            recognition_language: Some("de".into()),
            default_speaker_count: Some(SpeakerCountSetting::Exact { count: 2 }),
        });
        assert!(shared_patch(&before, &after).is_empty());
        assert!(after.recognition.differs);
        let local = after.local();
        assert_eq!(local.asr_model, "whisper-large-v3-turbo-q5km");
        assert_eq!(local.in_use(), after.in_use());
        // The local store keeps the configuration in use.
        let written = serde_json::to_value(&local).unwrap();
        assert_eq!(settings_from_value(&written), (local, false));
    }

    #[test]
    fn a_shared_value_that_this_version_does_not_know_keeps_the_configuration_in_use() {
        use shared_settings::view::{self, Availability};
        let mut raw = shared_settings::file::default_settings();
        raw.insert("asr_model".into(), json!("model-from-the-future"));
        raw.insert("live_transcript".into(), json!(false));
        raw.insert("schema_version".into(), json!(99));
        let cache = shared_settings::cache::Cache {
            source: PathBuf::from("/vault"),
            settings: raw,
            prompts: Vec::new(),
            active_prompt: None,
        };
        let shared = SharedSnapshot {
            view: view::cached(Some(&cache), Availability::Available),
            ..SharedSnapshot::default()
        };
        let local = LocalSettings {
            asr_model: "whisper-large-v3-turbo-q5km".into(),
            ..LocalSettings::default()
        };
        let settings = Settings::from_parts(local, &shared);
        // The unknown model reads as the model in use, and the known value
        // is the chosen value (R17).
        assert_eq!(settings.asr_model, "whisper-large-v3-turbo-q5km");
        assert_eq!(
            settings.chosen_recognition.asr_model,
            "whisper-large-v3-turbo-q5km"
        );
        assert!(!settings.chosen_recognition.live_transcript);
        assert!(settings.live_transcript);
        assert!(settings.recognition.chosen_not_known);
        assert_eq!(settings.chosen_config(), None);
        assert!(settings.shared.newer_version);
        assert_eq!(
            settings.shared.unknown_values,
            [crate::managers::shared::UnknownSharedValue {
                key: "asr_model".into(),
                value: "\"model-from-the-future\"".into(),
            }]
        );
        // The unknown value is not in a patch unless the user changes it.
        let mut after = settings.clone();
        after.endpoint_model = "gpt-4o".into();
        assert!(shared_patch(&settings, &after).is_empty());
    }

    #[test]
    fn the_list_puts_built_ins_first_and_marks_them_read_only() {
        let mut settings = Settings::default();
        settings.summary_prompts.push(custom("g", "Team sync"));
        settings.active_summary_prompt = "g".into();
        let list = settings.summary_prompt_list();
        let rows: Vec<_> = list
            .prompts
            .iter()
            .map(|p| (p.id.as_str(), p.name.as_str(), p.built_in))
            .collect();
        assert_eq!(
            rows,
            [
                (TOPIC_SUMMARY_ID, "Topic summary", true),
                (PLAY_BY_PLAY_ID, "Play-by-play summary", true),
                ("g", "Team sync", false),
            ]
        );
        assert_eq!(list.prompts[2].text, "Write a recap.");
        assert_eq!(list.active, "g");

        settings.active_summary_prompt = "gone".into();
        assert_eq!(settings.summary_prompt_list().active, TOPIC_SUMMARY_ID);
    }

    #[test]
    fn the_snapshot_falls_back_to_topic_summary_for_an_unknown_id() {
        let mut settings = Settings::default();
        settings.summary_prompts.push(custom("g", "Team sync"));

        settings.active_summary_prompt = "gone".into();
        assert_eq!(
            settings.active_summary_prompt_snapshot(),
            summarize::prompts::built_in(TOPIC_SUMMARY_ID).unwrap()
        );

        settings.active_summary_prompt = "g".into();
        assert_eq!(
            settings.active_summary_prompt_snapshot(),
            summarize::Prompt {
                id: "g".into(),
                name: "Team sync".into(),
                text: "Write a recap.".into(),
                title_text: String::new(),
            }
        );

        settings.active_summary_prompt = PLAY_BY_PLAY_ID.into();
        assert_eq!(
            settings.active_summary_prompt_snapshot().id,
            PLAY_BY_PLAY_ID
        );
    }

    #[test]
    fn the_snapshot_uses_the_last_copy_of_the_active_prompt() {
        // R14: the file of the active prompt is missing or has a conflict
        // copy, so the last copy that this computer read applies.
        let copy = summarize::Prompt {
            id: "g".into(),
            name: "Team sync".into(),
            text: "The last text read.".into(),
            title_text: String::new(),
        };
        let mut settings = Settings {
            active_summary_prompt: "g".into(),
            cached_active_prompt: Some(copy.clone()),
            ..Settings::default()
        };
        assert_eq!(settings.active_summary_prompt_snapshot(), copy);
        settings.summary_prompts.push(custom("g", "Team sync"));
        assert_eq!(settings.active_summary_prompt_snapshot(), copy);
        settings.active_summary_prompt = PLAY_BY_PLAY_ID.into();
        assert_eq!(
            settings.active_summary_prompt_snapshot().id,
            PLAY_BY_PLAY_ID
        );
    }

    #[test]
    fn a_new_store_has_no_saved_base_url_and_uses_the_openai_default() {
        let (settings, _) = migrated(&json!({}));
        assert_eq!(settings.endpoint_base_url, None);
        assert!(!settings.endpoint_base_url_saved());
        assert_eq!(
            settings.effective_endpoint_base_url(),
            "https://api.openai.com/v1"
        );
        assert_eq!(settings.endpoint_model, "");
    }

    #[test]
    fn the_migration_keeps_a_saved_base_url() {
        // Covers AE2.
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION - 1,
            "endpoint_base_url": "http://localhost:11434/v1",
            "endpoint_model": "llama3",
        });
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(
            settings.endpoint_base_url.as_deref(),
            Some("http://localhost:11434/v1")
        );
        assert!(settings.endpoint_base_url_saved());
        assert_eq!(
            settings.effective_endpoint_base_url(),
            "http://localhost:11434/v1"
        );
        let written = serde_json::to_value(settings.local()).unwrap();
        assert_eq!(settings_from_value(&written), (settings.local(), false));
    }

    #[test]
    fn an_existing_store_keeps_an_empty_base_url() {
        // The OpenAI default applies only to a new install (R2).
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION - 1,
            "endpoint_base_url": "",
            "endpoint_model": "gpt-4o",
        });
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(settings.endpoint_base_url.as_deref(), Some(""));
        assert_eq!(settings.effective_endpoint_base_url(), "");
        assert_eq!(settings.endpoint_model, "gpt-4o");
        let written = serde_json::to_value(settings.local()).unwrap();
        assert_eq!(settings_from_value(&written), (settings.local(), false));
    }

    #[test]
    fn a_current_store_keeps_a_cleared_base_url() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "endpoint_base_url": "",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(settings.endpoint_base_url.as_deref(), Some(""));
    }

    #[test]
    fn an_empty_base_url_from_the_setter_is_saved_and_clears_the_endpoint() {
        let mut settings = Settings::default();
        settings.set_endpoint_model("gpt-4o");
        settings.set_endpoint_base_url("   ").unwrap();
        assert_eq!(settings.endpoint_base_url.as_deref(), Some(""));
        assert!(settings.endpoint_base_url_saved());
        assert_eq!(settings.effective_endpoint_base_url(), "");
        assert!(!settings.endpoint_configured());
    }

    #[test]
    fn the_base_url_setter_trims_and_changes_only_the_base_url() {
        let mut settings = Settings {
            endpoint_model: "gpt-4o".into(),
            ..Settings::default()
        };
        settings
            .set_endpoint_base_url("  http://localhost:11434/v1 ")
            .unwrap();
        assert_eq!(
            settings.endpoint_base_url.as_deref(),
            Some("http://localhost:11434/v1")
        );
        assert_eq!(settings.endpoint_model, "gpt-4o");
    }

    #[test]
    fn the_model_setter_does_not_save_a_base_url() {
        let mut settings = Settings::default();
        settings.set_endpoint_model("  gpt-4o ");
        assert_eq!(settings.endpoint_model, "gpt-4o");
        assert_eq!(settings.endpoint_base_url, None);
        assert!(!settings.endpoint_base_url_saved());
        assert!(settings.endpoint_configured());
    }

    #[test]
    fn the_base_url_setter_refuses_a_url_with_no_host_and_changes_nothing() {
        let mut settings = Settings::default();
        let before = settings.clone();
        let error = settings.set_endpoint_base_url("not a url").unwrap_err();
        assert_eq!(
            error,
            secrets::endpoint_key("not a url").unwrap_err().to_string()
        );
        assert_eq!(settings, before);
    }

    #[test]
    fn the_salvage_path_keeps_a_valid_base_url() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "context_limit_tokens": "lots",
            "endpoint_base_url": "http://localhost:11434/v1",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(
            settings.endpoint_base_url.as_deref(),
            Some("http://localhost:11434/v1")
        );
    }

    #[test]
    fn the_gate_sends_a_request_only_for_an_endpoint_the_user_acted_on() {
        use EndpointGate::*;
        use KeyState::*;
        use NotRequestedReason::*;
        let default = DEFAULT_ENDPOINT_BASE_URL;
        let local = "http://localhost:11434/v1";
        assert_eq!(endpoint_gate(default, false, Saved), RequestWithKey);
        assert_eq!(
            endpoint_gate(default, false, Absent),
            NotRequested(KeyNeeded)
        );
        // The user can save the default URL, and then it counts as entered.
        assert_eq!(endpoint_gate(default, true, Absent), RequestWithoutKey);
        assert_eq!(endpoint_gate(local, true, Absent), RequestWithoutKey);
        assert_eq!(endpoint_gate(local, true, Saved), RequestWithKey);
        assert_eq!(
            endpoint_gate(local, true, Locked),
            NotRequested(KeychainLocked)
        );
        assert_eq!(
            endpoint_gate(default, false, Failed),
            NotRequested(KeychainFailed)
        );
        for key in [Saved, Absent, Locked, Failed] {
            assert_eq!(endpoint_gate("  ", true, key), NotRequested(NoBaseUrl));
        }
    }

    #[test]
    fn a_blank_saved_key_counts_as_no_key() {
        assert_eq!(key_state(&Ok(Some("sk-1".into()))), KeyState::Saved);
        assert_eq!(key_state(&Ok(Some("   ".into()))), KeyState::Absent);
        assert_eq!(key_state(&Ok(Some(String::new()))), KeyState::Absent);
        assert_eq!(key_state(&Ok(None)), KeyState::Absent);
        // A blank key does not open the gate for the default Base URL.
        let key = key_state(&Ok(Some(" \t".into())));
        assert_eq!(
            endpoint_gate(DEFAULT_ENDPOINT_BASE_URL, false, key),
            EndpointGate::NotRequested(NotRequestedReason::KeyNeeded)
        );
    }

    #[test]
    fn readiness_applies_the_same_gate_as_summary_processing() {
        use KeyState::*;
        let default_url = Settings {
            endpoint_model: "gpt-4o".into(),
            ..Settings::default()
        };
        // A model with no key for the default Base URL: processing refuses.
        assert!(!endpoint_ready(&default_url, Absent));
        assert!(endpoint_ready(&default_url, Saved));
        // A Keychain problem has its own message. It is not "unconfigured".
        assert!(endpoint_ready(&default_url, Locked));
        assert!(endpoint_ready(&default_url, Failed));

        let local = Settings {
            endpoint_base_url: Some("http://localhost:11434/v1".into()),
            ..default_url.clone()
        };
        assert!(endpoint_ready(&local, Absent));

        let cleared = Settings {
            endpoint_base_url: Some(String::new()),
            ..default_url.clone()
        };
        let no_model = Settings {
            endpoint_model: String::new(),
            ..local.clone()
        };
        for key in [Saved, Absent, Locked, Failed] {
            assert!(!endpoint_ready(&cleared, key));
            assert!(!endpoint_ready(&no_model, key));
        }
    }
}

#[cfg(test)]
mod recognition_tests {
    use super::*;
    use models::recognition::{ProcessingMode, SpeakerSource};

    #[test]
    fn old_live_choice_does_not_enable_live_speaker_analysis() {
        for live in [true, false] {
            let (settings, changed) = migrated(&serde_json::json!({
                "settings_schema_version":4,"asr_model":"parakeet-unified-en-0.6b-q80","live_transcript":live
            }));
            assert!(changed);
            assert_eq!(settings.live_transcript, live);
            assert_eq!(settings.speaker_source, SpeakerSource::Pyannote);
            assert_eq!(
                settings.recognition_config().speaker_processing_mode,
                ProcessingMode::AfterStop
            );
        }
    }

    #[test]
    fn live_speaker_analysis_does_not_enable_live_transcription() {
        let settings = Settings {
            asr_model: "whisper-large-v3-turbo-q5km".into(),
            live_transcript: false,
            speaker_source: SpeakerSource::Sortformer,
            speaker_processing_mode: ProcessingMode::DuringRecording,
            ..Settings::default()
        };
        let spec = models::recognition::resolve(&settings.recognition_config()).unwrap();
        assert!(!spec.enabled_live_transcription);
        assert!(spec.enabled_live_speaker_analysis);
    }
}

#[cfg(test)]
mod stale_recognition_tests {
    use serde_json::json;

    use super::*;
    use models::recognition::{ProcessingMode, SpeakerSource};

    const GRANITE: &str = "granite-speech-4.1-2b-plus-q5km";

    fn at(version: u64, fields: Value) -> Value {
        let mut stored = json!({ "settings_schema_version": version });
        for (key, value) in fields.as_object().unwrap() {
            stored[key] = value.clone();
        }
        stored
    }

    fn current(fields: Value) -> Value {
        at(u64::from(CURRENT_SETTINGS_SCHEMA_VERSION), fields)
    }

    /// A store of the last version before the shared settings.
    fn old(fields: Value) -> Value {
        at(SHARED_SETTINGS_VERSION - 1, fields)
    }

    #[test]
    fn a_removed_model_reads_as_parakeet_unified() {
        let (settings, updated) = migrated(&old(json!({ "asr_model": GRANITE })));
        assert!(updated);
        assert_eq!(settings.asr_model, models::DEFAULT_ASR_MODEL);
    }

    #[test]
    fn the_removed_speaker_source_reads_as_the_automatic_choice() {
        let (settings, updated) = migrated(&old(json!({
            "asr_model": GRANITE,
            "speaker_source": "integrated",
        })));
        assert!(updated);
        // A salvage resets the source to pyannote, so Nemotron shows that
        // the rewrite ran before the parse.
        assert_eq!(settings.speaker_source, SpeakerSource::Nemotron);
        // Settings shows the first preference as "Automatic".
        let automatic = models::model(&settings.asr_model)
            .and_then(|m| m.speaker_preferences.as_ref())
            .map(|p| p.after_stop[0]);
        assert_eq!(automatic, Some(settings.speaker_source));
    }

    #[test]
    fn the_removed_speaker_source_with_more_than_eight_speakers_reads_as_pyannote() {
        let (settings, updated) = migrated(&old(json!({
            "speaker_source": "integrated",
            "default_speaker_count": { "kind": "exact", "count": 9 },
        })));
        assert!(updated);
        assert_eq!(settings.speaker_source, SpeakerSource::Pyannote);
        assert_eq!(
            settings.default_speaker_count,
            Some(SpeakerCountSetting::Exact { count: 9 })
        );
    }

    #[test]
    fn a_configuration_in_use_in_a_current_store_gets_the_same_rewrite() {
        let (settings, updated) = settings_from_value(&current(json!({
            "asr_model": GRANITE,
            "speaker_source": "integrated",
            "recognition_language": "en",
        })));
        assert!(updated);
        assert_eq!(settings.asr_model, models::DEFAULT_ASR_MODEL);
        assert_eq!(settings.speaker_source, SpeakerSource::Nemotron);
        assert_eq!(settings.recognition_language.as_deref(), Some("en"));

        let (settings, updated) = settings_from_value(&current(json!({
            "speaker_source": "integrated",
            "default_speaker_count": { "kind": "exact", "count": 9 },
        })));
        assert!(updated);
        assert_eq!(settings.speaker_source, SpeakerSource::Pyannote);
        assert_eq!(
            settings.default_speaker_count,
            Some(SpeakerCountSetting::Exact { count: 9 })
        );

        // A count that is not valid is dropped.
        let (settings, updated) = settings_from_value(&current(json!({
            "default_speaker_count": { "kind": "exact", "count": 0 },
        })));
        assert!(updated);
        assert_eq!(settings.default_speaker_count, None);
    }

    #[test]
    fn kept_values_are_not_changed_and_not_marked_updated() {
        let stored = current(json!({
            "asr_model": "parakeet-tdt-0.6b-v3-q80",
            "speaker_source": "pyannote",
            "default_speaker_count": { "kind": "range", "min": 2, "max": 4 },
        }));
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(settings.asr_model, "parakeet-tdt-0.6b-v3-q80");
        assert_eq!(
            settings.default_speaker_count,
            Some(SpeakerCountSetting::Range { min: 2, max: 4 })
        );

        let legacy = legacy_shared(&old(json!({
            "asr_model": "whisper-large-v3-turbo-q5km",
            "speaker_source": "sortformer",
            "default_speaker_count": { "kind": "range", "min": 2, "max": 4 },
        })))
        .unwrap();
        let fields = shared_settings::file::SharedFields::from_raw(&legacy.settings);
        assert_eq!(
            fields.asr_model,
            Field::Known("whisper-large-v3-turbo-q5km".into())
        );
        assert_eq!(
            fields.speaker_source,
            Field::Known(SpeakerSource::Sortformer)
        );
    }

    #[test]
    fn the_rewrite_keeps_other_fields_and_converges() {
        let stored = old(json!({
            "interface_zoom_percent": 120,
            "endpoint_base_url": "http://localhost:11434/v1",
            "endpoint_model": "llama3",
            "default_speaker_count": { "kind": "exact", "count": 3 },
            "asr_model": GRANITE,
            "live_transcript": false,
            "speaker_source": "integrated",
            "speaker_processing_mode": "after_stop",
            "recognition_language": "de",
            "system_audio_default": true,
            "destination_roots": ["/vault/a"],
            "current_destination": "/vault/a",
        }));
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        let recognition = RecognitionChoice {
            asr_model: models::DEFAULT_ASR_MODEL.into(),
            live_transcript: false,
            speaker_source: SpeakerSource::Nemotron,
            speaker_processing_mode: ProcessingMode::AfterStop,
            recognition_language: Some("de".into()),
            default_speaker_count: Some(SpeakerCountSetting::Exact { count: 3 }),
        };
        let mut expected = Settings {
            interface_zoom_percent: 120,
            endpoint_base_url: Some("http://localhost:11434/v1".into()),
            endpoint_model: "llama3".into(),
            system_audio_default: true,
            storage_folder: "/vault/a".into(),
            chosen_recognition: recognition.clone(),
            ..Settings::default()
        };
        expected.set_in_use(&recognition);
        // The chosen configuration and the configuration in use are the
        // same after the update, so nothing downloads (R18).
        assert!(!expected.recognition.differs);
        assert_eq!(settings, expected);
        let written = serde_json::to_value(settings.local()).unwrap();
        assert_eq!(settings_from_value(&written), (settings.local(), false));
    }
}
