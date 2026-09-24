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
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

/// Relative, so `tauri-plugin-store` resolves it under the app data directory.
pub const SETTINGS_STORE_PATH: &str = "settings.json";
const SETTINGS_KEY: &str = "settings";

pub const CURRENT_SETTINGS_SCHEMA_VERSION: u32 = 2;

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

#[derive(Serialize, Deserialize, Type, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub settings_schema_version: u32,
    pub shortcut: String,
    /// Every folder the user has ever chosen, oldest first (R25a). The index
    /// is rebuilt by scanning these, so a root is never dropped silently.
    pub destination_roots: Vec<String>,
    /// Where the next session is saved (R23). Always one of the roots.
    pub current_destination: Option<String>,
    /// OpenAI-compatible base URL, e.g. `https://api.openai.com/v1` (R32).
    pub endpoint_base_url: String,
    pub endpoint_model: String,
    /// Overrides the probed context budget when set (KTD16).
    pub context_limit_tokens: Option<u32>,
    pub default_speaker_count: Option<SpeakerCountSetting>,
    /// Catalog id of the recognition model (KTD6).
    pub asr_model: String,
    /// Context limits endpoints have shown by rejecting longer prompts,
    /// keyed by [`learned_limit_key`] (KTD16).
    pub learned_context_limits: std::collections::BTreeMap<String, u32>,
    /// The user heard the alert cue played at setup (R5a, KTD14).
    pub alert_tone_heard: bool,
}

/// One endpoint and model: a limit learned for one says nothing of another.
pub fn learned_limit_key(base_url: &str, model: &str) -> String {
    format!("{} {}", base_url.trim(), model.trim())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            settings_schema_version: CURRENT_SETTINGS_SCHEMA_VERSION,
            shortcut: DEFAULT_SHORTCUT.to_string(),
            destination_roots: Vec::new(),
            current_destination: None,
            endpoint_base_url: String::new(),
            endpoint_model: String::new(),
            context_limit_tokens: None,
            default_speaker_count: None,
            asr_model: models::DEFAULT_ASR_MODEL.to_string(),
            learned_context_limits: Default::default(),
            alert_tone_heard: false,
        }
    }
}

impl Settings {
    /// Make `folder` the current destination and retain it as a root (R23, R25a).
    pub fn choose_destination(&mut self, folder: &str) {
        if !self.destination_roots.iter().any(|r| r == folder) {
            self.destination_roots.push(folder.to_string());
        }
        self.current_destination = Some(folder.to_string());
    }

    pub fn endpoint_configured(&self) -> bool {
        !self.endpoint_base_url.trim().is_empty() && !self.endpoint_model.trim().is_empty()
    }
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
    if updated {
        settings.settings_schema_version = CURRENT_SETTINGS_SCHEMA_VERSION;
    }
    updated
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
    store.set(
        SETTINGS_KEY,
        serde_json::to_value(settings).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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
        assert_eq!(settings, Settings::default());
        let written = serde_json::to_value(&settings).unwrap();
        assert!(written.get("probe_value").is_none());
        assert_eq!(written["settings_schema_version"], 2);
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
