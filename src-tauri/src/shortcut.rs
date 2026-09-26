//! The global shortcuts: the recording chord, which starts a session and
//! stops it (R1), and the optional chord that shows or hides the main
//! window (R5, R6).
//!
//! Registration follows Handy's `shortcut/tauri_impl.rs`, and a change
//! follows its `change_binding`: a press is handled on `Pressed` only, and a
//! refused chord leaves the previous chord registered. A press when the
//! recording shortcut is not armed does nothing — no dialog, no prompt —
//! because the surface already says what is missing (R1a). See
//! PROVENANCE.md.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::managers::capture::CaptureManager;
use crate::managers::settings::{self, Settings, SpeakerCountSetting};

/// Which of the two shortcuts a command acts on.
#[derive(Serialize, Deserialize, Type, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutKind {
    Recording,
    ShowWindow,
}

impl ShortcutKind {
    fn saved(self, settings: &Settings) -> Option<String> {
        match self {
            Self::Recording => settings.shortcut.clone(),
            Self::ShowWindow => settings.show_window_shortcut.clone(),
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Recording => Self::ShowWindow,
            Self::ShowWindow => Self::Recording,
        }
    }
}

/// One shortcut's state (KTD7): off when `chord` is `None`, registered, or
/// failed with a reason in `error`.
#[derive(Serialize, Type, Clone, Debug, Default, PartialEq)]
pub struct ShortcutStatus {
    pub chord: Option<String>,
    pub registered: bool,
    pub error: Option<String>,
}

impl ShortcutStatus {
    /// No chord is a choice, not a fault (R13).
    pub fn is_ok(&self) -> bool {
        self.chord.is_none() || self.registered
    }
}

#[derive(Serialize, Type, Clone, Debug)]
pub struct ShortcutStatuses {
    pub recording: ShortcutStatus,
    pub show_window: ShortcutStatus,
}

#[derive(Default)]
pub struct ShortcutRegistration {
    /// Held across every register, change, suspend and resume, so their
    /// plugin calls never interleave. Never taken on the main thread: the
    /// plugin sends each call to the main thread and waits for it.
    changes: Mutex<()>,
    recording: Mutex<ShortcutStatus>,
    show_window: Mutex<ShortcutStatus>,
}

impl ShortcutRegistration {
    fn slot(&self, kind: ShortcutKind) -> &Mutex<ShortcutStatus> {
        match kind {
            ShortcutKind::Recording => &self.recording,
            ShortcutKind::ShowWindow => &self.show_window,
        }
    }

    pub fn status(&self, kind: ShortcutKind) -> ShortcutStatus {
        self.slot(kind)
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn statuses(&self) -> ShortcutStatuses {
        ShortcutStatuses {
            recording: self.status(ShortcutKind::Recording),
            show_window: self.status(ShortcutKind::ShowWindow),
        }
    }

    fn set_status(&self, kind: ShortcutKind, status: ShortcutStatus) {
        *self.slot(kind).lock().unwrap_or_else(|e| e.into_inner()) = status;
    }
}

pub fn speaker_count(setting: Option<SpeakerCountSetting>) -> Option<session_core::SpeakerCount> {
    setting.map(|s| match s {
        SpeakerCountSetting::Exact { count } => session_core::SpeakerCount::Exact(count),
        SpeakerCountSetting::Range { min, max } => session_core::SpeakerCount::Range { min, max },
    })
}

/// The modifier names a chord may use, as the plugin parses them.
const MODIFIERS: &[&str] = &[
    "option",
    "alt",
    "control",
    "ctrl",
    "command",
    "cmd",
    "super",
    "shift",
    "commandorcontrol",
    "commandorctrl",
    "cmdorctrl",
    "cmdorcontrol",
];

/// Check that `raw` is a chord Longhand can register (R8): one or more
/// modifiers and one other key. The plugin refuses a modifier key or fn as
/// the main key, so a successful parse with a modifier is enough.
pub fn check_chord(raw: &str) -> Result<Shortcut, String> {
    let parts: Vec<String> = raw.split('+').map(|p| p.trim().to_lowercase()).collect();
    if parts.iter().all(|p| MODIFIERS.contains(&p.as_str())) {
        return Err("Add a key that is not a modifier, such as a letter.".into());
    }
    let shortcut: Shortcut = raw
        .parse()
        .map_err(|_| "Longhand can't use that key combination.".to_string())?;
    if shortcut.mods.is_empty() {
        return Err("Add a modifier key: Command, Control, Option or Shift.".into());
    }
    Ok(shortcut)
}

/// Whether two chords are the same keys, whatever names they use (R9).
pub fn same_chord(a: &str, b: &str) -> bool {
    match (a.parse::<Shortcut>(), b.parse::<Shortcut>()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The shown reason when the system refuses a chord (R10).
const NOT_AVAILABLE: &str = "Another app uses these keys. Choose different keys.";
const TAKEN_BY_OTHER: &str = "The other Longhand shortcut uses these keys.";

/// Change one shortcut from `old` to `new` (KTD4). The chord is checked
/// before anything is unregistered. When the new chord is refused, the old
/// chord is registered again. The caller saves only after `Ok`.
pub fn apply_change(
    old: Option<&str>,
    new: Option<&str>,
    other: Option<&str>,
    mut unregister: impl FnMut(&str),
    mut register: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    if let Some(new) = new {
        check_chord(new)?;
        if other.is_some_and(|other| same_chord(new, other)) {
            return Err(TAKEN_BY_OTHER.into());
        }
        if old.is_some_and(|old| same_chord(new, old)) {
            return Ok(());
        }
    }
    if let Some(old) = old {
        unregister(old);
    }
    let Some(new) = new else {
        return Ok(());
    };
    if let Err(e) = register(new) {
        log::warn!("shortcut {new} was refused: {e}");
        if let Some(old) = old
            && let Err(e) = register(old)
        {
            log::error!("failed to register {old} again: {e}");
        }
        return Err(NOT_AVAILABLE.into());
    }
    Ok(())
}

fn register_chord(app: &AppHandle, kind: ShortcutKind, chord: &str) -> Result<(), String> {
    let handler =
        move |app: &AppHandle, _: &Shortcut, event: tauri_plugin_global_shortcut::ShortcutEvent| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            match kind {
                ShortcutKind::Recording => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move { toggle(app).await });
                }
                ShortcutKind::ShowWindow => crate::window::toggle_main_window(app),
            }
        };
    app.global_shortcut()
        .on_shortcut(chord, handler)
        .map_err(|e| e.to_string())
}

fn unregister_chord(app: &AppHandle, chord: &str) {
    if let Err(e) = app.global_shortcut().unregister(chord) {
        log::warn!("failed to unregister shortcut {chord}: {e}");
    }
}

/// Register `kind`'s saved chord and record its status.
fn register_saved(
    app: &AppHandle,
    registration: &ShortcutRegistration,
    saved: &settings::Settings,
    kind: ShortcutKind,
) {
    let chord = kind.saved(saved);
    let status = match &chord {
        None => ShortcutStatus::default(),
        Some(chord) => match register_chord(app, kind, chord) {
            Ok(()) => {
                log::info!("shortcut {chord} registered");
                ShortcutStatus {
                    chord: Some(chord.clone()),
                    registered: true,
                    error: None,
                }
            }
            Err(e) => {
                log::error!("shortcut {chord} failed to register: {e}");
                ShortcutStatus {
                    chord: Some(chord.clone()),
                    registered: false,
                    error: Some(NOT_AVAILABLE.into()),
                }
            }
        },
    };
    registration.set_status(kind, status);
}

/// Register both saved chords at launch (R12).
pub fn register(app: &AppHandle) {
    let registration = ShortcutRegistration::default();
    let saved = settings::get_settings(app);
    register_saved(app, &registration, &saved, ShortcutKind::Recording);
    register_saved(app, &registration, &saved, ShortcutKind::ShowWindow);
    app.manage(registration);
}

/// Set, change or clear one shortcut (R1–R3, R5, R9, R10). Returns both
/// statuses on success, or the reason the chord was refused.
pub fn set_shortcut(
    app: &AppHandle,
    kind: ShortcutKind,
    chord: Option<String>,
) -> Result<ShortcutStatuses, String> {
    let registration = app.state::<ShortcutRegistration>();
    let _one_at_a_time = registration
        .changes
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Only a registered chord needs unregistering: during capture both are
    // suspended, and a failed chord was never registered.
    let status = registration.status(kind);
    let old = status.chord.filter(|_| status.registered);
    let other = kind.other().saved(&settings::get_settings(app));
    apply_change(
        old.as_deref(),
        chord.as_deref(),
        other.as_deref(),
        |c| unregister_chord(app, c),
        |c| register_chord(app, kind, c),
    )?;
    // Record the new registration before the save, so the status stays true
    // to the plugin when the save fails.
    registration.set_status(
        kind,
        ShortcutStatus {
            registered: chord.is_some(),
            chord: chord.clone(),
            error: None,
        },
    );
    settings::update_settings(app, |s| {
        match kind {
            ShortcutKind::Recording => s.shortcut = chord,
            ShortcutKind::ShowWindow => s.show_window_shortcut = chord,
        }
        Ok(())
    })?;
    Ok(registration.statuses())
}

/// Unregister both chords while the user presses keys for a new one, so the
/// keys start nothing (R11).
pub fn suspend(app: &AppHandle) {
    let registration = app.state::<ShortcutRegistration>();
    let _one_at_a_time = registration
        .changes
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for kind in [ShortcutKind::Recording, ShortcutKind::ShowWindow] {
        let status = registration.status(kind);
        if let (Some(chord), true) = (&status.chord, status.registered) {
            unregister_chord(app, chord);
            registration.set_status(
                kind,
                ShortcutStatus {
                    registered: false,
                    ..status
                },
            );
        }
    }
}

/// Register the saved chords again after capture ends (KTD6). A chord that
/// is already registered stays as it is, so this is safe to call more than
/// once.
pub fn resume(app: &AppHandle) {
    let registration = app.state::<ShortcutRegistration>();
    let _one_at_a_time = registration
        .changes
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let saved = settings::get_settings(app);
    for kind in [ShortcutKind::Recording, ShortcutKind::ShowWindow] {
        if !registration.status(kind).registered {
            register_saved(app, &registration, &saved, kind);
        }
    }
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
        match manager.start_session(&app, count) {
            // The main window comes forward and opens the new session's
            // page on the recording-changed event (KTD2).
            Ok(_) => crate::window::show_main_window(&app),
            Err(e) => log::error!("shortcut start failed: {e}"),
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modifier_and_a_key_is_a_chord() {
        assert!(check_chord("Super+Shift+KeyR").is_ok());
        assert!(check_chord("CommandOrControl+Alt+Shift+L").is_ok());
        assert!(check_chord("Control+Digit1").is_ok());
    }

    #[test]
    fn a_key_with_no_modifier_is_refused() {
        let reason = check_chord("KeyR").unwrap_err();
        assert!(reason.contains("modifier key"), "{reason}");
    }

    #[test]
    fn modifiers_with_no_key_are_refused() {
        let reason = check_chord("Shift+Alt").unwrap_err();
        assert!(reason.contains("not a modifier"), "{reason}");
    }

    #[test]
    fn fn_and_unparseable_chords_are_refused() {
        assert!(check_chord("Fn+KeyR").is_err());
        assert!(check_chord("Super+Shift+NotAKey").is_err());
        assert!(check_chord("").is_err());
    }

    /// Records the plugin calls `apply_change` makes, and refuses the chords
    /// in `refused`.
    #[derive(Default)]
    struct Plugin {
        calls: Vec<String>,
        refused: Vec<&'static str>,
    }

    fn change(
        plugin: &mut Plugin,
        old: Option<&str>,
        new: Option<&str>,
        other: Option<&str>,
    ) -> Result<(), String> {
        let refused = plugin.refused.clone();
        let calls = std::cell::RefCell::new(Vec::new());
        let result = apply_change(
            old,
            new,
            other,
            |c| calls.borrow_mut().push(format!("unregister {c}")),
            |c| {
                calls.borrow_mut().push(format!("register {c}"));
                if refused.contains(&c) {
                    Err("held".into())
                } else {
                    Ok(())
                }
            },
        );
        plugin.calls = calls.into_inner();
        result
    }

    #[test]
    fn a_change_unregisters_the_old_chord_and_registers_the_new() {
        let mut plugin = Plugin::default();
        let result = change(&mut plugin, Some("Super+KeyA"), Some("Super+KeyB"), None);
        assert_eq!(result, Ok(()));
        assert_eq!(
            plugin.calls,
            ["unregister Super+KeyA", "register Super+KeyB"]
        );
    }

    #[test]
    fn a_refused_chord_registers_the_old_chord_again() {
        let mut plugin = Plugin {
            refused: vec!["Super+KeyB"],
            ..Default::default()
        };
        let result = change(&mut plugin, Some("Super+KeyA"), Some("Super+KeyB"), None);
        assert_eq!(result, Err(NOT_AVAILABLE.to_string()));
        assert_eq!(
            plugin.calls,
            [
                "unregister Super+KeyA",
                "register Super+KeyB",
                "register Super+KeyA"
            ]
        );
    }

    #[test]
    fn the_other_shortcuts_chord_is_refused_before_anything_changes() {
        let mut plugin = Plugin::default();
        let result = change(
            &mut plugin,
            Some("Super+KeyA"),
            Some("Command+KeyL"),
            Some("Super+KeyL"),
        );
        assert_eq!(result, Err(TAKEN_BY_OTHER.to_string()));
        assert!(plugin.calls.is_empty());
    }

    #[test]
    fn an_invalid_chord_is_refused_before_anything_changes() {
        let mut plugin = Plugin::default();
        assert!(change(&mut plugin, Some("Super+KeyA"), Some("KeyB"), None).is_err());
        assert!(plugin.calls.is_empty());
    }

    #[test]
    fn a_clear_unregisters_the_old_chord() {
        let mut plugin = Plugin::default();
        assert_eq!(change(&mut plugin, Some("Super+KeyA"), None, None), Ok(()));
        assert_eq!(plugin.calls, ["unregister Super+KeyA"]);
    }

    #[test]
    fn a_clear_with_no_chord_changes_nothing() {
        let mut plugin = Plugin::default();
        assert_eq!(change(&mut plugin, None, None, None), Ok(()));
        assert!(plugin.calls.is_empty());
    }

    #[test]
    fn setting_the_same_keys_again_changes_nothing() {
        let mut plugin = Plugin::default();
        let result = change(&mut plugin, Some("Super+KeyA"), Some("Command+KeyA"), None);
        assert_eq!(result, Ok(()));
        assert!(plugin.calls.is_empty());
    }

    #[test]
    fn a_first_chord_is_registered_when_none_was_set() {
        let mut plugin = Plugin::default();
        assert_eq!(change(&mut plugin, None, Some("Super+KeyB"), None), Ok(()));
        assert_eq!(plugin.calls, ["register Super+KeyB"]);
    }

    #[test]
    fn two_names_for_the_same_keys_are_the_same_chord() {
        assert!(same_chord("Super+KeyR", "Command+KeyR"));
        assert!(same_chord("Shift+Alt+KeyL", "Option+Shift+L"));
        assert!(!same_chord("Super+KeyR", "Super+KeyT"));
        assert!(!same_chord("Super+KeyR", "not a chord"));
    }
}
