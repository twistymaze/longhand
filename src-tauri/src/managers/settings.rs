//! Settings in the application's own configuration store (R33, KTD15).
//!
//! Shape follows Handy's `settings.rs` (one `#[serde(default)]` struct, a
//! schema-version integer, migrations keyed on the stored version, salvage of
//! individually valid fields); see src-tauri/PROVENANCE.md. Portable mode is
//! omitted. Credentials are not settings: they live in the OS secret store,
//! keyed by endpoint host (see `managers::secrets`).

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use summarize::prompts::{PROMPT_TEXT_LIMIT, TOPIC_SUMMARY_ID};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

/// Relative, so `tauri-plugin-store` resolves it under the app data directory.
pub const SETTINGS_STORE_PATH: &str = "settings.json";
const SETTINGS_KEY: &str = "settings";

pub const CURRENT_SETTINGS_SCHEMA_VERSION: u32 = 5;

/// The Base URL that Longhand uses when the user never saved one (R1, KD3).
pub const DEFAULT_ENDPOINT_BASE_URL: &str = "https://api.openai.com/v1";

/// The recognition model that versions 1 to 3 selected by default. A store
/// of those versions with no `asr_model` read as this model, so the
/// migration to version 4 keeps it (KTD11).
const PREVIOUS_DEFAULT_ASR_MODEL: &str = "whisper-large-v3-turbo-q5km";

/// The chord a fresh install records with. Obscure enough to collide with
/// nothing common; the Phase 2a probe proved it registers on macOS.
pub const DEFAULT_SHORTCUT: &str = "CommandOrControl+Alt+Shift+L";

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
}

/// One row of the prompt list that the Settings window shows.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct SummaryPromptEntry {
    pub id: String,
    pub name: String,
    pub text: String,
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
}

const NEW_PROMPT_NAME: &str = "New prompt";
const NO_SUCH_PROMPT: &str = "That prompt no longer exists.";
const BUILT_IN_PROMPT: &str = "A built-in prompt can't be changed. Duplicate it to make your own.";

#[derive(Serialize, Deserialize, Type, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub settings_schema_version: u32,
    pub interface_zoom_percent: u16,
    /// The chord that starts and stops a recording. `None` when the user
    /// cleared it (R3). A stored string reads as `Some`, so no migration is
    /// needed (KTD3).
    pub shortcut: Option<String>,
    /// The chord that shows or hides the main window. Off by default (R5).
    pub show_window_shortcut: Option<String>,
    /// Every folder the user has ever chosen, oldest first (R25a). The index
    /// is rebuilt by scanning these, so a root is never dropped silently.
    pub destination_roots: Vec<String>,
    /// Where the next session is saved (R23). Always one of the roots.
    pub current_destination: Option<String>,
    /// OpenAI-compatible base URL, e.g. `https://api.openai.com/v1` (R32).
    /// `None` means a new install that has no settings yet, and
    /// [`DEFAULT_ENDPOINT_BASE_URL`] applies. An empty value means that the
    /// user cleared it, or that an existing install never set it (R2, KTD4).
    pub endpoint_base_url: Option<String>,
    pub endpoint_model: String,
    /// Overrides the probed context budget when set (KTD16).
    pub context_limit_tokens: Option<u32>,
    pub default_speaker_count: Option<SpeakerCountSetting>,
    /// Catalog id of the recognition model (KTD6).
    pub asr_model: String,
    /// A picked recognition model that is downloading. It becomes
    /// `asr_model` when its download is verified (R5, KTD11).
    pub pending_asr_model: Option<String>,
    /// A streaming model transcribes during recording (R18, KD9).
    pub live_transcript: bool,
    pub speaker_source: models::recognition::SpeakerSource,
    pub speaker_processing_mode: models::recognition::ProcessingMode,
    pub recognition_language: Option<String>,
    pub pending_recognition: Option<models::recognition::RecognitionConfig>,
    /// The microphone a new note records from. `None` is "System default"
    /// (R1, KTD17).
    pub microphone: Option<MicrophoneChoice>,
    /// Whether a new note starts with system audio on (R6, KD13).
    pub system_audio_default: bool,
    /// macOS only: the app has no Dock icon while no window is visible
    /// (R2, R3). Off by default: the Dock icon stays, as is usual on macOS.
    pub hide_from_dock: bool,
    /// Context limits endpoints have shown by rejecting longer prompts,
    /// keyed by [`learned_limit_key`] (KTD16).
    pub learned_context_limits: std::collections::BTreeMap<String, u32>,
    /// The custom summary prompts, in the order the user made them (KTD6).
    pub summary_prompts: Vec<SummaryPrompt>,
    /// The id of the prompt that the next summary uses. An id that matches
    /// no prompt resolves to the Topic summary.
    pub active_summary_prompt: String,
}

/// The settings changed without a command from the surface, for example
/// when a picked model becomes the selected model after its download.
#[derive(Serialize, Type, Debug, Clone, tauri_specta::Event)]
pub struct SettingsChanged {
    pub settings: Settings,
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
            destination_roots: Vec::new(),
            current_destination: None,
            endpoint_base_url: None,
            endpoint_model: String::new(),
            context_limit_tokens: None,
            default_speaker_count: None,
            asr_model: models::DEFAULT_ASR_MODEL.to_string(),
            pending_asr_model: None,
            live_transcript: true,
            speaker_source: Default::default(),
            speaker_processing_mode: Default::default(),
            recognition_language: None,
            pending_recognition: None,
            microphone: None,
            system_audio_default: false,
            hide_from_dock: false,
            learned_context_limits: Default::default(),
            summary_prompts: Vec::new(),
            active_summary_prompt: TOPIC_SUMMARY_ID.to_string(),
        }
    }
}

impl Settings {
    pub fn recognition_config(&self) -> models::recognition::RecognitionConfig {
        use models::recognition::{ProcessingMode, RecognitionConfig, SpeakerCount};
        let live_supported = models::model(&self.asr_model).is_some_and(|m| m.streaming);
        RecognitionConfig {
            asr_model: self.asr_model.clone(),
            speaker_source: self.speaker_source,
            processing_mode: if self.live_transcript && live_supported {
                ProcessingMode::DuringRecording
            } else {
                ProcessingMode::AfterStop
            },
            speaker_count: self.default_speaker_count.map(|c| match c {
                SpeakerCountSetting::Exact { count } => SpeakerCount::Exact { count },
                SpeakerCountSetting::Range { min, max } => SpeakerCount::Range { min, max },
            }),
            language: self.recognition_language.clone(),
            speaker_processing_mode: self.speaker_processing_mode,
        }
    }

    pub fn apply_recognition(&mut self, config: &models::recognition::RecognitionConfig) {
        use models::recognition::{ProcessingMode, SpeakerCount};
        self.asr_model = config.asr_model.clone();
        self.speaker_source = config.speaker_source;
        self.speaker_processing_mode = config.speaker_processing_mode;
        self.live_transcript = config.processing_mode == ProcessingMode::DuringRecording;
        self.recognition_language = config.language.clone();
        self.default_speaker_count = config.speaker_count.map(|c| match c {
            SpeakerCount::Exact { count } => SpeakerCountSetting::Exact { count },
            SpeakerCount::Range { min, max } => SpeakerCountSetting::Range { min, max },
        });
        self.pending_recognition = None;
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

    /// Make `folder` the current destination and retain it as a root (R23, R25a).
    pub fn choose_destination(&mut self, folder: &str) {
        if !self.destination_roots.iter().any(|r| r == folder) {
            self.destination_roots.push(folder.to_string());
        }
        self.current_destination = Some(folder.to_string());
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
    /// the Topic summary.
    pub fn active_summary_prompt_snapshot(&self) -> summarize::Prompt {
        self.find_summary_prompt(&self.active_summary_prompt)
            .unwrap_or_else(topic_summary)
    }

    pub fn summary_prompt_list(&self) -> SummaryPromptList {
        let built_ins = summarize::prompts::built_ins()
            .into_iter()
            .map(|p| SummaryPromptEntry {
                id: p.id,
                name: p.name,
                text: p.text,
                built_in: true,
            });
        let custom = self.summary_prompts.iter().map(|p| SummaryPromptEntry {
            id: p.id.clone(),
            name: p.name.clone(),
            text: p.text.clone(),
            built_in: false,
        });
        SummaryPromptList {
            prompts: built_ins.chain(custom).collect(),
            active: self.active_summary_prompt_snapshot().id,
            text_limit: summarize::prompts::PROMPT_TEXT_LIMIT as u32,
        }
    }

    /// Add a prompt with the Topic summary text (R3). Returns its id.
    pub fn create_summary_prompt(&mut self) -> String {
        let name = self.free_prompt_name(NEW_PROMPT_NAME);
        self.add_summary_prompt(name, topic_summary().text)
    }

    /// Add a copy of the prompt `id`, named "X copy" (R3). Returns its id.
    pub fn duplicate_summary_prompt(&mut self, id: &str) -> Result<String, String> {
        let source = self.find_summary_prompt(id).ok_or(NO_SUCH_PROMPT)?;
        let name = self.free_prompt_name(&format!("{} copy", source.name));
        Ok(self.add_summary_prompt(name, source.text))
    }

    pub fn rename_summary_prompt(&mut self, id: &str, name: &str) -> Result<(), String> {
        let index = self.custom_prompt_index(id)?;
        let name = name.trim();
        if name.is_empty() {
            return Err("Enter a name for the prompt.".into());
        }
        let taken = self
            .all_prompt_names()
            .any(|(other, existing)| other != id && same_name(&existing, name));
        if taken {
            return Err("A prompt with that name already exists.".into());
        }
        self.summary_prompts[index].name = name.to_string();
        Ok(())
    }

    pub fn set_summary_prompt_text(&mut self, id: &str, text: &str) -> Result<(), String> {
        let index = self.custom_prompt_index(id)?;
        if text.trim().is_empty() {
            return Err("Enter the text for the prompt.".into());
        }
        if text.chars().count() > PROMPT_TEXT_LIMIT {
            return Err(format!(
                "The prompt text is too long. Use {} characters or fewer.",
                format_count(PROMPT_TEXT_LIMIT)
            ));
        }
        self.summary_prompts[index].text = text.to_string();
        Ok(())
    }

    /// Remove a custom prompt. When it was active, the Topic summary
    /// becomes active (R4).
    pub fn delete_summary_prompt(&mut self, id: &str) -> Result<(), String> {
        let index = self.custom_prompt_index(id)?;
        self.summary_prompts.remove(index);
        if self.active_summary_prompt == id {
            self.active_summary_prompt = TOPIC_SUMMARY_ID.to_string();
        }
        Ok(())
    }

    pub fn activate_summary_prompt(&mut self, id: &str) -> Result<(), String> {
        if self.find_summary_prompt(id).is_none() {
            return Err(NO_SUCH_PROMPT.into());
        }
        self.active_summary_prompt = id.to_string();
        Ok(())
    }

    fn find_summary_prompt(&self, id: &str) -> Option<summarize::Prompt> {
        summarize::prompts::built_in(id).or_else(|| {
            self.summary_prompts
                .iter()
                .find(|p| p.id == id)
                .map(|p| summarize::Prompt {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    text: p.text.clone(),
                })
        })
    }

    /// The position of the custom prompt `id`. A built-in id or an unknown
    /// id fails.
    fn custom_prompt_index(&self, id: &str) -> Result<usize, String> {
        if summarize::prompts::built_in(id).is_some() {
            return Err(BUILT_IN_PROMPT.into());
        }
        self.summary_prompts
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| NO_SUCH_PROMPT.into())
    }

    fn add_summary_prompt(&mut self, name: String, text: String) -> String {
        // A random id never matches a built-in id, which is a fixed word.
        let id = uuid::Uuid::new_v4().to_string();
        self.summary_prompts.push(SummaryPrompt {
            id: id.clone(),
            name,
            text,
        });
        id
    }

    /// The id and name of every prompt, built-in and custom.
    fn all_prompt_names(&self) -> impl Iterator<Item = (String, String)> + '_ {
        summarize::prompts::built_ins()
            .into_iter()
            .map(|p| (p.id, p.name))
            .chain(
                self.summary_prompts
                    .iter()
                    .map(|p| (p.id.clone(), p.name.clone())),
            )
    }

    /// `base`, or `base 2`, `base 3`, … when that name is taken (R3).
    fn free_prompt_name(&self, base: &str) -> String {
        let taken: Vec<String> = self.all_prompt_names().map(|(_, name)| name).collect();
        let is_free = |candidate: &str| !taken.iter().any(|t| same_name(t, candidate));
        if is_free(base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|candidate| is_free(candidate))
            .expect("an unbounded range always gives a free name")
    }
}

/// Names are compared trimmed and without case (KTD7).
fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

fn topic_summary() -> summarize::Prompt {
    summarize::prompts::built_in(TOPIC_SUMMARY_ID).expect("the Topic summary is a built-in prompt")
}

/// `4000` as "4,000", for UI text.
fn format_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Parse a stored value into settings. Returns whether the stored form
/// differs from what would be written back, so the caller persists the
/// migration once and then converges.
pub fn settings_from_value(stored: &Value) -> (Settings, bool) {
    let (mut settings, mut updated) = match serde_json::from_value::<Settings>(stored.clone()) {
        Ok(settings) => (settings, false),
        Err(e) => {
            log::warn!("failed to parse stored settings ({e}); salvaging valid fields");
            (salvage_settings(stored), true)
        }
    };
    if apply_settings_migrations(&mut settings, stored) {
        updated = true;
    }
    (settings, updated)
}

fn apply_settings_migrations(settings: &mut Settings, stored: &Value) -> bool {
    // An absent version is 0: a store written before versioning existed.
    let stored_version = stored
        .get("settings_schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut updated = false;
    if stored_version < 1 {
        // Nothing to transform; the version is stamped below.
        updated = true;
    }
    if stored_version < 2 {
        // Version 1 carried only the Phase 2a probe field, which is dropped;
        // every real field takes its default.
        updated = true;
    }
    if stored_version < 3 {
        // Every existing install changes to the new default prompt (KTD6).
        settings.active_summary_prompt = TOPIC_SUMMARY_ID.to_string();
        updated = true;
    }
    if stored_version < 4 {
        // Version 4 changes the default model. An existing install keeps the
        // model it used, and only a store with no settings gets the new
        // default (R3, KTD11). Version 0 is a store with no version at all.
        if stored_version >= 1 && stored.get("asr_model").is_none() {
            settings.asr_model = PREVIOUS_DEFAULT_ASR_MODEL.to_string();
        }
        updated = true;
    }
    if stored_version < 5 {
        // Serde defaults preserve the existing separate speaker pipeline.
        updated = true;
    }
    let zoom = crate::interface_zoom::normalize(settings.interface_zoom_percent);
    if zoom != settings.interface_zoom_percent {
        settings.interface_zoom_percent = zoom;
        updated = true;
    }
    if drop_unusable_prompts(&mut settings.summary_prompts) {
        updated = true;
    }
    if updated {
        settings.settings_schema_version = CURRENT_SETTINGS_SCHEMA_VERSION;
    }
    updated
}

/// Remove each stored prompt whose id is a built-in id or repeats an earlier
/// id: such a prompt could never be selected apart from the other one.
/// Returns whether a prompt was removed.
fn drop_unusable_prompts(prompts: &mut Vec<SummaryPrompt>) -> bool {
    let before = prompts.len();
    let mut seen = std::collections::HashSet::new();
    prompts.retain(|p| {
        let usable = summarize::prompts::built_in(&p.id).is_none() && seen.insert(p.id.clone());
        if !usable {
            log::warn!("dropping a stored summary prompt with a built-in or repeated id");
        }
        usable
    });
    prompts.len() != before
}

/// Keep only the entries of a stored prompt list that are individually
/// valid, so one bad entry never drops the other prompts.
fn salvage_prompts(value: &Value) -> Value {
    let Some(entries) = value.as_array() else {
        return value.clone();
    };
    let kept: Vec<Value> = entries
        .iter()
        .filter(|entry| {
            let valid = serde_json::from_value::<SummaryPrompt>((*entry).clone()).is_ok();
            if !valid {
                log::warn!("dropping an invalid stored summary prompt");
            }
            valid
        })
        .cloned()
        .collect();
    Value::Array(kept)
}

/// Rebuild settings from a stored value that failed to deserialize as a
/// whole, keeping every field that is individually valid so one bad value
/// never resets the rest.
fn salvage_settings(stored: &Value) -> Settings {
    let Some(stored_map) = stored.as_object() else {
        log::warn!("stored settings are not a JSON object; falling back to defaults");
        return Settings::default();
    };
    let mut merged =
        serde_json::to_value(Settings::default()).expect("default settings serialize to an object");
    for (key, value) in stored_map {
        let value = if key == "summary_prompts" {
            &salvage_prompts(value)
        } else {
            value
        };
        let map = merged
            .as_object_mut()
            .expect("merged settings stay an object");
        let previous = map.insert(key.clone(), value.clone());
        if serde_json::from_value::<Settings>(merged.clone()).is_err() {
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

pub fn get_settings(app: &AppHandle) -> Settings {
    let store = match app.store(SETTINGS_STORE_PATH) {
        Ok(store) => store,
        Err(e) => {
            log::error!("failed to open settings store: {e}; using defaults");
            return Settings::default();
        }
    };
    let (settings, updated) = match store.get(SETTINGS_KEY) {
        Some(stored) => settings_from_value(&stored),
        None => (Settings::default(), true),
    };
    if updated {
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

/// Read the settings and write nothing: a migrated value is not saved.
/// Launch reads the Dock policy with this before the single-instance lock
/// (U19), so a second copy that exits changes no file.
pub fn read_settings(app: &AppHandle) -> Settings {
    app.store(SETTINGS_STORE_PATH)
        .ok()
        .and_then(|store| store.get(SETTINGS_KEY))
        .map(|stored| settings_from_value(&stored).0)
        .unwrap_or_default()
}

/// Serializes read-modify-write so two commands changing different fields
/// never lose one another's change.
static UPDATE_LOCK: Mutex<()> = Mutex::new(());

/// Apply `change` to the stored settings and persist the result. Nothing is
/// written when `change` refuses.
pub fn update_settings(
    app: &AppHandle,
    change: impl FnOnce(&mut Settings) -> Result<(), String>,
) -> Result<Settings, String> {
    let _guard = UPDATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut settings = get_settings(app);
    change(&mut settings)?;
    write_settings(app, &settings)?;
    Ok(settings)
}

/// Write and flush immediately: the store's autosave is debounced, and a
/// setting changed just before quitting must survive the quit.
fn write_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use summarize::prompts::PLAY_BY_PLAY_ID;

    use super::*;

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
                &serde_json::json!({ "settings_schema_version": 4, "interface_zoom_percent": value, "live_transcript": false }),
            );
            assert_eq!(settings.interface_zoom_percent, 100);
            assert!(!settings.live_transcript);
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
        assert_eq!(settings, Settings::default());
        assert_eq!(
            settings.settings_schema_version,
            CURRENT_SETTINGS_SCHEMA_VERSION
        );

        let written = serde_json::to_value(&settings).unwrap();
        assert_eq!(settings_from_value(&written), (settings, false));
    }

    #[test]
    fn a_phase_2a_store_migrates_onto_the_real_schema() {
        let stored = json!({ "settings_schema_version": 1, "probe_value": "Test" });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        // The install existed before the default changed, so it keeps the
        // model it read before (KTD11).
        assert_eq!(
            settings,
            Settings {
                asr_model: PREVIOUS_DEFAULT_ASR_MODEL.to_string(),
                ..Settings::default()
            }
        );
        let written = serde_json::to_value(&settings).unwrap();
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
        assert_eq!(settings.pending_asr_model, None);
        assert_eq!(settings_from_value(&json!({})).0, settings);
    }

    #[test]
    fn a_version_2_store_keeps_its_whisper_model() {
        let stored = json!({
            "settings_schema_version": 2,
            "asr_model": "whisper-large-v3-turbo-q5km",
            "endpoint_model": "kept",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(
            settings.settings_schema_version,
            CURRENT_SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(settings.asr_model, "whisper-large-v3-turbo-q5km");
        assert_eq!(settings.endpoint_model, "kept");
        assert!(settings.live_transcript);
        assert_eq!(settings.pending_asr_model, None);
        let written = serde_json::to_value(&settings).unwrap();
        assert_eq!(settings_from_value(&written), (settings, false));
    }

    #[test]
    fn a_version_2_store_without_a_model_keeps_the_previous_default() {
        let stored = json!({ "settings_schema_version": 2, "endpoint_model": "x" });
        let (settings, _) = settings_from_value(&stored);
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
        let (settings, updated) = settings_from_value(&stored);
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
        let (settings, _) = settings_from_value(&picker_only);
        assert_eq!(settings.asr_model, "nemotron-speech-streaming-en-0.6b-q80");
        assert_eq!(settings.active_summary_prompt, TOPIC_SUMMARY_ID);
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
        let written = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            written["microphone"],
            json!({ "id": "CoreAudio:AppleUSBAudioEngine:1", "label": "USB Audio" })
        );
        let (read, updated) = settings_from_value(&written);
        assert!(!updated);
        assert_eq!(read.microphone, Some(usb_mic()));

        settings.set_microphone(None).unwrap();
        let written = serde_json::to_value(&settings).unwrap();
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
        let (settings, updated) =
            settings_from_value(&json!({ "endpoint_model": "gpt-5", "destination_roots": ["/a"] }));
        assert!(updated);
        assert_eq!(settings.endpoint_model, "gpt-5");
        assert_eq!(settings.destination_roots, ["/a"]);
    }

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
        assert_eq!(settings, Settings::default());
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
        let (settings, _) = settings_from_value(&json!({}));
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
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(settings.endpoint_model, "gpt-5");
        assert_eq!(settings.active_summary_prompt, TOPIC_SUMMARY_ID);
        assert_eq!(
            settings.settings_schema_version,
            CURRENT_SETTINGS_SCHEMA_VERSION
        );
    }

    #[test]
    fn a_current_store_keeps_its_active_prompt() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "active_summary_prompt": PLAY_BY_PLAY_ID,
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(settings.active_summary_prompt, PLAY_BY_PLAY_ID);
    }

    #[test]
    fn one_malformed_prompt_entry_keeps_the_other_entries() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "endpoint_model": "kept",
            "summary_prompts": [
                { "id": "a", "name": "Team sync", "text": "Recap." },
                { "id": "b", "name": 7 },
                { "id": "c", "name": "One on one", "text": "Notes." },
            ],
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(settings.endpoint_model, "kept");
        assert_eq!(names(&settings), ["Team sync", "One on one"]);
    }

    #[test]
    fn a_stored_prompt_with_a_built_in_or_repeated_id_is_dropped() {
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION,
            "summary_prompts": [
                { "id": TOPIC_SUMMARY_ID, "name": "Shadow", "text": "x" },
                { "id": "a", "name": "Team sync", "text": "Recap." },
                { "id": "a", "name": "Again", "text": "x" },
            ],
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(names(&settings), ["Team sync"]);
    }

    #[test]
    fn create_names_new_prompts_in_turn_with_the_topic_summary_text() {
        let mut settings = Settings::default();
        settings.create_summary_prompt();
        settings.create_summary_prompt();
        assert_eq!(names(&settings), ["New prompt", "New prompt 2"]);
        let topic = summarize::prompts::built_in(TOPIC_SUMMARY_ID).unwrap();
        for prompt in &settings.summary_prompts {
            assert_eq!(prompt.text, topic.text);
            assert!(summarize::prompts::built_in(&prompt.id).is_none());
        }
        assert_ne!(
            settings.summary_prompts[0].id,
            settings.summary_prompts[1].id
        );
    }

    #[test]
    fn duplicate_names_the_copy_in_turn() {
        let mut settings = Settings::default();
        settings.duplicate_summary_prompt(TOPIC_SUMMARY_ID).unwrap();
        settings.duplicate_summary_prompt(TOPIC_SUMMARY_ID).unwrap();
        assert_eq!(
            names(&settings),
            ["Topic summary copy", "Topic summary copy 2"]
        );

        settings.summary_prompts.push(custom("g", "Team sync"));
        settings.duplicate_summary_prompt("g").unwrap();
        let copy = settings.summary_prompts.last().unwrap();
        assert_eq!(copy.name, "Team sync copy");
        assert_eq!(copy.text, "Write a recap.");
        assert_ne!(copy.id, "g");

        assert!(settings.duplicate_summary_prompt("missing").is_err());
    }

    #[test]
    fn rename_refuses_a_taken_empty_or_built_in_name() {
        let mut settings = Settings::default();
        settings.summary_prompts.push(custom("g", "Team sync"));
        settings.summary_prompts.push(custom("o", "One on one"));

        assert!(
            settings
                .rename_summary_prompt("g", "  topic SUMMARY ")
                .is_err()
        );
        assert!(settings.rename_summary_prompt("g", "one ON one").is_err());
        assert!(settings.rename_summary_prompt("g", "   ").is_err());
        assert!(
            settings
                .rename_summary_prompt(TOPIC_SUMMARY_ID, "Mine")
                .is_err()
        );
        assert!(settings.rename_summary_prompt("missing", "Mine").is_err());
        assert_eq!(names(&settings), ["Team sync", "One on one"]);

        settings
            .rename_summary_prompt("g", "  Team meeting ")
            .unwrap();
        // A change of case on the prompt's own name is not a clash.
        settings.rename_summary_prompt("o", "One On One").unwrap();
        assert_eq!(names(&settings), ["Team meeting", "One On One"]);
    }

    #[test]
    fn set_text_refuses_empty_or_too_long_text_and_built_ins() {
        let mut settings = Settings::default();
        settings.summary_prompts.push(custom("g", "Team sync"));
        let before = settings.clone();

        assert!(settings.set_summary_prompt_text("g", " \n ").is_err());
        let too_long = "é".repeat(PROMPT_TEXT_LIMIT + 1);
        assert!(settings.set_summary_prompt_text("g", &too_long).is_err());
        assert!(
            settings
                .set_summary_prompt_text(TOPIC_SUMMARY_ID, "x")
                .is_err()
        );
        assert!(settings.set_summary_prompt_text("missing", "x").is_err());
        assert_eq!(settings, before);

        // The limit counts characters, not bytes.
        let at_limit = "é".repeat(PROMPT_TEXT_LIMIT);
        settings.set_summary_prompt_text("g", &at_limit).unwrap();
        assert_eq!(settings.summary_prompts[0].text, at_limit);
    }

    #[test]
    fn deleting_the_active_prompt_makes_topic_summary_active() {
        // Covers AE5.
        let mut settings = Settings::default();
        settings.summary_prompts.push(custom("g", "Team sync"));
        settings.summary_prompts.push(custom("o", "One on one"));
        settings.activate_summary_prompt("g").unwrap();

        settings.delete_summary_prompt("o").unwrap();
        assert_eq!(settings.active_summary_prompt, "g");

        settings.delete_summary_prompt("g").unwrap();
        assert!(settings.summary_prompts.is_empty());
        assert_eq!(settings.active_summary_prompt, TOPIC_SUMMARY_ID);

        assert!(settings.delete_summary_prompt(PLAY_BY_PLAY_ID).is_err());
        assert!(settings.delete_summary_prompt("missing").is_err());
    }

    #[test]
    fn activate_accepts_any_known_prompt_and_refuses_an_unknown_id() {
        let mut settings = Settings::default();
        settings.summary_prompts.push(custom("g", "Team sync"));
        settings.activate_summary_prompt(PLAY_BY_PLAY_ID).unwrap();
        assert_eq!(settings.active_summary_prompt, PLAY_BY_PLAY_ID);
        settings.activate_summary_prompt("g").unwrap();
        assert_eq!(settings.active_summary_prompt, "g");
        assert!(settings.activate_summary_prompt("missing").is_err());
        assert_eq!(settings.active_summary_prompt, "g");
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
            }
        );

        settings.active_summary_prompt = PLAY_BY_PLAY_ID.into();
        assert_eq!(
            settings.active_summary_prompt_snapshot().id,
            PLAY_BY_PLAY_ID
        );
    }

    #[test]
    fn a_new_store_has_no_saved_base_url_and_uses_the_openai_default() {
        let (settings, _) = settings_from_value(&json!({}));
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
        let (settings, updated) = settings_from_value(&stored);
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
        let written = serde_json::to_value(&settings).unwrap();
        assert_eq!(settings_from_value(&written), (settings, false));
    }

    #[test]
    fn an_existing_store_keeps_an_empty_base_url() {
        // The OpenAI default applies only to a new install (R2).
        let stored = json!({
            "settings_schema_version": CURRENT_SETTINGS_SCHEMA_VERSION - 1,
            "endpoint_base_url": "",
            "endpoint_model": "gpt-4o",
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(settings.endpoint_base_url.as_deref(), Some(""));
        assert_eq!(settings.effective_endpoint_base_url(), "");
        assert_eq!(settings.endpoint_model, "gpt-4o");
        let written = serde_json::to_value(&settings).unwrap();
        assert_eq!(settings_from_value(&written), (settings, false));
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

    #[test]
    fn every_chosen_destination_is_retained_once() {
        let mut settings = Settings::default();
        for folder in ["/vault/a", "/vault/b", "/vault/a", "/vault/c"] {
            settings.choose_destination(folder);
        }
        assert_eq!(
            settings.destination_roots,
            ["/vault/a", "/vault/b", "/vault/c"]
        );
        assert_eq!(settings.current_destination.as_deref(), Some("/vault/c"));
    }
}

#[cfg(test)]
mod recognition_tests {
    use super::*;
    use models::recognition::{ProcessingMode, SpeakerSource};

    #[test]
    fn old_live_choice_does_not_enable_live_speaker_analysis() {
        for live in [true, false] {
            let (settings, changed) = settings_from_value(&serde_json::json!({
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
