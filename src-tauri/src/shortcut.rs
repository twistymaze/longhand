//! The global shortcuts: the recording chord, which starts a note or
//! records into the shown note, and stops the recording (R1, R10), and the
//! optional chord that shows or hides the main window (R5, R6), and the
//! optional chord that pauses and continues the open note (R16), and the
//! optional chord that shows or hides the recording widget (R13).
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
use crate::managers::processing::ProcessingManager;
use crate::managers::settings::{self, Settings, SpeakerCountSetting};

/// Which of the four shortcuts a command acts on.
#[derive(Serialize, Deserialize, Type, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutKind {
    Recording,
    ShowWindow,
    Pause,
    Widget,
}

impl ShortcutKind {
    const ALL: [Self; 4] = [Self::Recording, Self::ShowWindow, Self::Pause, Self::Widget];

    fn saved(self, settings: &Settings) -> Option<String> {
        match self {
            Self::Recording => settings.shortcut.clone(),
            Self::ShowWindow => settings.show_window_shortcut.clone(),
            Self::Pause => settings.pause_shortcut.clone(),
            Self::Widget => settings.widget_shortcut.clone(),
        }
    }

    /// The kinds whose chords a new chord of this kind must not use (KTD10).
    fn others(self) -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(move |kind| *kind != self)
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
    pub pause: ShortcutStatus,
    pub widget: ShortcutStatus,
}

#[derive(Default)]
pub struct ShortcutRegistration {
    /// Held across every register, change, suspend and resume, so their
    /// plugin calls never interleave. Never taken on the main thread: the
    /// plugin sends each call to the main thread and waits for it.
    changes: Mutex<()>,
    recording: Mutex<ShortcutStatus>,
    show_window: Mutex<ShortcutStatus>,
    pause: Mutex<ShortcutStatus>,
    widget: Mutex<ShortcutStatus>,
}

impl ShortcutRegistration {
    fn slot(&self, kind: ShortcutKind) -> &Mutex<ShortcutStatus> {
        match kind {
            ShortcutKind::Recording => &self.recording,
            ShortcutKind::ShowWindow => &self.show_window,
            ShortcutKind::Pause => &self.pause,
            ShortcutKind::Widget => &self.widget,
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
            pause: self.status(ShortcutKind::Pause),
            widget: self.status(ShortcutKind::Widget),
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
const TAKEN_BY_OTHER: &str = "Another Longhand shortcut uses these keys.";

/// Change one shortcut from `old` to `new` (KTD4). The chord is checked
/// before anything is unregistered, also against the chords of the other
/// shortcuts in `others` (KTD10). When the new chord is refused, the old
/// chord is registered again. The caller saves only after `Ok`.
pub fn apply_change(
    old: Option<&str>,
    new: Option<&str>,
    others: &[&str],
    mut unregister: impl FnMut(&str),
    mut register: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    if let Some(new) = new {
        check_chord(new)?;
        if others.iter().any(|other| same_chord(new, other)) {
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
    let handler = move |app: &AppHandle,
                        _: &Shortcut,
                        event: tauri_plugin_global_shortcut::ShortcutEvent| {
        if event.state() != ShortcutState::Pressed {
            return;
        }
        match kind {
            ShortcutKind::Recording => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { toggle(app).await });
            }
            ShortcutKind::ShowWindow => crate::window::toggle_main_window(app),
            ShortcutKind::Pause => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { pause_or_continue(app).await });
            }
            // With no open note the press does nothing (R15).
            ShortcutKind::Widget => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { crate::widget::toggle_open_note(&app) });
            }
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

/// Register all saved chords at launch (R12).
pub fn register(app: &AppHandle) {
    let registration = ShortcutRegistration::default();
    let saved = settings::get_settings(app);
    for kind in ShortcutKind::ALL {
        register_saved(app, &registration, &saved, kind);
    }
    app.manage(registration);
}

/// Set, change or clear one shortcut (R1–R3, R5, R9, R10, R16). Returns all
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
    // Only a registered chord needs unregistering: during capture all are
    // suspended, and a failed chord was never registered.
    let status = registration.status(kind);
    let old = status.chord.filter(|_| status.registered);
    let saved = settings::get_settings(app);
    let others: Vec<String> = kind.others().filter_map(|k| k.saved(&saved)).collect();
    let others: Vec<&str> = others.iter().map(String::as_str).collect();
    apply_change(
        old.as_deref(),
        chord.as_deref(),
        &others,
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
            ShortcutKind::Pause => s.pause_shortcut = chord,
            ShortcutKind::Widget => s.widget_shortcut = chord,
        }
        Ok(())
    })?;
    Ok(registration.statuses())
}

/// Unregister all chords while the user presses keys for a new one, so the
/// keys start nothing (R11).
pub fn suspend(app: &AppHandle) {
    let registration = app.state::<ShortcutRegistration>();
    let _one_at_a_time = registration
        .changes
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    suspend_registered(&registration, |chord| unregister_chord(app, chord));
}

/// Unregister each registered chord and record it as not registered. The
/// chord stays in the status, so a resume registers it again.
fn suspend_registered(registration: &ShortcutRegistration, mut unregister: impl FnMut(&str)) {
    for kind in ShortcutKind::ALL {
        let status = registration.status(kind);
        if let (Some(chord), true) = (&status.chord, status.registered) {
            unregister(chord);
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
    for kind in ShortcutKind::ALL {
        if !registration.status(kind).registered {
            register_saved(app, &registration, &saved, kind);
        }
    }
}

/// What one press of the recording shortcut reads (R10, KTD7).
#[derive(Debug, Clone, Copy)]
pub struct Press<'a> {
    /// A note is open: it records or is paused (R17).
    pub capturing: bool,
    /// The storage folder is available, so a note can start (R7).
    pub folder_available: bool,
    /// The setting: New Note starts a note that records (R8).
    pub new_note_records: bool,
    /// The note that the main window shows, while the window is visible
    /// and not minimized.
    pub shown: Option<&'a str>,
}

/// What one press of the recording shortcut does (R10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutAction {
    /// Stop the open note, recording or paused (R17).
    Stop,
    /// Start nothing: the storage folder is not available (AE8).
    Nothing,
    /// Start a new note that records.
    NewRecordingNote,
    /// Record into the shown note (AE1, AE9).
    RecordInto(String),
    /// Open a new note that does not record.
    NewNote,
}

/// The decision of the recording shortcut (R10, the "recording shortcut"
/// flowchart of the plan). `can_record` tells whether the Record button of
/// a note is available (R11, R12); it is read only for the shown note when
/// New Note does not record.
pub fn press_action(press: &Press, can_record: impl FnOnce(&str) -> bool) -> ShortcutAction {
    if press.capturing {
        return ShortcutAction::Stop;
    }
    if !press.folder_available {
        return ShortcutAction::Nothing;
    }
    if press.new_note_records {
        return ShortcutAction::NewRecordingNote;
    }
    match press.shown {
        Some(id) if can_record(id) => ShortcutAction::RecordInto(id.to_string()),
        _ => ShortcutAction::NewNote,
    }
}

/// The reported shown note, while the main window is visible and not
/// minimized (KTD7). Otherwise no note counts as shown.
pub fn shown_while_visible(
    shown: Option<String>,
    visible: bool,
    minimized: bool,
) -> Option<String> {
    shown.filter(|_| visible && !minimized)
}

/// The note that the main window shows now, from the frontend's report and
/// the state of the window (KTD7).
fn shown_note(app: &AppHandle) -> Option<String> {
    let shown = app.state::<crate::commands::session::ShownNote>().get();
    let window = app.get_webview_window(crate::window::MAIN_WINDOW)?;
    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(true);
    shown_while_visible(shown, visible, minimized)
}

/// The press decision with the state of the app. It reads the storage
/// folder and the store: call it off the main thread.
fn decide(app: &AppHandle, manager: &CaptureManager) -> ShortcutAction {
    let current = settings::get_settings(app);
    let folder = crate::managers::destination::status(&current.storage_folder);
    let shown = shown_note(app);
    let press = Press {
        capturing: manager.is_capturing(),
        folder_available: crate::managers::destination::note_start_refusal(&folder).is_none(),
        new_note_records: current.new_note_records,
        shown: shown.as_deref(),
    };
    press_action(&press, |id| {
        let processing = app.state::<Arc<ProcessingManager>>();
        let checked = crate::commands::session::check_record(
            &manager.shared_store(),
            processing.library_status(),
            processing.own_runs(),
            id,
            chrono::Local::now().fixed_offset(),
        );
        if let Err(reason) = &checked {
            log::info!("the shortcut does not record into the shown note {id}: {reason}");
        }
        checked.is_ok()
    })
}

/// Leave the reported shown note before a new note starts (R18, AE15). The
/// note goes when it is empty. A failure keeps the note and does not stop
/// the start.
fn leave_shown(app: &AppHandle, manager: &CaptureManager) {
    let Some(id) = app.state::<crate::commands::session::ShownNote>().get() else {
        return;
    };
    let processing = app.state::<Arc<ProcessingManager>>();
    if let Err(e) = crate::commands::session::leave_note_now(manager, &processing, &id) {
        log::warn!("the shortcut could not leave the note {id}: {e}");
    }
}

/// The type of a note that a shortcut starts: the default type (R23). When
/// the type list cannot be read, the note is in General.
fn default_type(app: &AppHandle) -> Option<String> {
    crate::commands::capture::type_of_new_note(app, None).unwrap_or_else(|e| {
        log::warn!("the shortcut could not read the default type, so the note is in General: {e}");
        None
    })
}

/// Do one press of the recording shortcut (R1a, R1c, R10). A recording
/// start needs an authorized microphone; without it, a press does nothing.
async fn toggle(app: AppHandle) {
    // One press at a time: a quick double press must not race two starts.
    static TOGGLE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one_at_a_time = TOGGLE.lock().await;
    let manager = Arc::clone(&app.state::<Arc<CaptureManager>>());
    let action = {
        let app = app.clone();
        let manager = Arc::clone(&manager);
        tauri::async_runtime::spawn_blocking(move || decide(&app, &manager)).await
    };
    let action = match action {
        Ok(action) => action,
        Err(e) => {
            log::error!("the shortcut could not decide: {e}");
            return;
        }
    };
    if matches!(
        action,
        ShortcutAction::NewRecordingNote | ShortcutAction::RecordInto(_)
    ) && !tauri_plugin_macos_permissions::check_microphone_permission().await
    {
        log::info!("shortcut pressed while not armed (microphone missing)");
        return;
    }
    let count = speaker_count(settings::get_settings(&app).default_speaker_count);
    let _ = tauri::async_runtime::spawn_blocking(move || match action {
        ShortcutAction::Stop => match manager.stop_session(&app) {
            Ok(outcome) => {
                log::info!("shortcut stopped {}: {outcome:?}", outcome.session_id());
                app.state::<Arc<ProcessingManager>>().wake();
            }
            Err(e) => log::error!("shortcut stop failed: {e}"),
        },
        ShortcutAction::Nothing => {
            log::info!("shortcut pressed while the storage folder is not available");
        }
        ShortcutAction::NewRecordingNote => {
            leave_shown(&app, &manager);
            match manager.start_session(&app, count, default_type(&app).as_deref()) {
                // The main window comes forward and opens the new note's
                // page on the recording-changed event (KTD2).
                Ok(_) => crate::window::show_main_window(&app),
                Err(e) => log::error!("shortcut start failed: {e}"),
            }
        }
        ShortcutAction::RecordInto(id) => match manager.record_session(&app, &id) {
            Ok(()) => crate::window::show_main_window(&app),
            Err(e) => log::error!("shortcut record into {id} failed: {e}"),
        },
        ShortcutAction::NewNote => {
            leave_shown(&app, &manager);
            match manager.open_note(&app, default_type(&app).as_deref()) {
                Ok(session_id) => {
                    crate::window::show_main_window(&app);
                    let _ =
                        tauri_specta::Event::emit(&crate::tray::ShowSession { session_id }, &app);
                }
                Err(e) => log::error!("shortcut new note failed: {e}"),
            }
        }
    })
    .await;
}

/// What one press of the Pause shortcut does (R16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseAction {
    /// Pause the recording note.
    Pause,
    /// Continue the paused note.
    Continue,
    /// No note is open.
    Nothing,
}

/// The decision of the Pause shortcut from the capture state: `open` when a
/// note is open (recording, reopening the microphone, or paused), and
/// `paused` when the user paused it. The press sends a target state, so a
/// press that races a button click cannot invert the state (KTD8).
pub fn pause_press_action(open: bool, paused: bool) -> PauseAction {
    match (open, paused) {
        (_, true) => PauseAction::Continue,
        (true, false) => PauseAction::Pause,
        (false, false) => PauseAction::Nothing,
    }
}

/// Do one press of the Pause shortcut (R16). It reads the capture state and
/// pauses or continues the open note. With no open note, it does nothing.
async fn pause_or_continue(app: AppHandle) {
    let manager = Arc::clone(&app.state::<Arc<CaptureManager>>());
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let status = match manager.status(&app) {
            Ok(status) => status,
            Err(e) => {
                log::error!("the Pause shortcut could not read the capture state: {e}");
                return;
            }
        };
        match pause_press_action(status.recording, status.paused) {
            PauseAction::Pause => {
                if let Err(e) = manager.pause_session(&app) {
                    log::error!("shortcut pause failed: {e}");
                }
            }
            PauseAction::Continue => {
                if let Err(e) = manager.continue_session(&app) {
                    log::error!("shortcut continue failed: {e}");
                }
            }
            PauseAction::Nothing => log::info!("the Pause shortcut was pressed with no open note"),
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
        others: &[&str],
    ) -> Result<(), String> {
        let refused = plugin.refused.clone();
        let calls = std::cell::RefCell::new(Vec::new());
        let result = apply_change(
            old,
            new,
            others,
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
        let result = change(&mut plugin, Some("Super+KeyA"), Some("Super+KeyB"), &[]);
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
        let result = change(&mut plugin, Some("Super+KeyA"), Some("Super+KeyB"), &[]);
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
            &["Super+KeyL"],
        );
        assert_eq!(result, Err(TAKEN_BY_OTHER.to_string()));
        assert!(plugin.calls.is_empty());
    }

    // A Pause chord that the Recording or the Show Longhand shortcut uses.
    #[test]
    fn a_chord_that_any_other_shortcut_uses_is_refused() {
        let recording = "Super+Shift+KeyR";
        let show_window = "Super+Shift+KeyL";
        for taken in ["Command+Shift+KeyR", "Command+Shift+KeyL"] {
            let mut plugin = Plugin::default();
            let result = change(&mut plugin, None, Some(taken), &[recording, show_window]);
            assert_eq!(
                result,
                Err("Another Longhand shortcut uses these keys.".to_string())
            );
            assert!(plugin.calls.is_empty());
        }
    }

    #[test]
    fn the_saved_chord_of_each_kind_is_its_setting() {
        let settings = Settings {
            shortcut: Some("Super+KeyR".into()),
            show_window_shortcut: Some("Super+KeyL".into()),
            pause_shortcut: Some("Super+KeyP".into()),
            widget_shortcut: Some("Super+KeyW".into()),
            ..Settings::default()
        };
        let saved: Vec<_> = ShortcutKind::ALL
            .iter()
            .map(|kind| kind.saved(&settings))
            .collect();
        assert_eq!(
            saved,
            [
                Some("Super+KeyR".to_string()),
                Some("Super+KeyL".to_string()),
                Some("Super+KeyP".to_string()),
                Some("Super+KeyW".to_string())
            ]
        );
    }

    #[test]
    fn the_other_kinds_are_all_kinds_but_this_one() {
        assert_eq!(
            ShortcutKind::Pause.others().collect::<Vec<_>>(),
            [
                ShortcutKind::Recording,
                ShortcutKind::ShowWindow,
                ShortcutKind::Widget
            ]
        );
        assert_eq!(
            ShortcutKind::Widget.others().collect::<Vec<_>>(),
            [
                ShortcutKind::Recording,
                ShortcutKind::ShowWindow,
                ShortcutKind::Pause
            ]
        );
    }

    #[test]
    fn suspending_unregisters_all_four_registered_chords() {
        let registration = ShortcutRegistration::default();
        for (kind, chord) in [
            (ShortcutKind::Recording, "Super+KeyR"),
            (ShortcutKind::ShowWindow, "Super+KeyL"),
            (ShortcutKind::Pause, "Super+KeyP"),
            (ShortcutKind::Widget, "Super+KeyW"),
        ] {
            registration.set_status(
                kind,
                ShortcutStatus {
                    chord: Some(chord.into()),
                    registered: true,
                    error: None,
                },
            );
        }
        let mut unregistered = Vec::new();
        suspend_registered(&registration, |c| unregistered.push(c.to_string()));
        assert_eq!(
            unregistered,
            ["Super+KeyR", "Super+KeyL", "Super+KeyP", "Super+KeyW"]
        );
        let statuses = registration.statuses();
        for status in [
            statuses.recording,
            statuses.show_window,
            statuses.pause,
            statuses.widget,
        ] {
            assert!(!status.registered);
            assert!(status.chord.is_some());
        }
    }

    // Covers AE5: a Pause press on a paused note continues it.
    #[test]
    fn a_pause_press_on_a_paused_note_continues_it() {
        assert_eq!(pause_press_action(true, true), PauseAction::Continue);
    }

    #[test]
    fn a_pause_press_on_a_recording_note_pauses_it() {
        assert_eq!(pause_press_action(true, false), PauseAction::Pause);
    }

    #[test]
    fn a_pause_press_with_no_open_note_does_nothing() {
        assert_eq!(pause_press_action(false, false), PauseAction::Nothing);
    }

    // Covers AE5, R17: `capturing` comes from `is_capturing`, which counts a
    // paused note, so the Recording shortcut stops a paused note.
    #[test]
    fn a_recording_press_on_a_paused_note_stops_it() {
        let action = press_action(
            &Press {
                capturing: true,
                ..press()
            },
            |_| panic!("a stop reads no note"),
        );
        assert_eq!(action, ShortcutAction::Stop);
    }

    #[test]
    fn an_invalid_chord_is_refused_before_anything_changes() {
        let mut plugin = Plugin::default();
        assert!(change(&mut plugin, Some("Super+KeyA"), Some("KeyB"), &[]).is_err());
        assert!(plugin.calls.is_empty());
    }

    #[test]
    fn a_clear_unregisters_the_old_chord() {
        let mut plugin = Plugin::default();
        assert_eq!(change(&mut plugin, Some("Super+KeyA"), None, &[]), Ok(()));
        assert_eq!(plugin.calls, ["unregister Super+KeyA"]);
    }

    #[test]
    fn a_clear_with_no_chord_changes_nothing() {
        let mut plugin = Plugin::default();
        assert_eq!(change(&mut plugin, None, None, &[]), Ok(()));
        assert!(plugin.calls.is_empty());
    }

    #[test]
    fn setting_the_same_keys_again_changes_nothing() {
        let mut plugin = Plugin::default();
        let result = change(&mut plugin, Some("Super+KeyA"), Some("Command+KeyA"), &[]);
        assert_eq!(result, Ok(()));
        assert!(plugin.calls.is_empty());
    }

    #[test]
    fn a_first_chord_is_registered_when_none_was_set() {
        let mut plugin = Plugin::default();
        assert_eq!(change(&mut plugin, None, Some("Super+KeyB"), &[]), Ok(()));
        assert_eq!(plugin.calls, ["register Super+KeyB"]);
    }

    /// A press with no recording, an available storage folder, the default
    /// "does not record" and the note "n" on the main window.
    fn press() -> Press<'static> {
        Press {
            capturing: false,
            folder_available: true,
            new_note_records: false,
            shown: Some("n"),
        }
    }

    // Covers AE1, AE9: the window shows "n" with an available Record button.
    #[test]
    fn the_shortcut_records_into_the_shown_note_when_new_note_does_not_record() {
        let mut asked = Vec::new();
        let action = press_action(&press(), |id| {
            asked.push(id.to_string());
            true
        });
        assert_eq!(action, ShortcutAction::RecordInto("n".into()));
        assert_eq!(asked, ["n"]);
    }

    // Covers AE1: the second press stops the recording.
    #[test]
    fn a_press_during_a_recording_stops_it() {
        let action = press_action(
            &Press {
                capturing: true,
                ..press()
            },
            |_| panic!("a stop reads no note"),
        );
        assert_eq!(action, ShortcutAction::Stop);
    }

    // Covers AE2.
    #[test]
    fn the_shortcut_starts_a_new_recording_note_when_new_note_records() {
        let action = press_action(
            &Press {
                new_note_records: true,
                ..press()
            },
            |_| panic!("the shown note does not count"),
        );
        assert_eq!(action, ShortcutAction::NewRecordingNote);
    }

    // Covers AE10: Record is not available, for example because the note
    // records on another computer.
    #[test]
    fn the_shortcut_opens_a_new_note_when_the_shown_note_cannot_record() {
        assert_eq!(press_action(&press(), |_| false), ShortcutAction::NewNote);
    }

    #[test]
    fn the_shortcut_opens_a_new_note_when_no_note_is_shown() {
        let action = press_action(
            &Press {
                shown: None,
                ..press()
            },
            |_| panic!("no note to read"),
        );
        assert_eq!(action, ShortcutAction::NewNote);
    }

    // Covers AE8.
    #[test]
    fn the_shortcut_starts_nothing_while_the_storage_folder_is_unavailable() {
        for new_note_records in [true, false] {
            let action = press_action(
                &Press {
                    folder_available: false,
                    new_note_records,
                    ..press()
                },
                |_| true,
            );
            assert_eq!(action, ShortcutAction::Nothing);
        }
    }

    // The window is minimized or hidden: the shortcut does the default.
    #[test]
    fn a_note_counts_as_shown_only_while_the_window_is_visible_and_not_minimized() {
        let id = || Some("n".to_string());
        assert_eq!(shown_while_visible(id(), true, false), id());
        assert_eq!(shown_while_visible(id(), true, true), None);
        assert_eq!(shown_while_visible(id(), false, false), None);
        assert_eq!(shown_while_visible(None, true, false), None);
    }

    #[test]
    fn two_names_for_the_same_keys_are_the_same_chord() {
        assert!(same_chord("Super+KeyR", "Command+KeyR"));
        assert!(same_chord("Shift+Alt+KeyL", "Option+Shift+L"));
        assert!(!same_chord("Super+KeyR", "Super+KeyT"));
        assert!(!same_chord("Super+KeyR", "not a chord"));
    }
}
