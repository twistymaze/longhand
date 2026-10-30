//! Settings in the application's own configuration store (R33, KTD15), and
//! the settings that the app uses: the local settings and the shared view
//! (KTD7).
//!
//! Shape follows Handy's `settings.rs` (one `#[serde(default)]` struct, a
//! schema-version integer, migrations keyed on the stored version, salvage of
//! individually valid fields); see src-tauri/PROVENANCE.md. Portable mode is
//! omitted. Credentials are not settings: they live in the OS secret store,
//! keyed by provider and host (see `managers::secrets`, KTD8).
//!
//! The local store holds only the settings that stay on this computer
//! (R13). The shared settings and the custom prompts come from the shared
//! settings manager (`managers::shared`). [`Settings`] joins both.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use shared_settings::file::{AudioRemoval, Field, SharedValue, is_catalog_asr_model};
use shared_settings::view::ActivePrompt;
use specta::Type;
use summarize::Provider;
use summarize::prompts::TOPIC_SUMMARY_ID;
use summarize::provider::ThinkingLevel;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::managers::local_store;
use crate::managers::shared::{
    SharedAvailability, SharedSettingsManager, SharedSnapshot, SharedState,
};

/// The key of the local settings in the local settings file.
const SETTINGS_KEY: &str = "settings";

pub const CURRENT_SETTINGS_SCHEMA_VERSION: u32 = 7;

/// The local store version that moved the shared settings to the storage
/// folder (R12, KTD7). A store below it holds the old shared values.
const SHARED_SETTINGS_VERSION: u64 = 6;

/// The local store version that replaced the one endpoint with the active
/// provider and an entry for each provider (KTD6). A store below it holds
/// the old endpoint fields.
const PROVIDER_SETTINGS_VERSION: u64 = 7;

/// The keys of the old endpoint in a local store from before version 7.
const LEGACY_ENDPOINT_BASE_URL: &str = "endpoint_base_url";
const LEGACY_ENDPOINT_MODEL: &str = "endpoint_model";
const LEGACY_CONTEXT_LIMIT: &str = "context_limit_tokens";

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

/// An app on the app list (R1, R5, KTD1): the app ID, the name that
/// Settings shows, and the path that the icon comes from when the app does
/// not run.
#[derive(Serialize, Deserialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct ListedApp {
    pub id: String,
    pub name: String,
    pub path: String,
}

impl From<audio_system::apps::App> for ListedApp {
    fn from(app: audio_system::apps::App) -> Self {
        Self {
            id: app.id,
            name: app.name,
            path: app.path,
        }
    }
}

impl From<&ListedApp> for audio_system::apps::App {
    fn from(app: &ListedApp) -> Self {
        Self {
            id: app.id.clone(),
            name: app.name.clone(),
            path: app.path.clone(),
        }
    }
}

/// The app list as the store holds it. An entry that does not parse or has
/// no ID is dropped, and the other entries stay (KTD8).
fn valid_apps<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<ListedApp>, D::Error> {
    let Value::Array(items) = Value::deserialize(d)? else {
        log::warn!("the stored app list is not a list; it reads as empty");
        return Ok(Vec::new());
    };
    let mut apps: Vec<ListedApp> = Vec::new();
    for item in items {
        match serde_json::from_value::<ListedApp>(item) {
            Ok(app) if !app.id.trim().is_empty() && !apps.iter().any(|a| a.id == app.id) => {
                apps.push(app)
            }
            Ok(app) if !app.id.trim().is_empty() => {}
            _ => log::warn!("dropping an invalid app list entry"),
        }
    }
    Ok(apps)
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
            // A stored prompt never holds the Background (KTD6).
            background: String::new(),
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
    /// The prompt text or the title instructions are over their limit, for
    /// example after an edit outside Longhand (R21). A run uses the whole
    /// text. Rename and delete stay available.
    pub over_limit: bool,
    /// The file is over the size limit that Longhand reads, so the list has
    /// no texts of the prompt (R21). The texts are empty.
    pub too_large: bool,
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

/// The remembered settings of one AI provider on this computer (R10, KTD6).
#[derive(Serialize, Deserialize, Type, Debug, Clone, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ProviderEntry {
    /// The address that the user entered. `None` uses the usual address
    /// of the provider ([`Provider::default_address`]). Only a local
    /// provider and Custom have one (R8, R9). Custom with `None` has no
    /// address.
    pub address: Option<String>,
    /// The model that summaries use. Empty until the user picks one (R17).
    pub model: String,
    /// The context limit that the user entered. It overrides the limit
    /// that the provider reports (R15).
    pub context_limit_tokens: Option<u32>,
    /// The Thinking level that the user picked (R5). `None` uses the
    /// default of the provider kind, which is never saved (R6, KTD7).
    pub thinking: Option<ThinkingLevel>,
    /// How many requests go to the provider at the same time, as the user
    /// set it (R12). `None` uses the default of the provider kind (KTD7).
    pub requests_at_once: Option<u32>,
}

/// The range of the Requests at the same time setting (R12).
pub const REQUESTS_AT_ONCE: std::ops::RangeInclusive<u32> = 1..=8;

impl ProviderEntry {
    /// The Thinking level that requests to `provider` use, and whether the
    /// user chose it. A level that is not chosen is the default of the
    /// provider kind (R6, KTD7).
    pub fn effective_thinking(&self, provider: Provider) -> (ThinkingLevel, bool) {
        match self.thinking {
            Some(level) => (level, true),
            None => (provider.default_thinking(), false),
        }
    }

    /// How many requests go to `provider` at the same time (R12).
    pub fn effective_requests_at_once(&self, provider: Provider) -> u32 {
        self.requests_at_once
            .unwrap_or_else(|| provider.default_requests_at_once())
    }
}

/// One [`ProviderEntry`] for each provider (R10, KTD6). Each field name is
/// the [`Provider::name`] of its provider.
#[derive(Serialize, Deserialize, Type, Debug, Clone, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ProviderEntries {
    pub openai: ProviderEntry,
    pub anthropic: ProviderEntry,
    pub gemini: ProviderEntry,
    pub openrouter: ProviderEntry,
    pub ollama: ProviderEntry,
    pub lm_studio: ProviderEntry,
    pub custom: ProviderEntry,
}

impl ProviderEntries {
    pub fn get(&self, provider: Provider) -> &ProviderEntry {
        match provider {
            Provider::OpenAi => &self.openai,
            Provider::Anthropic => &self.anthropic,
            Provider::Gemini => &self.gemini,
            Provider::OpenRouter => &self.openrouter,
            Provider::Ollama => &self.ollama,
            Provider::LmStudio => &self.lm_studio,
            Provider::Custom => &self.custom,
        }
    }

    pub fn get_mut(&mut self, provider: Provider) -> &mut ProviderEntry {
        match provider {
            Provider::OpenAi => &mut self.openai,
            Provider::Anthropic => &mut self.anthropic,
            Provider::Gemini => &mut self.gemini,
            Provider::OpenRouter => &mut self.openrouter,
            Provider::Ollama => &mut self.ollama,
            Provider::LmStudio => &mut self.lm_studio,
            Provider::Custom => &mut self.custom,
        }
    }
}

/// Where the Keychain holds the API key of one provider (KTD8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLocation {
    pub provider: Provider,
    /// The address of the provider. The account uses its host.
    pub address: String,
    /// The provider that the settings migration chose. Its key can still
    /// be in the old host-only entry (R23).
    pub legacy: bool,
}

impl KeyLocation {
    /// Read the key. The read moves an old host-only entry when `legacy`
    /// is set (KTD8).
    pub fn read<B: secrets::Backend>(
        &self,
        store: &secrets::SecretStore<B>,
    ) -> Result<Option<String>, secrets::SecretError> {
        store.provider_credential(self.provider.name(), &self.address, self.legacy)
    }

    pub fn save<B: secrets::Backend>(
        &self,
        store: &secrets::SecretStore<B>,
        value: &str,
    ) -> Result<(), secrets::SecretError> {
        store.set_provider_credential(self.provider.name(), &self.address, value)
    }

    /// Delete the key, and the old host-only entry when `legacy` is set,
    /// so a removed key cannot come back (KTD8).
    pub fn delete<B: secrets::Backend>(
        &self,
        store: &secrets::SecretStore<B>,
    ) -> Result<(), secrets::SecretError> {
        store.delete_provider_credential(self.provider.name(), &self.address, self.legacy)
    }

    /// The host part of the account, as the Settings pane shows it.
    pub fn host(&self) -> String {
        host_of(&self.address)
    }
}

/// The host part of `address`, or `address` itself when it is not a valid
/// URL.
pub fn host_of(address: &str) -> String {
    secrets::endpoint_key(address).unwrap_or_else(|_| address.to_string())
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
    pub pause_shortcut: Option<String>,
    pub widget_shortcut: Option<String>,
    pub storage_folder: String,
    /// The AI provider that summaries use (R7). `None` sends no summary
    /// request (R11).
    pub active_provider: Option<Provider>,
    pub providers: ProviderEntries,
    /// The provider whose key can still be in the old host-only Keychain
    /// entry (KTD8, R23). The version 7 migration sets it.
    pub legacy_key_provider: Option<Provider>,
    /// The recognition configuration in use on this computer (R18, KTD14).
    /// It is never written to the shared settings.
    pub asr_model: String,
    pub live_transcript: bool,
    pub speaker_source: models::recognition::SpeakerSource,
    pub speaker_processing_mode: models::recognition::ProcessingMode,
    pub recognition_language: Option<String>,
    pub default_speaker_count: Option<SpeakerCountSetting>,
    pub microphone: Option<MicrophoneChoice>,
    /// The apps whose audio each new note records (R1, KTD8). App IDs
    /// differ by platform, so the list stays on this computer.
    #[serde(deserialize_with = "valid_apps")]
    pub apps: Vec<ListedApp>,
    /// Whether a recording that stops and cannot restart plays the alert
    /// sound (R9). Local to this computer (KTD1).
    pub alert_sound: bool,
    pub hide_from_dock: bool,
    pub keep_running_in_tray: bool,
    /// The watched folder of this computer (R4), or `None` when it is off.
    /// It is never written to the shared settings.
    pub watched_folder: Option<String>,
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
#[derive(Serialize, Deserialize, Type, Debug, Clone, PartialEq, Eq)]
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
    /// The choice with the language and speaker timing that its model and
    /// speaker source use. The configuration in use stores this form, so an
    /// older build on this computer, which applies no fallback, can resolve
    /// it.
    pub fn usable(&self) -> Self {
        let config = self.config();
        Self {
            speaker_processing_mode: config.speaker_processing_mode,
            recognition_language: config.language,
            ..self.clone()
        }
    }

    /// Whether the two choices record with the same configuration.
    pub fn same_use(&self, other: &Self) -> bool {
        self.usable() == other.usable()
    }

    /// The configuration that recognition resolves. Live transcript, live
    /// speaker recognition and the language are preferences: each one
    /// applies only when the model and the speaker source can use it, and
    /// otherwise gives After Stop or automatic language. The choice keeps
    /// the preference for a model that can use it.
    pub fn config(&self) -> models::recognition::RecognitionConfig {
        use models::recognition::{ProcessingMode, RecognitionConfig};
        let model = models::model(&self.asr_model);
        let live_supported = model.is_some_and(|m| m.streaming);
        let language = self
            .recognition_language
            .clone()
            .filter(|language| model.is_none_or(|m| m.languages.contains(language)));
        RecognitionConfig {
            asr_model: self.asr_model.clone(),
            speaker_source: self.speaker_source,
            processing_mode: if self.live_transcript && live_supported {
                ProcessingMode::DuringRecording
            } else {
                ProcessingMode::AfterStop
            },
            speaker_count: self.default_speaker_count.map(Into::into),
            language,
            speaker_processing_mode: if self.speaker_source.has_live_mode() {
                self.speaker_processing_mode
            } else {
                ProcessingMode::AfterStop
            },
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
    /// The chosen configuration differs from the configuration in use. A
    /// chosen configuration with a value that this version does not know
    /// always differs, so Settings tells the user to update.
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
    /// The chord that pauses the recording note and continues the paused
    /// note. Off by default (R16).
    pub pause_shortcut: Option<String>,
    /// The chord that shows or hides the recording widget for the open
    /// note. Off by default (R13).
    pub widget_shortcut: Option<String>,
    /// The one storage folder (R1). The library lists the notes in it, and
    /// the next note saves to it. Empty only until the launch sets the
    /// default folder (R2, KTD12).
    pub storage_folder: String,
    /// The AI provider that summaries use (R7). `None` sends no summary
    /// request, and a new install has none (R11).
    pub active_provider: Option<Provider>,
    /// The remembered address, model and context limit of each provider on
    /// this computer (R10, R13, R15).
    pub providers: ProviderEntries,
    /// The provider whose key can still be in the old host-only Keychain
    /// entry (KTD8, R23).
    #[serde(skip)]
    pub legacy_key_provider: Option<Provider>,
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
    /// The app list: the apps whose audio each new note records (R1). Empty
    /// on a new install (KD3).
    pub apps: Vec<ListedApp>,
    /// A shared setting. What New Note and the recording shortcut do (R8,
    /// R10): start a note that records, or open a note that does not
    /// record. On by default.
    pub new_note_records: bool,
    /// Whether a recording that stops and cannot restart plays the alert
    /// sound (R9, KTD1). On by default. Test Sound plays when it is off.
    pub alert_sound: bool,
    /// macOS only: the app has no Dock icon while no window is visible
    /// (R2, R3). Off by default: the Dock icon stays, as is usual on macOS.
    pub hide_from_dock: bool,
    /// Windows only: closing the main window hides it, and the app keeps
    /// running in the system tray. Off by default: closing the window quits,
    /// as is usual on Windows. A recording keeps the app running either way.
    pub keep_running_in_tray: bool,
    /// The watched folder of this computer (R4), or `None` when it is off
    /// (the default). Each new audio file in it becomes a note (R5).
    pub watched_folder: Option<String>,
    /// A shared setting. Whether a new note shows the recording widget: the
    /// pill that is on screen while a note is open (R11). On by default. A
    /// change applies to the next note (R14).
    pub show_recording_widget: bool,
    /// A shared setting. A release build checks for a newer release at
    /// launch and one hour after each successful check (R9, R10). On by
    /// default. "Check for Updates…" works when it is off (R11).
    pub check_updates_automatically: bool,
    /// A shared setting. When the audio of a saved note goes (R1, R2). On
    /// by default, with a delay of 30 days. A value that this version does
    /// not know shows as the default here and in `shared.unknown_values`.
    /// The removal pass does not read this field: it reads
    /// `SharedSnapshot::audio_removal_days`, which gives no delay for such a
    /// value or while the shared settings are read-only.
    pub audio_removal: AudioRemovalSetting,
    /// A shared setting. The custom summary prompts, in the order that
    /// Settings shows them (KTD6).
    pub summary_prompts: Vec<SummaryPrompt>,
    /// A shared setting. The id of the prompt that the next summary uses.
    /// An id that matches no prompt resolves to the Topic summary.
    pub active_summary_prompt: String,
    /// Whether this computer can change the shared settings now, and the
    /// values that this version does not know (R16, R17).
    pub shared: SharedState,
    /// A shared setting. The Background text that each summary uses (R1,
    /// R7). It is the cached text while the Background file or the storage
    /// folder is not available (KTD3).
    pub background: String,
    /// The most characters the Background can have (R6).
    pub background_limit: u32,
    /// Why the Background cannot change now, as UI text. `None` when
    /// this computer can change it (R5, KTD3).
    pub background_read_only: Option<String>,
    /// The last copy of the active custom prompt that this computer read.
    /// A summary uses it while the prompt's file is missing or has a
    /// conflict copy (R14).
    #[serde(skip)]
    pub cached_active_prompt: Option<summarize::Prompt>,
    /// A shared setting. The summary prompt choice of each note type: the
    /// type name, with "General" for General, and the prompt id (KTD1).
    /// The Types and Tags pane reads the choices from the type list.
    #[serde(skip)]
    pub note_type_prompts: std::collections::BTreeMap<String, String>,
    /// The prompt of each id that `note_type_prompts` names, by id: the
    /// folder copy, or the last copy that this computer read (R7, KTD2).
    /// A prompt that this computer never read is not here.
    #[serde(skip)]
    pub type_prompts: std::collections::BTreeMap<String, summarize::Prompt>,
    /// The ids of the custom prompts whose files are over the size limit
    /// that Longhand reads (R21).
    #[serde(skip)]
    pub too_large_prompts: std::collections::BTreeSet<String>,
}

/// The most characters the Background can have (R6).
const BACKGROUND_LIMIT: u32 = summarize::prompts::BACKGROUND_TEXT_LIMIT as u32;

/// Why the Background cannot change now, as UI text, or `None` when
/// this computer can change it. The shared settings are read-only, or the
/// Background file cannot be used (R5, KTD3).
fn background_read_only(shared: &SharedSnapshot) -> Option<String> {
    match &shared.availability {
        // While only the settings file does not parse, the Background can
        // change (R22).
        SharedAvailability::ReadOnly { cause } if !shared.files_writable() => {
            Some(cause.message().to_string())
        }
        // A Background file that links outside the storage folder is not
        // read, and a save replaces the link, so the Background can change.
        SharedAvailability::ReadOnly { .. } | SharedAvailability::Available => shared
            .view
            .background_unavailable
            .as_ref()
            .filter(|_| !shared.view.background_writable())
            .map(ToString::to_string),
    }
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

/// The state of the saved key of a provider for its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Saved,
    Absent,
    /// Keychain is locked or refused access. The user can repair it (R16).
    Locked,
    /// Keychain cannot be read for another reason.
    Failed,
}

/// Why Longhand sends no request to a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRequestedReason {
    /// No provider is active (R7, R11).
    NoProvider,
    /// Custom has no address (R9).
    NoAddress,
    /// A local provider or Custom with no key that the user did not choose
    /// (R12).
    NotChosen,
    KeychainLocked,
    KeychainFailed,
    /// A cloud provider with no saved key (R8, R12).
    KeyNeeded,
}

/// Whether Longhand can send a request to a provider, and with which key.
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

/// The request gate for one provider (R12). Longhand contacts only a
/// provider that the user acted on: a key is saved for it, or the user
/// chose a local provider or Custom, which need no key. `chosen` is true
/// when `provider` is the active provider. A provider that takes no key
/// (R8) has `key` absent. The connection check and summary processing
/// both call it.
pub fn provider_gate(
    provider: Option<Provider>,
    chosen: bool,
    address: &str,
    key: KeyState,
) -> EndpointGate {
    let Some(provider) = provider else {
        return EndpointGate::NotRequested(NotRequestedReason::NoProvider);
    };
    if address.trim().is_empty() {
        return EndpointGate::NotRequested(NotRequestedReason::NoAddress);
    }
    match key {
        KeyState::Locked => EndpointGate::NotRequested(NotRequestedReason::KeychainLocked),
        KeyState::Failed => EndpointGate::NotRequested(NotRequestedReason::KeychainFailed),
        KeyState::Saved => EndpointGate::RequestWithKey,
        KeyState::Absent if provider.is_cloud() => {
            EndpointGate::NotRequested(NotRequestedReason::KeyNeeded)
        }
        KeyState::Absent if chosen => EndpointGate::RequestWithoutKey,
        KeyState::Absent => EndpointGate::NotRequested(NotRequestedReason::NotChosen),
    }
}

/// Whether summary processing can use the active provider in `s`, given
/// the state of its key. Readiness reports this value, so it applies the
/// same gate as summary processing. A locked or failed Keychain read
/// counts as configured: the Keychain problem has its own message.
pub fn provider_ready(s: &Settings, key: KeyState) -> bool {
    let Some(provider) = s.active_provider else {
        return false;
    };
    s.provider_configured()
        && matches!(
            provider_gate(Some(provider), true, &s.provider_address(provider), key),
            EndpointGate::RequestWithKey
                | EndpointGate::RequestWithoutKey
                | EndpointGate::NotRequested(
                    NotRequestedReason::KeychainLocked | NotRequestedReason::KeychainFailed
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
            pause_shortcut: None,
            widget_shortcut: None,
            storage_folder: String::new(),
            active_provider: None,
            providers: ProviderEntries::default(),
            legacy_key_provider: None,
            default_speaker_count: None,
            asr_model: models::DEFAULT_ASR_MODEL.to_string(),
            live_transcript: true,
            speaker_source: Default::default(),
            speaker_processing_mode: Default::default(),
            recognition_language: None,
            chosen_recognition: RecognitionChoice::default(),
            recognition: RecognitionState::default(),
            microphone: None,
            apps: Vec::new(),
            new_note_records: true,
            alert_sound: true,
            hide_from_dock: false,
            keep_running_in_tray: false,
            watched_folder: None,
            show_recording_widget: true,
            check_updates_automatically: true,
            audio_removal: AudioRemovalSetting::default(),
            summary_prompts: Vec::new(),
            active_summary_prompt: TOPIC_SUMMARY_ID.to_string(),
            shared: SharedState::default(),
            background: String::new(),
            background_limit: BACKGROUND_LIMIT,
            background_read_only: background_read_only(&SharedSnapshot::default()),
            cached_active_prompt: None,
            note_type_prompts: Default::default(),
            type_prompts: Default::default(),
            too_large_prompts: Default::default(),
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
            pause_shortcut: local.pause_shortcut,
            widget_shortcut: local.widget_shortcut,
            storage_folder: local.storage_folder,
            active_provider: local.active_provider,
            providers: local.providers,
            legacy_key_provider: local.legacy_key_provider,
            default_speaker_count: local.default_speaker_count,
            asr_model: local.asr_model,
            live_transcript: local.live_transcript,
            speaker_source: local.speaker_source,
            speaker_processing_mode: local.speaker_processing_mode,
            recognition_language: local.recognition_language,
            recognition: RecognitionState {
                differs: chosen_not_known || !chosen_recognition.same_use(&in_use),
                chosen_not_known,
                ..RecognitionState::default()
            },
            chosen_recognition,
            microphone: local.microphone,
            apps: local.apps,
            new_note_records: known_or(&fields.new_note_records, defaults.new_note_records),
            alert_sound: local.alert_sound,
            hide_from_dock: local.hide_from_dock,
            keep_running_in_tray: local.keep_running_in_tray,
            watched_folder: local.watched_folder,
            show_recording_widget: known_or(
                &fields.show_recording_widget,
                defaults.show_recording_widget,
            ),
            check_updates_automatically: known_or(
                &fields.check_updates_automatically,
                defaults.check_updates_automatically,
            ),
            audio_removal: known_or(&fields.audio_removal, AudioRemoval::default()).into(),
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
            background: shared.view.background.clone(),
            background_limit: BACKGROUND_LIMIT,
            background_read_only: background_read_only(shared),
            cached_active_prompt: match &shared.view.active {
                ActivePrompt::Custom {
                    prompt,
                    from_cache: true,
                } => Some(prompt.clone()),
                _ => None,
            },
            note_type_prompts: fields.type_prompt_map(),
            type_prompts: shared
                .view
                .type_prompts
                .iter()
                .filter_map(|(id, prompt)| Some((id.clone(), prompt.prompt()?.clone())))
                .collect(),
            too_large_prompts: shared.view.too_large_prompts.clone(),
        }
    }

    /// The local fields, as the local store holds them.
    pub fn local(&self) -> LocalSettings {
        LocalSettings {
            settings_schema_version: self.settings_schema_version,
            interface_zoom_percent: self.interface_zoom_percent,
            shortcut: self.shortcut.clone(),
            show_window_shortcut: self.show_window_shortcut.clone(),
            pause_shortcut: self.pause_shortcut.clone(),
            widget_shortcut: self.widget_shortcut.clone(),
            storage_folder: self.storage_folder.clone(),
            active_provider: self.active_provider,
            providers: self.providers.clone(),
            legacy_key_provider: self.legacy_key_provider,
            asr_model: self.asr_model.clone(),
            live_transcript: self.live_transcript,
            speaker_source: self.speaker_source,
            speaker_processing_mode: self.speaker_processing_mode,
            recognition_language: self.recognition_language.clone(),
            default_speaker_count: self.default_speaker_count,
            microphone: self.microphone.clone(),
            apps: self.apps.clone(),
            alert_sound: self.alert_sound,
            hide_from_dock: self.hide_from_dock,
            keep_running_in_tray: self.keep_running_in_tray,
            watched_folder: self.watched_folder.clone(),
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
    /// It stores the values that the model can use (`RecognitionChoice::usable`).
    pub fn set_in_use(&mut self, choice: &RecognitionChoice) {
        let choice = choice.usable();
        self.asr_model = choice.asr_model.clone();
        self.live_transcript = choice.live_transcript;
        self.speaker_source = choice.speaker_source;
        self.speaker_processing_mode = choice.speaker_processing_mode;
        self.recognition_language = choice.recognition_language.clone();
        self.default_speaker_count = choice.default_speaker_count;
        self.recognition.differs =
            self.recognition.chosen_not_known || !self.chosen_recognition.same_use(&choice);
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

    /// Add `app` to the app list (R2). An app that is on the list already
    /// stays there once. An app with a blank ID or name is refused.
    pub fn add_app(&mut self, app: ListedApp) -> Result<(), String> {
        if app.id.trim().is_empty() || app.name.trim().is_empty() {
            return Err("An app needs an ID and a name.".into());
        }
        if !self.apps.iter().any(|a| a.id == app.id) {
            self.apps.push(app);
        }
        Ok(())
    }

    /// Remove the app with `id` from the app list (R4). The app set of an
    /// open note does not change (R11).
    pub fn remove_app(&mut self, id: &str) {
        self.apps.retain(|a| a.id != id);
    }

    /// The storage folder, or `None` before the launch sets one.
    pub fn storage_folder(&self) -> Option<PathBuf> {
        folder_path(&self.storage_folder)
    }

    /// The address that requests to `provider` use: the address that the
    /// user entered, or the usual address of the provider. Empty for Custom
    /// with no address.
    pub fn provider_address(&self, provider: Provider) -> String {
        self.providers
            .get(provider)
            .address
            .as_deref()
            .map(str::trim)
            .filter(|address| !address.is_empty())
            .or_else(|| provider.default_address())
            .unwrap_or_default()
            .to_string()
    }

    /// Whether a provider is active and has an address and a model. The
    /// request gate ([`provider_gate`]) decides the rest.
    pub fn provider_configured(&self) -> bool {
        self.active_provider.is_some_and(|provider| {
            !self.provider_address(provider).is_empty()
                && !self.providers.get(provider).model.trim().is_empty()
        })
    }

    /// Where the Keychain holds the key of `provider`. `None` for a
    /// provider that takes no key (R8) or that has no address.
    pub fn key_location(&self, provider: Provider) -> Option<KeyLocation> {
        let address = self.provider_address(provider);
        (provider.takes_key() && !address.is_empty()).then(|| KeyLocation {
            provider,
            address,
            legacy: self.legacy_key_provider == Some(provider),
        })
    }

    /// Make `provider` the active provider, or choose none (R7). The entry
    /// of each provider does not change, so a switch back restores it
    /// (R10).
    pub fn choose_provider(&mut self, provider: Option<Provider>) {
        self.active_provider = provider;
    }

    /// Save the address of `provider` (R8, R9). An empty value gives a
    /// local provider its usual address again, and gives Custom no address.
    /// Any other value must be a URL with a host, because the key is kept
    /// for that host (R10). A cloud provider has a fixed address.
    pub fn set_provider_address(
        &mut self,
        provider: Provider,
        address: &str,
    ) -> Result<(), String> {
        if !provider.address_editable() {
            return Err("The address of this provider cannot change.".into());
        }
        let address = address.trim();
        if !address.is_empty() {
            secrets::endpoint_key(address).map_err(|e| e.to_string())?;
        }
        self.providers.get_mut(provider).address =
            (!address.is_empty()).then(|| address.to_string());
        Ok(())
    }

    /// Save the model of `provider`.
    pub fn set_provider_model(&mut self, provider: Provider, model: &str) {
        self.providers.get_mut(provider).model = model.trim().to_string();
    }

    /// Save the context limit that the user entered for `provider`, or
    /// `None` to use the reported limit (R15).
    pub fn set_provider_context_limit(
        &mut self,
        provider: Provider,
        limit: Option<u32>,
    ) -> Result<(), String> {
        if limit == Some(0) {
            return Err("a context limit must be above zero".into());
        }
        self.providers.get_mut(provider).context_limit_tokens = limit;
        Ok(())
    }

    /// Save the Thinking level that the user picked for `provider` (R5).
    /// A pick equal to the default is saved too (KTD7).
    pub fn set_provider_thinking(&mut self, provider: Provider, level: ThinkingLevel) {
        self.providers.get_mut(provider).thinking = Some(level);
    }

    /// Save how many requests go to `provider` at the same time (R12). A
    /// number out of [`REQUESTS_AT_ONCE`] is refused, and the saved number
    /// stays.
    pub fn set_provider_requests_at_once(
        &mut self,
        provider: Provider,
        count: u32,
    ) -> Result<(), String> {
        if !REQUESTS_AT_ONCE.contains(&count) {
            return Err(format!(
                "requests at the same time must be from {} to {}",
                REQUESTS_AT_ONCE.start(),
                REQUESTS_AT_ONCE.end()
            ));
        }
        self.providers.get_mut(provider).requests_at_once = Some(count);
        Ok(())
    }

    /// Save the address that the connection check of `provider` resolved
    /// (KTD9, R14). Only Custom gets a changed address, and only while its
    /// address is still `started`, the address that the check used. A
    /// check that ends after the user entered a newer address saves
    /// nothing. Returns whether the address changed.
    pub fn save_checked_address(
        &mut self,
        provider: Provider,
        started: &str,
        resolved: &str,
    ) -> bool {
        let resolved = resolved.trim();
        if provider != Provider::Custom
            || resolved.is_empty()
            || resolved == started.trim()
            || self.provider_address(provider) != started.trim()
        {
            return false;
        }
        self.set_provider_address(provider, resolved).is_ok()
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

    /// The prompt that a summary of a note of the type `note_type` uses now
    /// (R3, KTD2). `None` is General. A type with no choice, a type whose
    /// name is not valid (R18), and a choice that names a prompt that this
    /// computer never read use the default prompt
    /// ([`Settings::active_summary_prompt_snapshot`]). While the file of the
    /// chosen prompt is missing or has a conflict copy, the last copy that
    /// this computer read applies (R7).
    pub fn summary_prompt_for_type(&self, note_type: Option<&str>) -> summarize::Prompt {
        use session_save::naming::check_type_name;
        use shared_settings::note_types::prompt_choice;
        // The type of a note is in the effective type list, so the type
        // name check gives its validity.
        let valid = note_type.is_none_or(|name| check_type_name(name).as_deref() == Ok(name));
        prompt_choice(&self.note_type_prompts, note_type, valid)
            .and_then(|id| self.type_prompts.get(id))
            .cloned()
            .unwrap_or_else(|| self.active_summary_prompt_snapshot())
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
                over_limit: false,
                too_large: false,
            });
        let custom = self.summary_prompts.iter().map(|p| {
            let too_large = self.too_large_prompts.contains(&p.id);
            SummaryPromptEntry {
                id: p.id.clone(),
                name: p.name.clone(),
                text: p.text.clone(),
                title_text: p.title_text.clone(),
                built_in: false,
                over_limit: too_large
                    || p.text.chars().count() > summarize::prompts::PROMPT_TEXT_LIMIT
                    || p.title_text.chars().count() > summarize::prompts::TITLE_TEXT_LIMIT,
                too_large,
            }
        });
        SummaryPromptList {
            prompts: built_ins.chain(custom).collect(),
            active: self.active_summary_prompt_snapshot().id,
            text_limit: summarize::prompts::PROMPT_TEXT_LIMIT as u32,
            title_text_limit: summarize::prompts::TITLE_TEXT_LIMIT as u32,
        }
    }

    /// The prompt `id` with its text. A prompt whose file is over the size
    /// limit has no text here, so a summary never uses it (R21).
    fn find_summary_prompt(&self, id: &str) -> Option<summarize::Prompt> {
        summarize::prompts::built_in(id).or_else(|| {
            self.summary_prompts
                .iter()
                .find(|p| p.id == id && !self.too_large_prompts.contains(id))
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
    if before.show_recording_widget != after.show_recording_widget {
        patch.push(SharedValue::ShowRecordingWidget(
            after.show_recording_widget,
        ));
    }
    if before.new_note_records != after.new_note_records {
        patch.push(SharedValue::NewNoteRecords(after.new_note_records));
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
    // Version 7 replaces the one endpoint with the active provider and an
    // entry for each provider (KTD6, R23). A store that already has the
    // entries, for example from a development build, keeps them.
    if stored_version < PROVIDER_SETTINGS_VERSION {
        if stored.get("providers").is_none() {
            migrate_endpoint(settings, stored);
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

/// The version 7 migration of the old endpoint (R23):
///
/// - A saved address becomes Custom with the same address, model and
///   context limit.
/// - No saved address and a model becomes OpenAI with that model and
///   context limit. The old build sent such a model to the OpenAI address.
/// - A cleared (empty) address, or no address and no model, becomes no
///   provider.
///
/// The provider that this migration chooses gets the old host-only key in
/// the Keychain on its first read (KTD8).
fn migrate_endpoint(settings: &mut LocalSettings, stored: &Value) {
    let model = stored
        .get(LEGACY_ENDPOINT_MODEL)
        .and_then(Value::as_str)
        .map_or("", str::trim);
    let limit = stored
        .get(LEGACY_CONTEXT_LIMIT)
        .and_then(Value::as_u64)
        .and_then(|limit| u32::try_from(limit).ok())
        .filter(|&limit| limit > 0);
    let address = stored
        .get(LEGACY_ENDPOINT_BASE_URL)
        .and_then(Value::as_str)
        .map(str::trim);
    let (provider, address) = match address {
        Some("") => return,
        Some(address) => (Provider::Custom, Some(address.to_string())),
        None if model.is_empty() => return,
        None => (Provider::OpenAi, None),
    };
    settings.active_provider = Some(provider);
    settings.legacy_key_provider = Some(provider);
    *settings.providers.get_mut(provider) = ProviderEntry {
        address,
        model: model.to_string(),
        context_limit_tokens: limit,
        ..Default::default()
    };
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
            type_prompts: Vec::new(),
            // A local store from before shared settings has no Background.
            background: String::new(),
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
        let _guard = UPDATES.lock.lock().unwrap_or_else(|e| e.into_inner());
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

/// The Background text of the shared view: the value of
/// `Settings::background`. It is the cached text while the Background file
/// or the storage folder is not available (KTD3).
pub fn get_background(app: &AppHandle) -> String {
    shared_snapshot(app).view.background
}

/// The values of the local settings file after the launch read, as the app
/// uses them. [`write_settings`] changes them only after the file write
/// succeeded.
pub struct LocalStore {
    path: PathBuf,
    values: Mutex<Map<String, Value>>,
}

/// The path of the local settings file in the app data folder.
fn local_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(local_store::FILE_NAME))
        .map_err(|e| e.to_string())
}

/// The local settings in `values`, and whether a migration changed them.
fn local_from(values: &Map<String, Value>) -> (LocalSettings, bool) {
    match values.get(SETTINGS_KEY) {
        Some(stored) => settings_from_value(stored),
        None => (LocalSettings::default(), true),
    }
}

/// What the launch does with the result of the read of the local settings
/// file. A file that cannot be read needs the user's choice: the defaults
/// never replace it without that choice.
#[derive(Debug, PartialEq)]
pub enum LaunchRead {
    /// The values of the file. A missing file has no values.
    Values(Map<String, Value>),
    /// The file is there and cannot be read.
    Ask(local_store::Unreadable),
}

pub fn launch_read(
    read: Result<Option<Map<String, Value>>, local_store::Unreadable>,
) -> LaunchRead {
    match read {
        Ok(values) => LaunchRead::Values(values.unwrap_or_default()),
        Err(e) => LaunchRead::Ask(e),
    }
}

/// Read the local settings file at launch, before any other reader that
/// writes. A first run writes the defaults, and an old file migrates. The
/// migration of a file from before version 6 puts the old shared values into
/// the cache first, when no cache exists. When that write fails, the file
/// stays as it is, so the next launch tries again. A file that cannot be
/// read shows the startup alert: Quit changes no file, and Use Default
/// Settings keeps the file under another name and starts as a first run.
pub fn load_at_launch(app: &AppHandle) -> Result<LocalSettings, crate::startup::Failure> {
    let failure = |e: String| crate::startup::Failure {
        description: crate::startup::SETTINGS_FAILURE,
        source: e.into(),
    };
    let path = local_store_path(app).map_err(failure)?;
    local_store::remove_temp_files(&path);
    let read = local_store::read(&path, local_store::LAUNCH_TRIES, local_store::LAUNCH_DELAY);
    let values = match launch_read(read) {
        LaunchRead::Values(values) => values,
        LaunchRead::Ask(error) => {
            log::error!(
                "the local settings file {} can't be read: {error}",
                path.display()
            );
            crate::startup::ask_about_settings(&path, &error.to_string());
            let kept = local_store::set_aside(&path)
                .map_err(|e| failure(format!("{}: {e}", path.display())))?;
            log::info!("the unreadable settings file is kept as {}", kept.display());
            Map::new()
        }
    };
    let (settings, updated) = local_from(&values);
    let values = if updated {
        let legacy = values.get(SETTINGS_KEY).and_then(legacy_shared);
        let seeded = match legacy {
            Some(legacy) => shared_cache_dir(app).and_then(|dir| {
                crate::managers::shared::seed_cache(&dir, legacy).map_err(|e| e.to_string())
            }),
            None => Ok(false),
        };
        match seeded {
            Err(e) => {
                log::error!("could not keep the old shared settings in the cache: {e}");
                values
            }
            Ok(_) => {
                let mut migrated = values.clone();
                migrated.insert(
                    SETTINGS_KEY.into(),
                    serde_json::to_value(&settings).expect("settings serialize"),
                );
                match local_store::write(&path, &migrated) {
                    Ok(()) => migrated,
                    Err(e) => {
                        log::error!("failed to persist migrated settings: {e}");
                        values
                    }
                }
            }
        }
    } else {
        values
    };
    app.manage(LocalStore {
        path,
        values: Mutex::new(values),
    });
    Ok(settings)
}

/// The local settings. Before the launch read, this reads the file and
/// writes nothing, as [`read_settings`] does: the update install at launch
/// runs before the launch read.
pub fn get_local_settings(app: &AppHandle) -> LocalSettings {
    match app.try_state::<LocalStore>() {
        Some(store) => local_from(&store.values.lock().unwrap_or_else(|e| e.into_inner())).0,
        None => read_settings(app),
    }
}

/// The local settings in the file at `path`, with no write: a migrated
/// value is not saved, and a file that cannot be read gives the defaults.
pub fn read_settings_at(path: &Path) -> LocalSettings {
    local_store::read(path, 1, std::time::Duration::ZERO)
        .ok()
        .flatten()
        .map(|values| local_from(&values).0)
        .unwrap_or_default()
}

/// Read the local settings and write nothing: a migrated value is not
/// saved. Launch reads the Dock policy with this before the single-instance
/// lock (U19), so a second copy that exits changes no file.
pub fn read_settings(app: &AppHandle) -> LocalSettings {
    local_store_path(app)
        .map(|path| read_settings_at(&path))
        .unwrap_or_default()
}

/// Apply `change` to the settings and save each store that the change
/// touches (R15, KTD7). The local fields go to the local store. The shared
/// fields that changed go to the shared settings file as a patch of only
/// those values. A store with no changed field is not written. Nothing is
/// written when `change` refuses. The local part of a change that also sets
/// a shared field is written first; when the shared save refuses (for
/// example while the shared settings are read-only), the local part is put
/// back and the error is returned. The prompts have their own commands: a
/// change of `summary_prompts` here is not saved.
pub fn update_settings(
    app: &AppHandle,
    change: impl FnOnce(&mut Settings) -> Result<(), String>,
) -> Result<Settings, String> {
    let (local, shared) = update_with(
        &UPDATES,
        &Stores {
            read_local: &|| get_local_settings(app),
            snapshot: &|| shared_snapshot(app),
            save_shared: &|patch| {
                app.try_state::<Arc<SharedSettingsManager>>()
                    .ok_or("The shared settings are not ready yet.")?
                    .save(patch)
            },
            write_local: &|local| write_settings(app, local),
        },
        change,
    )?;
    Ok(with_downloads(app, Settings::from_parts(local, &shared)))
}

/// The reads and writes of [`update_with`].
struct Stores<'a> {
    read_local: &'a dyn Fn() -> LocalSettings,
    snapshot: &'a dyn Fn() -> SharedSnapshot,
    save_shared: &'a dyn Fn(&[SharedValue]) -> Result<SharedSnapshot, String>,
    write_local: &'a dyn Fn(&LocalSettings) -> Result<(), String>,
}

/// The order of [`update_with`]. `lock` serializes each read-modify-write
/// of the local settings, so two commands changing different fields never
/// lose one another's change. A change of a shared setting does not hold
/// `lock` while the shared settings save, because that save can wait for
/// the storage folder. It takes a turn instead, so shared changes take
/// effect in the order that they were made.
struct Updates {
    lock: Mutex<()>,
    turns: Mutex<Turns>,
    turn_ended: Condvar,
}

/// The turns of the shared changes: `given` turns were given, and the turns
/// before `current` have ended. `pending` holds the patch of each turn that
/// has not ended: a change made while an earlier shared save waits starts
/// from the values that the earlier change chose, not from the values in
/// the file.
struct Turns {
    given: u64,
    current: u64,
    pending: Vec<(u64, Vec<SharedValue>)>,
    #[cfg(test)]
    waiting: u64,
}

static UPDATES: Updates = Updates::new();

impl Updates {
    const fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            turns: Mutex::new(Turns {
                given: 0,
                current: 0,
                pending: Vec::new(),
                #[cfg(test)]
                waiting: 0,
            }),
            turn_ended: Condvar::new(),
        }
    }

    /// Give the next turn to a change with `patch`. Call this with `lock`
    /// held.
    fn give_turn(&self, patch: &[SharedValue]) -> u64 {
        let mut turns = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        let turn = turns.given;
        turns.given += 1;
        turns.pending.push((turn, patch.to_vec()));
        turn
    }

    /// Put the patches of the turns that have not ended onto `settings`, in
    /// the order of the turns. Call this with `lock` held.
    fn with_pending(&self, settings: &mut Settings) {
        let turns = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        for (_, patch) in &turns.pending {
            for value in patch {
                apply_shared(settings, value);
            }
        }
    }

    /// Wait until `turn` is the current turn. The turn ends when the result
    /// is dropped, also after an error.
    fn wait_for(&self, turn: u64) -> TurnGuard<'_> {
        let mut turns = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(test)]
        {
            turns.waiting += 1;
            self.turn_ended.notify_all();
        }
        while turns.current != turn {
            turns = self
                .turn_ended
                .wait(turns)
                .unwrap_or_else(|e| e.into_inner());
        }
        #[cfg(test)]
        {
            turns.waiting -= 1;
        }
        TurnGuard { updates: self }
    }
}

struct TurnGuard<'a> {
    updates: &'a Updates,
}

impl Drop for TurnGuard<'_> {
    fn drop(&mut self) {
        let mut turns = self.updates.turns.lock().unwrap_or_else(|e| e.into_inner());
        let ended = turns.current;
        turns.pending.retain(|(turn, _)| *turn != ended);
        turns.current += 1;
        self.updates.turn_ended.notify_all();
    }
}

/// Set the value of one shared setting in `settings`, as [`shared_patch`]
/// reads it. A value that [`shared_patch`] never gives changes nothing.
fn apply_shared(settings: &mut Settings, value: &SharedValue) {
    let chosen = &mut settings.chosen_recognition;
    match value {
        SharedValue::ActiveSummaryPrompt(id) => settings.active_summary_prompt = id.clone(),
        SharedValue::DefaultSpeakerCount(count) => {
            chosen.default_speaker_count = count.map(Into::into)
        }
        SharedValue::AsrModel(model) => chosen.asr_model = model.clone(),
        SharedValue::LiveTranscript(on) => chosen.live_transcript = *on,
        SharedValue::SpeakerSource(source) => chosen.speaker_source = *source,
        SharedValue::SpeakerProcessingMode(mode) => chosen.speaker_processing_mode = *mode,
        SharedValue::RecognitionLanguage(language) => {
            chosen.recognition_language = language.clone()
        }
        SharedValue::CheckUpdatesAutomatically(on) => settings.check_updates_automatically = *on,
        SharedValue::AudioRemoval(removal) => settings.audio_removal = (*removal).into(),
        SharedValue::ShowRecordingWidget(on) => settings.show_recording_widget = *on,
        SharedValue::NewNoteRecords(on) => settings.new_note_records = *on,
        SharedValue::NoteTypes(_)
        | SharedValue::DefaultNoteType(_)
        | SharedValue::NoteTypePrompts(_) => {}
    }
}

/// `current` with each top-level field that a change set from its value in
/// `before` to its value in `after` put back to its value in `before`. A
/// field that no longer has the value in `after` keeps its value in
/// `current`: another change set it.
fn without_local_changes(
    current: &LocalSettings,
    before: &LocalSettings,
    after: &LocalSettings,
) -> Result<LocalSettings, String> {
    let as_map = |local: &LocalSettings| match serde_json::to_value(local) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err("the local settings are not an object".to_string()),
        Err(e) => Err(e.to_string()),
    };
    let (before, after) = (as_map(before)?, as_map(after)?);
    let mut reverted = as_map(current)?;
    for (key, value) in before {
        if after.get(&key) != Some(&value) && reverted.get(&key) == after.get(&key) {
            reverted.insert(key, value);
        }
    }
    serde_json::from_value(Value::Object(reverted)).map_err(|e| e.to_string())
}

/// The steps of [`update_settings`], with the stores as arguments. Returns
/// the local settings and the shared view after the change.
fn update_with(
    updates: &Updates,
    stores: &Stores<'_>,
    change: impl FnOnce(&mut Settings) -> Result<(), String>,
) -> Result<(LocalSettings, SharedSnapshot), String> {
    let guard = updates.lock.lock().unwrap_or_else(|e| e.into_inner());
    let local = (stores.read_local)();
    let shared = (stores.snapshot)();
    let mut before = Settings::from_parts(local.clone(), &shared);
    updates.with_pending(&mut before);
    let mut after = before.clone();
    change(&mut after)?;
    let patch = shared_patch(&before, &after);
    let changed_local = after.local();
    if changed_local != local {
        (stores.write_local)(&changed_local)?;
    }
    if patch.is_empty() {
        return Ok((changed_local, shared));
    }
    // The local part is written, in the order of the lock. The shared save
    // can wait for the storage folder, so it runs without the lock, in the
    // order of the turns.
    let turn = updates.give_turn(&patch);
    drop(guard);
    let _turn = updates.wait_for(turn);
    let saved = (stores.save_shared)(&patch);
    let _guard = updates.lock.lock().unwrap_or_else(|e| e.into_inner());
    let current = (stores.read_local)();
    match saved {
        Ok(shared) => Ok((current, shared)),
        Err(e) => {
            // Nothing of a refused change is kept: put back each local
            // field that this change changed and that still has this
            // change's value. A change that another command made during
            // the shared save stays.
            let reverted = without_local_changes(&current, &local, &changed_local)?;
            if reverted != current
                && let Err(write) = (stores.write_local)(&reverted)
            {
                log::error!("could not put back the local settings of a refused change: {write}");
            }
            Err(e)
        }
    }
}

/// Write the local settings file now, so a setting changed just before
/// quitting survives the quit. The values in memory change only when the
/// write succeeded.
fn write_settings(app: &AppHandle, settings: &LocalSettings) -> Result<(), String> {
    let store = app
        .try_state::<LocalStore>()
        .ok_or("The settings are not ready yet.")?;
    // The callers hold `UPDATES.lock`, which orders the writes. The values
    // stay readable during the file write.
    let mut changed = store
        .values
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    changed.insert(
        SETTINGS_KEY.into(),
        serde_json::to_value(settings).map_err(|e| e.to_string())?,
    );
    local_store::write(&store.path, &changed).map_err(|e| e.to_string())?;
    *store.values.lock().unwrap_or_else(|e| e.into_inner()) = changed;
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
                &serde_json::json!({ "settings_schema_version": 4, "interface_zoom_percent": value, "shortcut": "kept" }),
            );
            assert_eq!(settings.interface_zoom_percent, 100);
            assert_eq!(settings.shortcut.as_deref(), Some("kept"));
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
            "shortcut": "kept",
        });
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(
            settings.settings_schema_version,
            CURRENT_SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(settings.asr_model, "whisper-large-v3-turbo-q5km");
        assert_eq!(settings.shortcut.as_deref(), Some("kept"));
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
                "shortcut": "kept",
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
            assert_eq!(local.shortcut.as_deref(), Some("kept"));
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
            "shortcut": "kept",
        });
        let (local, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(local.shortcut.as_deref(), Some("kept"));
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
        // A new install has no Pause chord (R16).
        assert_eq!(settings.pause_shortcut, None);
        // A new install has no recording widget chord (R13).
        assert_eq!(settings.widget_shortcut, None);
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
    fn a_store_without_the_recording_keys_reads_as_system_default() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "shortcut": "kept",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated, "the new keys need no migration (KTD17)");
        assert_eq!(settings.microphone, None);
        assert_eq!(settings.shortcut.as_deref(), Some("kept"));
        assert_eq!(Settings::default().microphone, None);
    }

    // Covers R9: a store from before the setting plays the alert sound, as
    // before.
    #[test]
    fn a_store_without_the_alert_sound_key_reads_as_on() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "shortcut": "kept",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated, "the new key needs no migration");
        assert!(settings.alert_sound);
        assert!(Settings::default().alert_sound);
    }

    #[test]
    fn the_alert_sound_setting_stays_local() {
        let settings = Settings {
            alert_sound: false,
            ..Settings::default()
        };
        let stored = serde_json::to_value(settings.local()).unwrap();
        assert_eq!(stored["alert_sound"], json!(false));
        let (read, _) = settings_from_value(&stored);
        assert!(!read.alert_sound);
    }

    /// The settings with the shared settings file `raw`, as a view of a
    /// storage folder that is available.
    fn with_shared_file(raw: serde_json::Map<String, Value>) -> Settings {
        use shared_settings::view::{self, Availability};
        let cache = shared_settings::cache::Cache {
            source: PathBuf::from("/Volumes/Notes"),
            settings: raw,
            prompts: Vec::new(),
            active_prompt: None,
            type_prompts: Vec::new(),
            background: String::new(),
        };
        let shared = SharedSnapshot {
            view: view::cached(Some(&cache), Availability::Available),
            availability: crate::managers::shared::SharedAvailability::Available,
            damaged_file: None,
        };
        Settings::from_parts(LocalSettings::default(), &shared)
    }

    // Covers R15 (safe agent edits U7): Show recording widget and New Note
    // records are shared settings. A change goes only into the shared
    // file, and the local store has no value for them.
    #[test]
    fn the_widget_and_new_note_settings_are_shared() {
        let before = Settings::default();
        assert!(before.show_recording_widget);
        assert!(before.new_note_records);
        let mut after = before.clone();
        after.show_recording_widget = false;
        after.new_note_records = false;
        assert_eq!(
            shared_patch(&before, &after),
            [
                SharedValue::ShowRecordingWidget(false),
                SharedValue::NewNoteRecords(false),
            ]
        );
        assert_eq!(after.local(), before.local());
        let stored = serde_json::to_value(after.local()).unwrap();
        assert!(stored.get("show_recording_widget").is_none());
        assert!(stored.get("new_note_records").is_none());
    }

    // A shared file without the keys gives the defaults on every computer,
    // also when an old local store has a value (no installs to migrate).
    #[test]
    fn a_shared_file_without_the_widget_and_new_note_keys_gives_on() {
        let mut raw = shared_settings::file::default_settings();
        raw.remove("show_recording_widget");
        raw.remove("new_note_records");
        let settings = with_shared_file(raw);
        assert!(settings.show_recording_widget);
        assert!(settings.new_note_records);
        let (local, _) = settings_from_value(&json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "show_recording_widget": false,
            "new_note_records": false,
        }));
        assert_eq!(local, LocalSettings::default());
    }

    // An edit of the shared file changes the setting at the next read.
    #[test]
    fn the_widget_and_new_note_settings_come_from_the_shared_file() {
        let mut raw = shared_settings::file::default_settings();
        raw.insert("show_recording_widget".into(), json!(false));
        raw.insert("new_note_records".into(), json!(false));
        let settings = with_shared_file(raw);
        assert!(!settings.show_recording_widget);
        assert!(!settings.new_note_records);
    }

    #[test]
    fn a_store_without_hide_from_dock_reads_as_off_and_needs_no_migration() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "shortcut": "kept",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated, "the new key needs no migration");
        assert!(!settings.hide_from_dock);
        assert_eq!(settings.shortcut.as_deref(), Some("kept"));
        assert!(!Settings::default().hide_from_dock);
    }

    #[test]
    fn an_old_store_without_the_update_setting_reads_as_on() {
        let stored = json!({ "settings_schema_version": OLD, "shortcut": "kept" });
        let (settings, _) = migrated(&stored);
        assert!(settings.check_updates_automatically);
        assert_eq!(settings.shortcut.as_deref(), Some("kept"));
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

    // R4: the watched folder is a setting of this computer, off by default.
    #[test]
    fn the_watched_folder_is_a_local_setting_that_is_off_by_default() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "shortcut": "kept",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated, "the new key needs no migration");
        assert_eq!(settings.watched_folder, None);
        assert_eq!(Settings::default().watched_folder, None);
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "watched_folder": "/Users/me/Recordings",
        });
        let (settings, _) = settings_from_value(&stored);
        assert_eq!(
            settings.watched_folder.as_deref(),
            Some("/Users/me/Recordings")
        );
        let before = Settings::default();
        let mut after = before.clone();
        after.watched_folder = Some("/Users/me/Recordings".into());
        assert!(shared_patch(&before, &after).is_empty());
        assert_eq!(after.local().watched_folder, after.watched_folder);
    }

    // An older store has no value: the close quits, and nothing migrates.
    #[test]
    fn keep_running_in_tray_is_a_local_setting_that_is_off_by_default() {
        let stored = json!({ "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated, "the new key needs no migration");
        assert!(!settings.keep_running_in_tray);
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "keep_running_in_tray": true,
        });
        let (settings, _) = settings_from_value(&stored);
        assert!(settings.keep_running_in_tray);
        let before = Settings::default();
        let mut after = before.clone();
        after.keep_running_in_tray = true;
        assert!(shared_patch(&before, &after).is_empty());
        assert!(after.local().keep_running_in_tray);
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
                "shortcut": "kept",
                "alert_tone_heard": heard,
            });
            let (settings, updated) = settings_from_value(&stored);
            assert!(!updated, "the removed key needs no migration (KTD16)");
            assert_eq!(settings.shortcut.as_deref(), Some("kept"));
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
            "hide_from_dock": true,
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(settings.microphone, None);
        assert!(settings.hide_from_dock);
    }

    fn zoom() -> ListedApp {
        ListedApp {
            id: "us.zoom.xos".into(),
            name: "zoom.us".into(),
            path: "/Applications/zoom.us.app".into(),
        }
    }

    #[test]
    fn a_store_without_the_app_list_reads_as_an_empty_list() {
        let stored = json!({ "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION });
        let (settings, _) = settings_from_value(&stored);
        assert!(settings.apps.is_empty());
        assert!(Settings::default().apps.is_empty());
    }

    #[test]
    fn the_app_list_round_trips_and_an_app_is_on_it_once() {
        let mut settings = Settings::default();
        settings.add_app(zoom()).unwrap();
        settings.add_app(zoom()).unwrap();
        assert_eq!(settings.apps, vec![zoom()]);
        let written = serde_json::to_value(settings.local()).unwrap();
        let (read, updated) = settings_from_value(&written);
        assert!(!updated);
        assert_eq!(read.apps, vec![zoom()]);
        settings.remove_app("us.zoom.xos");
        assert!(settings.apps.is_empty());
    }

    #[test]
    fn an_app_needs_an_id_and_a_name() {
        let mut settings = Settings::default();
        for bad in [
            ListedApp {
                id: " ".into(),
                ..zoom()
            },
            ListedApp {
                name: String::new(),
                ..zoom()
            },
        ] {
            assert!(settings.add_app(bad).is_err());
        }
        assert!(settings.apps.is_empty());
    }

    #[test]
    fn a_malformed_app_entry_is_dropped_and_the_rest_is_kept() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "apps": [
                "us.zoom.xos",
                { "id": "", "name": "Blank", "path": "" },
                { "id": "us.zoom.xos", "name": "zoom.us", "path": "/Applications/zoom.us.app" },
                { "id": "us.zoom.xos", "name": "Second copy", "path": "" },
            ],
            "hide_from_dock": true,
        });
        let (settings, _) = settings_from_value(&stored);
        assert_eq!(settings.apps, vec![zoom()]);
        assert!(settings.hide_from_dock);

        let (settings, _) = settings_from_value(&json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "apps": "us.zoom.xos",
            "alert_sound": false,
        }));
        assert!(settings.apps.is_empty());
        assert!(!settings.alert_sound);
    }

    #[test]
    fn present_fields_survive_a_version_migration() {
        let (settings, updated) = settings_from_value(
            &json!({ "endpoint_model": "gpt-5", "destination_roots": ["/a"], "current_destination": "/a" }),
        );
        assert!(updated);
        assert_eq!(settings.providers.openai.model, "gpt-5");
        assert_eq!(settings.storage_folder, "/a");
    }

    /// R8: the current folder becomes the one storage folder, and the other
    /// folders are forgotten.
    #[test]
    fn a_store_with_three_earlier_folders_keeps_only_the_current_one() {
        for version in [OLD, u64::from(CURRENT_SETTINGS_SCHEMA_VERSION)] {
            let stored = json!({
                "settings_schema_version": version,
                "shortcut": "kept",
                "destination_roots": ["/vault/a", "/vault/b", "/vault/c", "/vault/d"],
                "current_destination": "/vault/c",
            });
            let (settings, updated) = settings_from_value(&stored);
            assert!(updated, "version {version}");
            assert_eq!(settings.storage_folder, "/vault/c");
            assert_eq!(settings.shortcut.as_deref(), Some("kept"));
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
            "shortcut": "x",
            "from_the_future": true,
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(settings.shortcut.as_deref(), Some("x"));
        let written = serde_json::to_value(&settings).unwrap();
        assert!(written.get("from_the_future").is_none());
    }

    #[test]
    fn one_invalid_field_keeps_the_rest() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "interface_zoom_percent": "lots",
            "shortcut": "kept",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(settings.shortcut.as_deref(), Some("kept"));
        assert_eq!(settings.interface_zoom_percent, 100);
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
        assert_eq!(settings.providers.openai.model, "gpt-5");
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
            "shortcut": "kept",
            "summary_prompts": [
                { "id": "a", "name": "Team sync", "text": "Recap." },
                { "id": "b", "name": 7 },
                { "id": "c", "name": "One on one", "text": "Notes." },
            ],
        });
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(settings.shortcut.as_deref(), Some("kept"));
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
                background: String::new(),
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
    fn a_change_of_a_local_value_changes_only_local_fields() {
        // A local value is not a shared value, so the save does not write
        // the shared settings file (R15).
        let before = Settings::default();
        let mut after = before.clone();
        after.hide_from_dock = !before.hide_from_dock;
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
            type_prompts: Vec::new(),
            background: String::new(),
        };
        let shared = SharedSnapshot {
            view: view::cached(Some(&cache), Availability::Available),
            availability: crate::managers::shared::SharedAvailability::Available,
            damaged_file: None,
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
        use models::recognition::{ProcessingMode, SpeakerSource};
        let before = Settings::default();
        let mut after = before.clone();
        after.chosen_recognition = RecognitionChoice {
            asr_model: "whisper-large-v3-turbo-q5km".into(),
            live_transcript: false,
            speaker_source: SpeakerSource::Sortformer,
            speaker_processing_mode: ProcessingMode::AfterStop,
            recognition_language: Some("de".into()),
            default_speaker_count: None,
        };
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
            type_prompts: Vec::new(),
            background: String::new(),
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
        after.providers.openai.model = "gpt-4o".into();
        assert!(shared_patch(&settings, &after).is_empty());
    }

    #[test]
    fn the_launch_asks_for_a_file_that_cannot_be_read_and_starts_a_missing_file_empty() {
        assert_eq!(launch_read(Ok(None)), LaunchRead::Values(Map::new()));
        let values = json!({ "settings": { "storage_folder": "/notes" } })
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(
            launch_read(Ok(Some(values.clone()))),
            LaunchRead::Values(values)
        );
        let damaged = local_store::Unreadable::DoesNotParse("EOF".into());
        assert_eq!(launch_read(Err(damaged.clone())), LaunchRead::Ask(damaged));
    }

    #[test]
    fn a_read_before_the_launch_read_writes_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(local_store::FILE_NAME);
        // A missing file gives the defaults and stays missing.
        assert_eq!(read_settings_at(&path), LocalSettings::default());
        assert!(!path.exists());
        // A damaged file gives the defaults and does not change.
        std::fs::write(&path, b"{\"settings\": {\"storage_fol").unwrap();
        assert_eq!(read_settings_at(&path), LocalSettings::default());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"settings\": {\"storage_fol"
        );
        // An old file reads migrated and does not change.
        let old = serde_json::to_vec(&json!({ "settings": {
            "settings_schema_version": 2,
            "storage_folder": "/notes",
        }}))
        .unwrap();
        std::fs::write(&path, &old).unwrap();
        assert_eq!(read_settings_at(&path).storage_folder, "/notes");
        assert_eq!(std::fs::read(&path).unwrap(), old);
    }

    type SaveStarts = std::sync::mpsc::Receiver<Vec<SharedValue>>;
    type SaveReleases = std::sync::mpsc::Sender<Result<(), String>>;

    /// Stores in memory for [`update_with`]. A shared save waits for a
    /// message on its own channel after it says that it started.
    struct MemoryStores {
        local: Mutex<LocalSettings>,
        saves: Mutex<Vec<Vec<SharedValue>>>,
        started: Mutex<std::sync::mpsc::Sender<Vec<SharedValue>>>,
        release: Mutex<std::sync::mpsc::Receiver<Result<(), String>>>,
    }

    impl MemoryStores {
        /// The stores, the start of each shared save, and the release of it.
        fn new() -> (Arc<Self>, SaveStarts, SaveReleases) {
            let (started, on_start) = std::sync::mpsc::channel();
            let (release, on_release) = std::sync::mpsc::channel();
            let stores = Arc::new(Self {
                local: Mutex::new(LocalSettings::default()),
                saves: Mutex::new(Vec::new()),
                started: Mutex::new(started),
                release: Mutex::new(on_release),
            });
            (stores, on_start, release)
        }

        fn update(
            &self,
            updates: &Updates,
            change: impl FnOnce(&mut Settings) -> Result<(), String>,
        ) -> Result<LocalSettings, String> {
            update_with(
                updates,
                &Stores {
                    read_local: &|| self.local.lock().unwrap().clone(),
                    snapshot: &SharedSnapshot::default,
                    save_shared: &|patch| {
                        self.started.lock().unwrap().send(patch.to_vec()).unwrap();
                        self.release.lock().unwrap().recv().unwrap()?;
                        self.saves.lock().unwrap().push(patch.to_vec());
                        Ok(SharedSnapshot::default())
                    },
                    write_local: &|local| {
                        *self.local.lock().unwrap() = local.clone();
                        Ok(())
                    },
                },
                change,
            )
            .map(|(local, _)| local)
        }
    }

    fn new_updates() -> &'static Updates {
        Box::leak(Box::new(Updates::new()))
    }

    /// Wait until `count` changes wait for their turn.
    /// Only a failure reaches the time limit.
    fn wait_for_waiting(updates: &Updates, count: u64) {
        let (turns, _) = updates
            .turn_ended
            .wait_timeout_while(
                updates.turns.lock().unwrap(),
                std::time::Duration::from_secs(30),
                |turns| turns.waiting != count,
            )
            .unwrap();
        assert_eq!(turns.waiting, count, "no change waited for its turn");
    }

    #[test]
    fn shared_changes_take_effect_in_the_order_that_they_were_made() {
        let updates = new_updates();
        let (stores, on_start, release) = MemoryStores::new();
        // Each change sets a shared field and a local field to one value.
        let change = |value: bool| {
            let stores = Arc::clone(&stores);
            std::thread::spawn(move || {
                stores.update(updates, move |s| {
                    s.new_note_records = value;
                    s.alert_sound = value;
                    Ok(())
                })
            })
        };
        let first = change(false);
        on_start.recv().unwrap();
        let second = change(true);
        // The second change waits for the turn of the first.
        wait_for_waiting(updates, 1);
        release.send(Ok(())).unwrap();
        assert!(first.join().unwrap().is_ok());
        on_start.recv().unwrap();
        release.send(Ok(())).unwrap();
        assert!(second.join().unwrap().unwrap().alert_sound);
        assert_eq!(
            *stores.saves.lock().unwrap(),
            [
                vec![SharedValue::NewNoteRecords(false)],
                vec![SharedValue::NewNoteRecords(true)],
            ]
        );
        // The local part of the later change is the one that stays.
        assert!(stores.local.lock().unwrap().alert_sound);
    }

    #[test]
    fn a_change_back_during_a_shared_save_is_saved_after_it() {
        let updates = new_updates();
        let (stores, on_start, release) = MemoryStores::new();
        let set = |value: bool| {
            let stores = Arc::clone(&stores);
            std::thread::spawn(move || {
                stores.update(updates, move |s| {
                    s.new_note_records = value;
                    Ok(())
                })
            })
        };
        // The user turns the setting off, and on again while the first
        // save waits for the storage folder.
        let off = set(false);
        assert_eq!(
            on_start.recv().unwrap(),
            [SharedValue::NewNoteRecords(false)]
        );
        let on = set(true);
        wait_for_waiting(updates, 1);
        release.send(Ok(())).unwrap();
        assert!(off.join().unwrap().is_ok());
        assert_eq!(
            on_start.recv().unwrap(),
            [SharedValue::NewNoteRecords(true)]
        );
        release.send(Ok(())).unwrap();
        assert!(on.join().unwrap().is_ok());
        assert_eq!(
            *stores.saves.lock().unwrap(),
            [
                vec![SharedValue::NewNoteRecords(false)],
                vec![SharedValue::NewNoteRecords(true)],
            ]
        );
    }

    #[test]
    fn a_refused_shared_save_keeps_the_local_changes_of_other_commands() {
        let updates = new_updates();
        let (stores, on_start, release) = MemoryStores::new();
        let failing = {
            let stores = Arc::clone(&stores);
            std::thread::spawn(move || {
                stores.update(updates, |s| {
                    s.new_note_records = false;
                    s.alert_sound = false;
                    s.providers.custom.model = "refused".into();
                    Ok(())
                })
            })
        };
        on_start.recv().unwrap();
        // During the shared save, other commands change a field of the
        // refused change and a field that it did not change.
        stores
            .update(updates, |s| {
                s.providers.custom.model = "later".into();
                s.live_transcript = false;
                Ok(())
            })
            .unwrap();
        release.send(Err("the folder is gone".into())).unwrap();
        assert!(failing.join().unwrap().is_err());
        let local = stores.local.lock().unwrap().clone();
        // The refused change's own field goes back; the others stay.
        assert!(local.alert_sound);
        assert_eq!(local.providers.custom.model, "later");
        assert!(!local.live_transcript);
    }

    #[test]
    fn each_shared_value_reads_back_as_the_same_patch() {
        let before = Settings::default();
        let mut after = before.clone();
        after.active_summary_prompt = "custom".into();
        after.chosen_recognition.default_speaker_count =
            Some(SpeakerCountSetting::Exact { count: 3 });
        after.chosen_recognition.asr_model = "whisper-large-v3-turbo-q5km".into();
        after.chosen_recognition.live_transcript = !before.chosen_recognition.live_transcript;
        after.chosen_recognition.recognition_language = Some("de".into());
        after.check_updates_automatically = !before.check_updates_automatically;
        after.show_recording_widget = !before.show_recording_widget;
        after.new_note_records = !before.new_note_records;
        let patch = shared_patch(&before, &after);
        assert_eq!(patch.len(), 8);
        let mut applied = before.clone();
        for value in &patch {
            apply_shared(&mut applied, value);
        }
        assert_eq!(shared_patch(&before, &applied), patch);
        assert!(shared_patch(&after, &applied).is_empty());
    }

    #[test]
    fn a_failed_shared_save_ends_its_turn_and_writes_no_local_change() {
        let updates = new_updates();
        let (stores, on_start, release) = MemoryStores::new();
        let failing = {
            let stores = Arc::clone(&stores);
            std::thread::spawn(move || {
                stores.update(updates, |s| {
                    s.new_note_records = false;
                    s.alert_sound = false;
                    Ok(())
                })
            })
        };
        on_start.recv().unwrap();
        release.send(Err("the folder is gone".into())).unwrap();
        assert_eq!(
            failing.join().unwrap(),
            Err("the folder is gone".to_string())
        );
        assert!(stores.local.lock().unwrap().alert_sound);
        // The next shared change gets its turn.
        let next = {
            let stores = Arc::clone(&stores);
            std::thread::spawn(move || {
                stores.update(updates, |s| {
                    s.new_note_records = false;
                    Ok(())
                })
            })
        };
        on_start.recv().unwrap();
        release.send(Ok(())).unwrap();
        assert!(next.join().unwrap().is_ok());
    }

    #[test]
    fn a_local_change_does_not_wait_for_a_shared_save() {
        let updates = new_updates();
        let (stores, on_start, release) = MemoryStores::new();
        let shared_change = {
            let stores = Arc::clone(&stores);
            std::thread::spawn(move || {
                stores.update(updates, |s| {
                    s.new_note_records = false;
                    Ok(())
                })
            })
        };
        // The shared save started and waits, for example on a slow folder.
        assert_eq!(
            on_start.recv().unwrap(),
            [SharedValue::NewNoteRecords(false)]
        );
        let (done, local_done) = std::sync::mpsc::channel();
        let local_change = {
            let stores = Arc::clone(&stores);
            std::thread::spawn(move || {
                let result = stores.update(updates, |s| {
                    s.alert_sound = false;
                    Ok(())
                });
                done.send(()).unwrap();
                result
            })
        };
        // Only a failure reaches the time limit: the local change does not
        // wait for the folder.
        local_done
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("a local change waited for the shared save");
        assert!(!local_change.join().unwrap().unwrap().alert_sound);
        release.send(Ok(())).unwrap();
        let after = shared_change.join().unwrap().unwrap();
        // The local change that ran during the shared save stays.
        assert!(!after.alert_sound);
        assert!(!stores.local.lock().unwrap().alert_sound);
    }

    #[test]
    fn a_model_from_a_newer_version_differs_when_the_other_chosen_values_are_in_use() {
        use shared_settings::view::{self, Availability};
        let in_use = LocalSettings::default();
        let mut raw = shared_settings::file::default_settings();
        raw.insert("asr_model".into(), json!("model-from-the-future"));
        let cache = shared_settings::cache::Cache {
            source: PathBuf::from("/vault"),
            settings: raw,
            prompts: Vec::new(),
            active_prompt: None,
            type_prompts: Vec::new(),
            background: String::new(),
        };
        let shared = SharedSnapshot {
            view: view::cached(Some(&cache), Availability::Available),
            ..SharedSnapshot::default()
        };
        let mut settings = Settings::from_parts(in_use, &shared);
        assert_eq!(settings.chosen_recognition, settings.in_use());
        // Settings shows "Chosen in a newer version of Longhand" only when
        // the choice differs (Models.tsx).
        assert!(settings.recognition.chosen_not_known);
        assert!(settings.recognition.differs);
        let in_use = settings.in_use();
        settings.set_in_use(&in_use);
        assert!(settings.recognition.differs);
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
                background: String::new(),
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
            background: String::new(),
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

    /// A store of the last version before the provider entries.
    fn version_6(fields: Value) -> Value {
        let mut stored = json!({ "settings_schema_version": PROVIDER_SETTINGS_VERSION - 1 });
        for (key, value) in fields.as_object().unwrap() {
            stored[key] = value.clone();
        }
        stored
    }

    /// The written form of `settings` reads back the same, with no update.
    fn converges(settings: &Settings) {
        let written = serde_json::to_value(settings.local()).unwrap();
        assert_eq!(settings_from_value(&written), (settings.local(), false));
    }

    #[test]
    fn a_new_store_has_no_provider() {
        // Covers R11.
        let (settings, _) = migrated(&json!({}));
        assert_eq!(settings.active_provider, None);
        assert_eq!(settings.providers, ProviderEntries::default());
        assert_eq!(settings.legacy_key_provider, None);
        assert!(!settings.provider_configured());
    }

    #[test]
    fn a_version_6_store_with_an_address_becomes_custom_with_the_same_address_and_model() {
        // Covers AE7.
        let stored = version_6(json!({
            "endpoint_base_url": "https://litellm.ewal.io/v1",
            "endpoint_model": "qwen3-30b",
            "context_limit_tokens": 64000,
        }));
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(settings.active_provider, Some(Provider::Custom));
        assert_eq!(
            settings.providers.custom,
            ProviderEntry {
                address: Some("https://litellm.ewal.io/v1".into()),
                model: "qwen3-30b".into(),
                context_limit_tokens: Some(64000),
                ..Default::default()
            }
        );
        assert_eq!(
            settings.provider_address(Provider::Custom),
            "https://litellm.ewal.io/v1"
        );
        // The key moves from the old host-only entry on the first read.
        assert_eq!(
            settings.key_location(Provider::Custom),
            Some(KeyLocation {
                provider: Provider::Custom,
                address: "https://litellm.ewal.io/v1".into(),
                legacy: true,
            })
        );
        assert!(settings.provider_configured());
        converges(&settings);
    }

    #[test]
    fn a_version_6_store_with_no_address_and_a_model_becomes_openai_with_that_model() {
        let stored =
            version_6(json!({ "endpoint_model": "gpt-4o", "context_limit_tokens": 32000 }));
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(settings.active_provider, Some(Provider::OpenAi));
        assert_eq!(
            settings.providers.openai,
            ProviderEntry {
                address: None,
                model: "gpt-4o".into(),
                context_limit_tokens: Some(32000),
                ..Default::default()
            }
        );
        assert_eq!(settings.legacy_key_provider, Some(Provider::OpenAi));
        assert_eq!(
            settings.provider_address(Provider::OpenAi),
            "https://api.openai.com/v1"
        );
        converges(&settings);
    }

    #[test]
    fn a_version_6_store_with_no_address_and_no_model_has_no_provider() {
        for stored in [
            version_6(json!({})),
            version_6(json!({ "endpoint_model": "  " })),
        ] {
            let (settings, updated) = migrated(&stored);
            assert!(updated);
            assert_eq!(settings.active_provider, None);
            assert_eq!(settings.providers, ProviderEntries::default());
            assert_eq!(settings.legacy_key_provider, None);
            converges(&settings);
        }
    }

    #[test]
    fn a_cleared_address_migrates_to_no_provider() {
        let stored = version_6(json!({ "endpoint_base_url": "", "endpoint_model": "gpt-4o" }));
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(settings.active_provider, None);
        assert_eq!(settings.providers, ProviderEntries::default());
        assert_eq!(settings.legacy_key_provider, None);
        converges(&settings);
    }

    #[test]
    fn the_old_endpoint_fields_are_dropped_on_the_next_write() {
        let stored = version_6(json!({
            "endpoint_base_url": "https://litellm.ewal.io/v1",
            "endpoint_model": "qwen3-30b",
        }));
        let (settings, _) = migrated(&stored);
        let written = serde_json::to_value(settings.local()).unwrap();
        for key in [
            LEGACY_ENDPOINT_BASE_URL,
            LEGACY_ENDPOINT_MODEL,
            LEGACY_CONTEXT_LIMIT,
        ] {
            assert!(written.get(key).is_none(), "{key}");
        }
        assert_eq!(written["active_provider"], json!("custom"));
        assert_eq!(
            written["providers"]["custom"],
            json!({ "address": "https://litellm.ewal.io/v1", "model": "qwen3-30b", "context_limit_tokens": null, "thinking": null, "requests_at_once": null })
        );
    }

    #[test]
    fn a_version_6_store_that_has_provider_entries_keeps_them() {
        // A development build can write the entries before version 7.
        let stored = version_6(json!({
            "endpoint_base_url": "https://litellm.ewal.io/v1",
            "endpoint_model": "qwen3-30b",
            "active_provider": "anthropic",
            "providers": { "anthropic": { "model": "claude-haiku-5-5" } },
        }));
        let (settings, updated) = migrated(&stored);
        assert!(updated);
        assert_eq!(settings.active_provider, Some(Provider::Anthropic));
        assert_eq!(settings.providers.anthropic.model, "claude-haiku-5-5");
        assert_eq!(settings.providers.custom, ProviderEntry::default());
        assert_eq!(settings.legacy_key_provider, None);
    }

    #[test]
    fn a_current_store_keeps_its_provider_and_is_not_updated() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "active_provider": "ollama",
            "providers": { "ollama": { "model": "llama3" } },
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(settings.active_provider, Some(Provider::Ollama));
        assert_eq!(settings.providers.ollama.model, "llama3");
    }

    #[test]
    fn a_switch_to_another_provider_and_back_restores_its_model_and_key() {
        // Covers AE1.
        let mut settings = Settings::default();
        settings.choose_provider(Some(Provider::Anthropic));
        settings.set_provider_model(Provider::Anthropic, "claude-haiku-5-5");
        let key = settings.key_location(Provider::Anthropic);

        settings.choose_provider(Some(Provider::Ollama));
        settings.set_provider_model(Provider::Ollama, "llama3");
        assert_eq!(settings.active_provider, Some(Provider::Ollama));

        settings.choose_provider(Some(Provider::Anthropic));
        assert_eq!(settings.providers.anthropic.model, "claude-haiku-5-5");
        assert_eq!(settings.providers.ollama.model, "llama3");
        assert_eq!(settings.key_location(Provider::Anthropic), key);
        assert_eq!(
            provider_gate(
                settings.active_provider,
                true,
                &settings.provider_address(Provider::Anthropic),
                KeyState::Saved
            ),
            EndpointGate::RequestWithKey
        );
    }

    #[test]
    fn each_provider_keeps_its_own_entered_context_limit() {
        let mut settings = Settings::default();
        settings
            .set_provider_context_limit(Provider::Ollama, Some(32_768))
            .unwrap();
        settings
            .set_provider_context_limit(Provider::Custom, Some(200_000))
            .unwrap();
        assert_eq!(settings.providers.ollama.context_limit_tokens, Some(32_768));
        assert_eq!(
            settings.providers.custom.context_limit_tokens,
            Some(200_000)
        );
        assert_eq!(settings.providers.openai.context_limit_tokens, None);
        settings
            .set_provider_context_limit(Provider::Ollama, None)
            .unwrap();
        assert_eq!(settings.providers.ollama.context_limit_tokens, None);
        assert_eq!(
            settings.providers.custom.context_limit_tokens,
            Some(200_000)
        );
        assert!(
            settings
                .set_provider_context_limit(Provider::Custom, Some(0))
                .is_err()
        );
        assert_eq!(
            settings.providers.custom.context_limit_tokens,
            Some(200_000)
        );
    }

    #[test]
    fn a_version_7_entry_with_no_new_fields_uses_the_defaults_of_its_kind() {
        // Covers AE4, R6, R12, KTD7. No new field means no write.
        let stored = json!({
            "settings_schema_version": 7,
            "active_provider": "lm_studio",
            "providers": {
                "lm_studio": { "model": "qwen3-30b" },
                "openrouter": { "model": "openai/gpt-5" },
            },
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        let lm_studio = &settings.providers.lm_studio;
        assert_eq!(lm_studio.thinking, None);
        assert_eq!(lm_studio.requests_at_once, None);
        assert_eq!(
            lm_studio.effective_thinking(Provider::LmStudio),
            (ThinkingLevel::Low, false)
        );
        assert_eq!(lm_studio.effective_requests_at_once(Provider::LmStudio), 1);
        let openrouter = &settings.providers.openrouter;
        assert_eq!(
            openrouter.effective_thinking(Provider::OpenRouter),
            (ThinkingLevel::ModelDefault, false)
        );
        assert_eq!(
            openrouter.effective_requests_at_once(Provider::OpenRouter),
            4
        );
    }

    #[test]
    fn a_thinking_level_that_the_user_picks_is_stored_and_chosen() {
        // KTD7: a pick equal to the default is stored too.
        let mut settings = Settings::default();
        settings.set_provider_thinking(Provider::LmStudio, ThinkingLevel::Low);
        assert_eq!(
            settings.providers.lm_studio.thinking,
            Some(ThinkingLevel::Low)
        );
        assert_eq!(
            settings
                .providers
                .lm_studio
                .effective_thinking(Provider::LmStudio),
            (ThinkingLevel::Low, true)
        );
        let written = serde_json::to_value(settings.local()).unwrap();
        assert_eq!(written["providers"]["lm_studio"]["thinking"], json!("low"));
    }

    #[test]
    fn requests_at_once_accepts_1_to_8_and_keeps_its_value_after_a_refusal() {
        let mut settings = Settings::default();
        settings
            .set_provider_requests_at_once(Provider::Custom, 1)
            .unwrap();
        assert_eq!(settings.providers.custom.requests_at_once, Some(1));
        settings
            .set_provider_requests_at_once(Provider::Custom, 8)
            .unwrap();
        assert_eq!(settings.providers.custom.requests_at_once, Some(8));
        for refused in [0, 9] {
            assert!(
                settings
                    .set_provider_requests_at_once(Provider::Custom, refused)
                    .is_err()
            );
            assert_eq!(settings.providers.custom.requests_at_once, Some(8));
        }
        assert_eq!(
            settings
                .providers
                .custom
                .effective_requests_at_once(Provider::Custom),
            8
        );
    }

    #[test]
    fn each_provider_keeps_its_own_thinking_and_requests_at_once() {
        let mut settings = Settings::default();
        settings.choose_provider(Some(Provider::Ollama));
        settings.set_provider_thinking(Provider::Ollama, ThinkingLevel::High);
        settings
            .set_provider_requests_at_once(Provider::Ollama, 2)
            .unwrap();
        settings.choose_provider(Some(Provider::OpenAi));
        settings.set_provider_thinking(Provider::OpenAi, ThinkingLevel::Off);
        settings.choose_provider(Some(Provider::Ollama));
        assert_eq!(
            settings.providers.ollama.thinking,
            Some(ThinkingLevel::High)
        );
        assert_eq!(settings.providers.ollama.requests_at_once, Some(2));
        assert_eq!(settings.providers.openai.thinking, Some(ThinkingLevel::Off));
        assert_eq!(settings.providers.openai.requests_at_once, None);
    }

    #[test]
    fn an_entry_with_an_unknown_field_beside_the_new_fields_still_loads() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "active_provider": "ollama",
            "providers": { "ollama": {
                "model": "qwen3",
                "thinking": "medium",
                "requests_at_once": 3,
                "a_field_from_another_build": true,
            } },
        });
        let (settings, _) = settings_from_value(&stored);
        assert_eq!(settings.active_provider, Some(Provider::Ollama));
        let ollama = &settings.providers.ollama;
        assert_eq!(ollama.model, "qwen3");
        assert_eq!(ollama.thinking, Some(ThinkingLevel::Medium));
        assert_eq!(ollama.requests_at_once, Some(3));
    }

    #[test]
    fn the_address_setter_trims_and_an_empty_value_gives_the_usual_address() {
        let mut settings = Settings::default();
        settings
            .set_provider_address(Provider::Ollama, "  http://studio.local:11434 ")
            .unwrap();
        assert_eq!(
            settings.provider_address(Provider::Ollama),
            "http://studio.local:11434"
        );
        settings
            .set_provider_address(Provider::Ollama, "  ")
            .unwrap();
        assert_eq!(settings.providers.ollama.address, None);
        assert_eq!(
            settings.provider_address(Provider::Ollama),
            "http://localhost:11434"
        );
        settings
            .set_provider_address(Provider::Custom, "https://litellm.ewal.io")
            .unwrap();
        settings.set_provider_address(Provider::Custom, "").unwrap();
        assert_eq!(settings.provider_address(Provider::Custom), "");
    }

    #[test]
    fn the_address_setter_refuses_a_cloud_provider_and_a_url_with_no_host() {
        let mut settings = Settings::default();
        let before = settings.clone();
        assert!(
            settings
                .set_provider_address(Provider::OpenAi, "https://example.com/v1")
                .is_err()
        );
        let error = settings
            .set_provider_address(Provider::Custom, "not a url")
            .unwrap_err();
        assert_eq!(
            error,
            secrets::endpoint_key("not a url").unwrap_err().to_string()
        );
        assert_eq!(settings, before);
    }

    #[test]
    fn the_check_saves_the_resolved_v1_address_for_custom() {
        // Covers KTD9, AE3.
        let mut settings = Settings::default();
        settings
            .set_provider_address(Provider::Custom, "https://litellm.example.com")
            .unwrap();
        assert!(settings.save_checked_address(
            Provider::Custom,
            "https://litellm.example.com",
            "https://litellm.example.com/v1",
        ));
        assert_eq!(
            settings.provider_address(Provider::Custom),
            "https://litellm.example.com/v1"
        );
    }

    #[test]
    fn the_check_does_not_save_when_the_address_changed_while_it_ran() {
        let mut settings = Settings::default();
        settings
            .set_provider_address(Provider::Custom, "https://other.example.com")
            .unwrap();
        assert!(!settings.save_checked_address(
            Provider::Custom,
            "https://litellm.example.com",
            "https://litellm.example.com/v1",
        ));
        assert_eq!(
            settings.provider_address(Provider::Custom),
            "https://other.example.com"
        );
    }

    #[test]
    fn the_check_saves_no_address_for_a_provider_other_than_custom() {
        let mut settings = Settings::default();
        let before = settings.clone();
        assert!(!settings.save_checked_address(
            Provider::Ollama,
            "http://localhost:11434",
            "http://localhost:11434/v1",
        ));
        assert_eq!(settings, before);
    }

    #[test]
    fn with_no_provider_the_gate_sends_nothing() {
        // Covers AE2.
        for key in [KeyState::Saved, KeyState::Absent, KeyState::Locked] {
            assert_eq!(
                provider_gate(None, true, "https://api.openai.com/v1", key),
                EndpointGate::NotRequested(NotRequestedReason::NoProvider)
            );
        }
        assert!(!provider_ready(&Settings::default(), KeyState::Saved));
    }

    #[test]
    fn a_cloud_provider_with_no_key_is_not_set_up_and_a_chosen_local_provider_is_allowed() {
        use EndpointGate::*;
        use KeyState::*;
        use NotRequestedReason::*;
        let openai = "https://api.openai.com/v1";
        let ollama = "http://localhost:11434";
        for cloud in [
            Provider::OpenAi,
            Provider::Anthropic,
            Provider::Gemini,
            Provider::OpenRouter,
        ] {
            assert_eq!(
                provider_gate(Some(cloud), true, openai, Absent),
                NotRequested(KeyNeeded)
            );
            assert_eq!(
                provider_gate(Some(cloud), false, openai, Saved),
                RequestWithKey
            );
        }
        for keyless in [Provider::Ollama, Provider::LmStudio, Provider::Custom] {
            assert_eq!(
                provider_gate(Some(keyless), true, ollama, Absent),
                RequestWithoutKey
            );
            assert_eq!(
                provider_gate(Some(keyless), false, ollama, Absent),
                NotRequested(NotChosen)
            );
        }
        assert_eq!(
            provider_gate(Some(Provider::Custom), true, "  ", Saved),
            NotRequested(NoAddress)
        );
        assert_eq!(
            provider_gate(Some(Provider::Custom), true, ollama, Locked),
            NotRequested(KeychainLocked)
        );
        assert_eq!(
            provider_gate(Some(Provider::OpenAi), true, openai, Failed),
            NotRequested(KeychainFailed)
        );
    }

    #[test]
    fn a_blank_saved_key_counts_as_no_key() {
        assert_eq!(key_state(&Ok(Some("sk-1".into()))), KeyState::Saved);
        assert_eq!(key_state(&Ok(Some("   ".into()))), KeyState::Absent);
        assert_eq!(key_state(&Ok(Some(String::new()))), KeyState::Absent);
        assert_eq!(key_state(&Ok(None)), KeyState::Absent);
        // A blank key does not open the gate for a cloud provider.
        let key = key_state(&Ok(Some(" \t".into())));
        assert_eq!(
            provider_gate(
                Some(Provider::OpenAi),
                true,
                "https://api.openai.com/v1",
                key
            ),
            EndpointGate::NotRequested(NotRequestedReason::KeyNeeded)
        );
    }

    #[test]
    fn only_a_provider_that_takes_a_key_and_has_an_address_has_a_key_location() {
        let mut settings = Settings {
            legacy_key_provider: Some(Provider::Custom),
            ..Settings::default()
        };
        assert_eq!(settings.key_location(Provider::Ollama), None);
        assert_eq!(settings.key_location(Provider::LmStudio), None);
        assert_eq!(settings.key_location(Provider::Custom), None);
        settings
            .set_provider_address(Provider::Custom, "https://openrouter.ai/api/v1")
            .unwrap();
        let custom = settings.key_location(Provider::Custom).unwrap();
        assert!(custom.legacy);
        assert_eq!(custom.host(), "openrouter.ai");
        let openrouter = settings.key_location(Provider::OpenRouter).unwrap();
        assert!(!openrouter.legacy);
        assert_eq!(openrouter.host(), "openrouter.ai");
    }

    #[test]
    fn readiness_applies_the_same_gate_as_summary_processing() {
        use KeyState::*;
        let mut openai = Settings::default();
        openai.choose_provider(Some(Provider::OpenAi));
        openai.set_provider_model(Provider::OpenAi, "gpt-4o");
        assert!(!provider_ready(&openai, Absent));
        assert!(provider_ready(&openai, Saved));
        // A Keychain problem has its own message. It is not "unconfigured".
        assert!(provider_ready(&openai, Locked));
        assert!(provider_ready(&openai, Failed));

        let mut ollama = openai.clone();
        ollama.choose_provider(Some(Provider::Ollama));
        ollama.set_provider_model(Provider::Ollama, "llama3");
        assert!(provider_ready(&ollama, Absent));

        let mut custom = openai.clone();
        custom.choose_provider(Some(Provider::Custom));
        custom.set_provider_model(Provider::Custom, "qwen3");
        let mut no_model = ollama.clone();
        no_model.set_provider_model(Provider::Ollama, "");
        for key in [Saved, Absent, Locked, Failed] {
            // Custom with no address, and a provider with no model.
            assert!(!provider_ready(&custom, key));
            assert!(!provider_ready(&no_model, key));
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

    const ENGLISH_ONLY: &str = "parakeet-unified-en-0.6b-q80";
    const MULTILINGUAL: &str = "nemotron-3.5-asr-streaming-0.6b-q80";

    fn choice(
        asr_model: &str,
        speaker_source: SpeakerSource,
        speaker_processing_mode: ProcessingMode,
        language: Option<&str>,
        count: Option<u32>,
    ) -> RecognitionChoice {
        RecognitionChoice {
            asr_model: asr_model.into(),
            live_transcript: true,
            speaker_source,
            speaker_processing_mode,
            recognition_language: language.map(Into::into),
            default_speaker_count: count.map(|count| SpeakerCountSetting::Exact { count }),
        }
    }

    #[test]
    fn a_language_the_model_does_not_list_falls_back_to_automatic() {
        let english = choice(
            ENGLISH_ONLY,
            SpeakerSource::Pyannote,
            ProcessingMode::AfterStop,
            Some("de"),
            None,
        );
        let config = english.config();
        assert_eq!(config.language, None);
        models::recognition::resolve(&config).unwrap();
        assert_eq!(english.recognition_language.as_deref(), Some("de"));

        let multilingual = RecognitionChoice {
            asr_model: MULTILINGUAL.into(),
            ..english
        };
        assert_eq!(multilingual.config().language.as_deref(), Some("de"));
    }

    #[test]
    fn live_speakers_fall_back_only_when_the_source_has_no_live_mode() {
        let pyannote = choice(
            ENGLISH_ONLY,
            SpeakerSource::Pyannote,
            ProcessingMode::DuringRecording,
            None,
            Some(9),
        );
        let config = pyannote.config();
        assert_eq!(config.speaker_processing_mode, ProcessingMode::AfterStop);
        models::recognition::resolve(&config).unwrap();
        assert_eq!(
            pyannote.speaker_processing_mode,
            ProcessingMode::DuringRecording
        );

        let nemotron = choice(
            ENGLISH_ONLY,
            SpeakerSource::Nemotron,
            ProcessingMode::DuringRecording,
            None,
            Some(3),
        );
        let config = nemotron.config();
        assert_eq!(
            config.speaker_processing_mode,
            ProcessingMode::DuringRecording
        );
        models::recognition::resolve(&config).unwrap();
    }

    #[test]
    fn a_count_above_the_source_limit_keeps_its_error() {
        for mode in [ProcessingMode::AfterStop, ProcessingMode::DuringRecording] {
            let config =
                choice(ENGLISH_ONLY, SpeakerSource::Nemotron, mode, None, Some(9)).config();
            assert_eq!(config.speaker_source, SpeakerSource::Nemotron);
            assert!(matches!(
                models::recognition::resolve(&config),
                Err(models::recognition::ResolveError::SpeakerLimit { .. })
            ));
        }
    }

    #[test]
    fn two_unsupported_preferences_fall_back_together() {
        let both = choice(
            ENGLISH_ONLY,
            SpeakerSource::Pyannote,
            ProcessingMode::DuringRecording,
            Some("de"),
            None,
        );
        let config = both.config();
        assert_eq!(config.language, None);
        assert_eq!(config.speaker_processing_mode, ProcessingMode::AfterStop);
        models::recognition::resolve(&config).unwrap();
        assert_eq!(both.recognition_language.as_deref(), Some("de"));
        assert_eq!(
            both.speaker_processing_mode,
            ProcessingMode::DuringRecording
        );
    }

    /// An older build on this computer reads the local values in use as
    /// they are, with no fallback. They must resolve, so recording starts.
    #[test]
    fn the_configuration_in_use_keeps_only_values_its_model_can_use() {
        let chosen = choice(
            ENGLISH_ONLY,
            SpeakerSource::Pyannote,
            ProcessingMode::DuringRecording,
            Some("de"),
            None,
        );
        let mut settings = Settings {
            chosen_recognition: chosen.clone(),
            ..Settings::default()
        };
        settings.set_in_use(&chosen);
        assert_eq!(settings.recognition_language, None);
        assert_eq!(settings.speaker_processing_mode, ProcessingMode::AfterStop);
        assert!(!settings.recognition.differs);
        assert_eq!(settings.chosen_recognition, chosen);

        // The configuration as an older build reads it from the local values.
        let streaming = models::model(ENGLISH_ONLY).unwrap().streaming;
        models::recognition::resolve(&models::recognition::RecognitionConfig {
            asr_model: settings.asr_model.clone(),
            speaker_source: settings.speaker_source,
            processing_mode: if settings.live_transcript && streaming {
                ProcessingMode::DuringRecording
            } else {
                ProcessingMode::AfterStop
            },
            speaker_count: settings.default_speaker_count.map(Into::into),
            language: settings.recognition_language.clone(),
            speaker_processing_mode: settings.speaker_processing_mode,
        })
        .unwrap();
    }

    #[test]
    fn a_stored_configuration_in_use_with_a_preference_it_cannot_use_does_not_differ() {
        let chosen = choice(
            ENGLISH_ONLY,
            SpeakerSource::Pyannote,
            ProcessingMode::AfterStop,
            Some("de"),
            None,
        );
        let local = LocalSettings {
            asr_model: ENGLISH_ONLY.into(),
            live_transcript: true,
            speaker_source: SpeakerSource::Pyannote,
            speaker_processing_mode: ProcessingMode::AfterStop,
            recognition_language: Some("de".into()),
            default_speaker_count: None,
            ..LocalSettings::default()
        };
        assert!(chosen.same_use(&local.in_use()));
    }

    #[test]
    fn live_transcript_on_a_model_that_does_not_stream_stays_a_preference() {
        let whisper = choice(
            "whisper-large-v3-turbo-q5km",
            SpeakerSource::Pyannote,
            ProcessingMode::AfterStop,
            None,
            None,
        );
        assert_eq!(whisper.config().processing_mode, ProcessingMode::AfterStop);
        assert!(whisper.live_transcript);
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
            active_provider: Some(Provider::Custom),
            providers: ProviderEntries {
                custom: ProviderEntry {
                    address: Some("http://localhost:11434/v1".into()),
                    model: "llama3".into(),
                    context_limit_tokens: None,
                    ..Default::default()
                },
                ..ProviderEntries::default()
            },
            legacy_key_provider: Some(Provider::Custom),
            storage_folder: "/vault/a".into(),
            chosen_recognition: recognition.clone(),
            ..Settings::default()
        };
        expected.set_in_use(&recognition);
        // The update writes the stored language as it is.
        expected.recognition_language = recognition.recognition_language.clone();
        // The chosen configuration and the configuration in use are the
        // same after the update, so nothing downloads (R18).
        assert!(!expected.recognition.differs);
        assert_eq!(settings, expected);
        let written = serde_json::to_value(settings.local()).unwrap();
        assert_eq!(settings_from_value(&written), (settings.local(), false));
    }
}
