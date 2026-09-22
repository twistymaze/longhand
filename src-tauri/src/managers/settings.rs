//! Settings in the application's own configuration store (R33, KTD15).
//!
//! Shape follows Handy's `settings.rs` (one `#[serde(default)]` struct, a
//! schema-version integer, migrations keyed on the stored version, salvage of
//! individually valid fields); see src-tauri/PROVENANCE.md. Portable mode is
//! omitted. The struct carries a single probe field in Phase 2a; U4 replaces
//! it with the real schema through a version migration.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

/// Relative, so `tauri-plugin-store` resolves it under the app data directory.
pub const SETTINGS_STORE_PATH: &str = "settings.json";
const SETTINGS_KEY: &str = "settings";

pub const CURRENT_SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Type, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub settings_schema_version: u32,
    /// Phase 2a only: a value the checkpoint writes and reads back across a
    /// relaunch and a rebuild.
    pub probe_value: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            settings_schema_version: CURRENT_SETTINGS_SCHEMA_VERSION,
            probe_value: String::new(),
        }
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
        // Nothing to transform yet; stamp the version so later migrations
        // can key on it.
        settings.settings_schema_version = 1;
        updated = true;
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

/// Write and flush immediately: the store's autosave is debounced, and a
/// setting changed just before quitting must survive the quit.
pub fn write_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
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

        let saved = serde_json::to_value(&settings).unwrap();
        assert_eq!(settings_from_value(&saved), (settings, false));
    }

    #[test]
    fn present_fields_survive_a_version_migration() {
        let (settings, updated) = settings_from_value(&json!({ "probe_value": "kept" }));
        assert!(updated);
        assert_eq!(settings.probe_value, "kept");
        assert_eq!(settings.settings_schema_version, 1);
    }

    #[test]
    fn unknown_field_loads_and_is_dropped_on_save() {
        let stored = json!({
            "settings_schema_version": 1,
            "probe_value": "x",
            "from_the_future": true,
        });
        let (settings, updated) = settings_from_value(&stored);
        assert!(!updated);
        assert_eq!(settings.probe_value, "x");
        let saved = serde_json::to_value(&settings).unwrap();
        assert!(saved.get("from_the_future").is_none());
    }

    #[test]
    fn one_invalid_field_keeps_the_rest() {
        let stored = json!({ "settings_schema_version": "one", "probe_value": "kept" });
        let (settings, updated) = settings_from_value(&stored);
        assert!(updated);
        assert_eq!(settings.probe_value, "kept");
        assert_eq!(
            settings.settings_schema_version,
            CURRENT_SETTINGS_SCHEMA_VERSION
        );
    }

    #[test]
    fn non_object_store_falls_back_to_defaults() {
        let (settings, updated) = settings_from_value(&json!("garbage"));
        assert!(updated);
        assert_eq!(settings, Settings::default());
    }
}
