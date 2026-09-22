//! Phase 2a permission probe. Temporary: it proves the declared global-shortcut
//! permission works on a real machine before anything depends on it, and is
//! removed when U7 lands the real shortcut. The microphone half of the probe
//! lives in the frontend, through `tauri-plugin-macos-permissions`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_specta::Event;

/// Deliberately obscure so it collides with nothing the user already has bound.
pub const PROBE_SHORTCUT: &str = "CommandOrControl+Alt+Shift+L";

#[derive(Default)]
struct ProbeState {
    fired: AtomicU32,
    registration_error: Mutex<Option<String>>,
}

#[derive(Serialize, Clone, Type, Event)]
pub struct ProbeShortcutFired {
    pub count: u32,
}

#[derive(Serialize, Type)]
pub struct ProbeStatus {
    pub shortcut: String,
    pub registered: bool,
    pub registration_error: Option<String>,
    pub fired: u32,
}

pub fn register(app: &AppHandle) {
    app.manage(ProbeState::default());
    let result = app
        .global_shortcut()
        .on_shortcut(PROBE_SHORTCUT, |app, _shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let count = app
                .state::<ProbeState>()
                .fired
                .fetch_add(1, Ordering::SeqCst)
                + 1;
            log::info!("probe shortcut fired ({count})");
            if let Err(e) = (ProbeShortcutFired { count }).emit(app) {
                log::error!("failed to emit probe event: {e}");
            }
        })
        .map_err(|e| e.to_string());
    if let Err(e) = &result {
        log::error!("probe shortcut registration failed: {e}");
    }
    *app.state::<ProbeState>().registration_error.lock().unwrap() = result.err();
}

#[tauri::command]
#[specta::specta]
pub fn probe_status(app: AppHandle) -> ProbeStatus {
    let state = app.state::<ProbeState>();
    let registration_error = state.registration_error.lock().unwrap().clone();
    ProbeStatus {
        shortcut: PROBE_SHORTCUT.to_string(),
        registered: registration_error.is_none(),
        registration_error,
        fired: state.fired.load(Ordering::SeqCst),
    }
}
