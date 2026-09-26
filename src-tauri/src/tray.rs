//! Tray icon and menu. The builder shape and the Windows click convention are
//! Handy's (`lib.rs` tray setup); see src-tauri/PROVENANCE.md.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use specta::Type;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;

use crate::managers::capture::CaptureManager;
use crate::window::{open_settings, show_main_window};

const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");
const OPEN: &str = "Open Longhand";
const ALERT_TITLE: &str = "Interrupted";

/// The menu item that fronts the window, renamed while an alert is raised.
struct OpenItem(MenuItem<tauri::Wry>);

/// Show a failed-capture alert on the tray, or clear it (U15): a title
/// beside the menu-bar icon on macOS, which takes precedence over the
/// recording time, the tooltip, and the menu's first item, which fronts the
/// window on the Resume and Stop controls.
pub fn set_alert(app: &AppHandle, headline: Option<&str>) {
    update_title(app, |title| title.set_alert(headline.is_some()));
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(match headline {
            Some(headline) => format!("Longhand: {headline}"),
            None => "Longhand".to_string(),
        }));
    }
    if let Some(item) = app.try_state::<OpenItem>() {
        let text = match headline {
            Some(_) => "Recording interrupted: Open Longhand",
            None => OPEN,
        };
        let _ = item.0.set_text(text);
    }
}

/// A session started, stopped, or resumed recording. The title shows the
/// recording time from the next tick, and is empty after a stop.
pub fn recording_changed(app: &AppHandle, session_id: Option<&str>) {
    update_title(app, |title| title.recording_changed(session_id));
}

/// A capture tick: the title shows the new recorded time once a second.
pub fn recording_tick(app: &AppHandle, session_id: &str, recorded_ms: u64) {
    update_title(app, |title| title.tick(session_id, recorded_ms));
}

/// The main window must show the page of this session (R3).
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct ShowSession {
    pub session_id: String,
}

/// Show the main window. While a session records or is interrupted, the
/// window also shows that session's page (R3), because the user who opens
/// Longhand from the tray during a recording wants the recording.
fn open_longhand(app: &AppHandle) {
    show_main_window(app);
    let Some(capture) = app.try_state::<Arc<CaptureManager>>() else {
        return;
    };
    match capture.status(app) {
        Ok(status) => {
            if let Some(session_id) = status.session_id {
                let _ = ShowSession { session_id }.emit(app);
            }
        }
        Err(e) => log::error!("tray open could not read the recording status: {e}"),
    }
}

pub fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    app.manage(TitleState(Mutex::new(TrayTitle::default())));
    let open = MenuItem::with_id(app, "open", OPEN, true, None::<&str>)?;
    app.manage(OpenItem(open.clone()));
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Longhand", true, Some("CmdOrCtrl+Q"))?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(TRAY_ICON)?)
        .icon_as_template(true)
        .tooltip("Longhand")
        .menu(&menu);

    // Windows notification-area convention: left click opens the app, right
    // click shows the menu. The macOS menu bar keeps the menu on left click.
    #[cfg(target_os = "windows")]
    {
        builder = builder
            .show_menu_on_left_click(false)
            .on_tray_icon_event(|tray, event| {
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
                let opens_window = matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                );
                if opens_window {
                    open_longhand(tray.app_handle());
                }
            });
    }
    #[cfg(not(target_os = "windows"))]
    {
        builder = builder.show_menu_on_left_click(true);
    }

    let tray = builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => open_longhand(app),
            "settings" => open_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    app.manage(tray);
    Ok(())
}

/// The text beside the menu-bar icon. An alert title takes precedence over
/// the recording time; with neither, the title is empty.
#[derive(Default)]
struct TrayTitle {
    alert: bool,
    /// The session now recording, and its last recorded time in whole seconds.
    recording: Option<(String, Option<u64>)>,
    /// The text last given to the tray, so an update that changes nothing
    /// makes no call.
    shown: String,
}

impl TrayTitle {
    /// Record a raised or cleared alert. Returns the new title when it changed.
    fn set_alert(&mut self, raised: bool) -> Option<String> {
        self.alert = raised;
        self.refresh()
    }

    /// Record a start, a stop, or a resume. Returns the new title when it changed.
    fn recording_changed(&mut self, session_id: Option<&str>) -> Option<String> {
        match session_id {
            // A resume of the same session keeps its time until the next tick.
            Some(id) if self.recording.as_ref().is_some_and(|(r, _)| r == id) => {}
            Some(id) => self.recording = Some((id.to_string(), None)),
            None => self.recording = None,
        }
        self.refresh()
    }

    /// Record a capture tick. Returns the new title when it changed, which
    /// is once a second while recording.
    fn tick(&mut self, session_id: &str, recorded_ms: u64) -> Option<String> {
        match &mut self.recording {
            Some((id, seconds)) if id == session_id => *seconds = Some(recorded_ms / 1000),
            // A tick after a stop, or from another session, changes nothing.
            _ => return None,
        }
        self.refresh()
    }

    fn text(&self) -> String {
        if self.alert {
            return ALERT_TITLE.to_string();
        }
        match &self.recording {
            Some((_, Some(seconds))) => recording_title(seconds * 1000),
            _ => String::new(),
        }
    }

    fn refresh(&mut self) -> Option<String> {
        let text = self.text();
        if text == self.shown {
            return None;
        }
        self.shown = text.clone();
        Some(text)
    }
}

/// The recording time for the menu bar: "● H:MM:SS".
fn recording_title(recorded_ms: u64) -> String {
    let seconds = recorded_ms / 1000;
    format!(
        "● {}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// The managed title state of the tray.
struct TitleState(Mutex<TrayTitle>);

impl TitleState {
    fn lock(&self) -> std::sync::MutexGuard<'_, TrayTitle> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Apply `change` to the title state, and give a changed title to the tray.
fn update_title(app: &AppHandle, change: impl FnOnce(&mut TrayTitle) -> Option<String>) {
    let Some(state) = app.try_state::<TitleState>() else {
        return;
    };
    if change(&mut state.lock()).is_some() {
        show_title(app);
    }
}

/// Queue the title change on the main thread, and do not wait for it. The
/// callers include the capture tick thread, which must not wait for the
/// main thread: a stop on the main thread holds the capture lock while it
/// waits for that thread. The queued call reads the newest title when it
/// runs, so the last call to run always shows the current title.
#[cfg(target_os = "macos")]
fn show_title(app: &AppHandle) {
    let handle = app.clone();
    let queued = app.run_on_main_thread(move || {
        let Some(state) = handle.try_state::<TitleState>() else {
            return;
        };
        let text = state.lock().shown.clone();
        let Some(tray) = handle.tray_by_id("main") else {
            return;
        };
        // An empty title, not `None`: tray-icon ignores `None` on macOS and
        // would leave the old title in the menu bar.
        if let Err(e) = tray.set_title(Some(text.as_str())) {
            log::warn!("the tray title could not change: {e}");
        }
    });
    if let Err(e) = queued {
        log::warn!("the tray title change could not be queued: {e}");
    }
}

/// Only the macOS menu bar shows a title beside the icon.
#[cfg(not(target_os = "macos"))]
fn show_title(_app: &AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_title_shows_hours_minutes_and_seconds() {
        assert_eq!(recording_title(5_000), "● 0:00:05");
        assert_eq!(recording_title(4_360_000), "● 1:12:40");
        assert_eq!(recording_title(0), "● 0:00:00");
        // Part seconds round down, as a clock does.
        assert_eq!(recording_title(5_999), "● 0:00:05");
    }

    #[test]
    fn ticks_update_the_title_once_a_second() {
        let mut title = TrayTitle::default();
        assert_eq!(title.recording_changed(Some("a")), None);
        assert_eq!(title.tick("a", 0), Some("● 0:00:00".into()));
        assert_eq!(title.tick("a", 250), None);
        assert_eq!(title.tick("a", 750), None);
        assert_eq!(title.tick("a", 1_000), Some("● 0:00:01".into()));
    }

    #[test]
    fn stop_empties_the_title_and_ignores_a_late_tick() {
        let mut title = TrayTitle::default();
        title.recording_changed(Some("a"));
        title.tick("a", 5_000);
        assert_eq!(title.recording_changed(None), Some(String::new()));
        assert_eq!(title.tick("a", 5_250), None);
    }

    #[test]
    fn a_tick_from_another_session_changes_nothing() {
        let mut title = TrayTitle::default();
        title.recording_changed(Some("a"));
        assert_eq!(title.tick("b", 5_000), None);
    }

    #[test]
    fn a_resume_keeps_the_time_until_the_next_tick() {
        let mut title = TrayTitle::default();
        title.recording_changed(Some("a"));
        title.tick("a", 5_000);
        assert_eq!(title.recording_changed(Some("a")), None);
        assert_eq!(title.tick("a", 6_000), Some("● 0:00:06".into()));
    }

    #[test]
    fn an_alert_takes_precedence_over_the_recording_time() {
        let mut title = TrayTitle::default();
        title.recording_changed(Some("a"));
        title.tick("a", 5_000);
        assert_eq!(title.set_alert(true), Some(ALERT_TITLE.into()));
        assert_eq!(title.tick("a", 6_000), None);
        assert_eq!(title.set_alert(false), Some("● 0:00:06".into()));
    }

    #[test]
    fn a_cleared_alert_with_no_recording_empties_the_title() {
        let mut title = TrayTitle::default();
        assert_eq!(title.set_alert(true), Some(ALERT_TITLE.into()));
        assert_eq!(title.set_alert(false), Some(String::new()));
    }
}
