//! The global shortcut: one chord that starts a session and stops it (R1).
//!
//! Registration follows Handy's `shortcut/tauri_impl.rs`: the chord is
//! registered once at launch from settings, and a press is handled on
//! `Pressed` only. A press when the shortcut is not armed does nothing — no
//! dialog, no prompt — because the surface already says what is missing
//! (R1a). See PROVENANCE.md.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::managers::capture::CaptureManager;
use crate::managers::settings::{self, SpeakerCountSetting};

#[derive(Default)]
pub struct ShortcutRegistration {
    chord: Mutex<Option<String>>,
    error: Mutex<Option<String>>,
}

#[derive(Serialize, Type, Clone, Debug)]
pub struct ShortcutStatus {
    pub chord: String,
    pub registered: bool,
    pub error: Option<String>,
}

impl ShortcutRegistration {
    pub fn status(&self) -> ShortcutStatus {
        let chord = self.chord.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let error = self.error.lock().unwrap_or_else(|e| e.into_inner()).clone();
        ShortcutStatus {
            registered: chord.is_some() && error.is_none(),
            chord: chord.unwrap_or_default(),
            error,
        }
    }
}

pub fn speaker_count(setting: Option<SpeakerCountSetting>) -> Option<session_core::SpeakerCount> {
    setting.map(|s| match s {
        SpeakerCountSetting::Exact { count } => session_core::SpeakerCount::Exact(count),
        SpeakerCountSetting::Range { min, max } => session_core::SpeakerCount::Range { min, max },
    })
}

pub fn register(app: &AppHandle) {
    let registration = ShortcutRegistration::default();
    let chord = settings::get_settings(app).shortcut;
    let result = app
        .global_shortcut()
        .on_shortcut(chord.as_str(), |app, _shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move { toggle(app).await });
        })
        .map_err(|e| e.to_string());
    match &result {
        Ok(()) => log::info!("shortcut {chord} registered"),
        Err(e) => log::error!("shortcut {chord} failed to register: {e}"),
    }
    *registration.chord.lock().unwrap_or_else(|e| e.into_inner()) = Some(chord);
    *registration.error.lock().unwrap_or_else(|e| e.into_inner()) = result.err();
    app.manage(registration);
}

/// Stop a recording session, or start one when armed (R1a, R1c): the
/// microphone is authorized and a destination is chosen.
async fn toggle(app: AppHandle) {
    // One press at a time: a quick double press must not race two starts.
    static TOGGLE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one_at_a_time = TOGGLE.lock().await;
    let manager = Arc::clone(&app.state::<Arc<CaptureManager>>());
    if manager.is_capturing() {
        let app = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || match manager.stop_session(&app) {
            Ok((id, state)) => {
                log::info!("shortcut stopped {id}; now {}", state.name());
                app.state::<Arc<crate::managers::processing::ProcessingManager>>()
                    .wake();
            }
            Err(e) => log::error!("shortcut stop failed: {e}"),
        })
        .await;
        return;
    }
    let current = settings::get_settings(&app);
    let microphone = tauri_plugin_macos_permissions::check_microphone_permission().await;
    if !microphone || current.current_destination.is_none() {
        log::info!(
            "shortcut pressed while not armed (microphone {}, destination {})",
            if microphone { "ok" } else { "missing" },
            if current.current_destination.is_some() {
                "ok"
            } else {
                "missing"
            }
        );
        return;
    }
    let count = speaker_count(current.default_speaker_count);
    let _ = tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = manager.start_session(&app, count) {
            log::error!("shortcut start failed: {e}");
        }
    })
    .await;
}
