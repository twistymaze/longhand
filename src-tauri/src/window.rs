//! Window lifecycle: closing the main window hides it rather than exits
//! (KTD11), and Settings opens in a second window (KTD3).
//! Adapted from Handy's `lib.rs` (`show_main_window`, the `CloseRequested`
//! handler); see src-tauri/PROVENANCE.md.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{
    AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window, WindowEvent,
};

pub const MAIN_WINDOW: &str = "main";
pub const SETTINGS_WINDOW: &str = "settings";
pub const LICENCES_WINDOW: &str = "licences";

pub fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    let builder = WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::App("/".into()))
        .title("Longhand")
        .inner_size(960.0, 680.0)
        .min_inner_size(640.0, 480.0)
        .visible(false);
    // On macOS the title bar is empty and has the window's colour: the page
    // draws a strip under it, and the window buttons are centred in that
    // strip (`.title-strip` in index.css). Windows keeps its own title bar.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(20.0, window_button_inset()));
    builder.build()?;
    Ok(())
}

/// The inset that centres the window buttons in the page's 44-point title
/// strip. macOS 26 moved the buttons in their frame, so the same position
/// needs a larger inset there. Anarlog uses 25 and 19 for its 48-point
/// window header; the strip here is 4 points shorter, so each inset is 2
/// points less.
#[cfg(target_os = "macos")]
fn window_button_inset() -> f64 {
    if macos_major_version().is_some_and(|major| major >= 26) {
        23.0
    } else {
        17.0
    }
}

/// The major version of macOS, from the system's version file.
#[cfg(target_os = "macos")]
fn macos_major_version() -> Option<u32> {
    let plist = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").ok()?;
    let after_key = plist.split("<key>ProductVersion</key>").nth(1)?;
    let version = after_key
        .split("<string>")
        .nth(1)?
        .split("</string>")
        .next()?;
    version.trim().split('.').next()?.parse().ok()
}

pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        log::error!("main window not found");
        return;
    };
    if let Err(e) = window.unminimize() {
        log::error!("failed to unminimize main window: {e}");
    }
    front(app, &window);
}

/// What a press of the show shortcut does to the main window (KTD5).
#[derive(Debug, PartialEq, Eq)]
pub enum ToggleAction {
    Hide,
    Show,
}

/// A window that is visible and focused is in front, so the press hides it.
/// A window that is hidden, minimized, or behind another app or the
/// Settings window is shown (R6).
pub fn toggle_action(visible: bool, focused: bool) -> ToggleAction {
    if visible && focused {
        ToggleAction::Hide
    } else {
        ToggleAction::Show
    }
}

/// The show shortcut's press: hide the main window when it is in front, or
/// show it. A recording is not read or changed (R7).
pub fn toggle_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        log::error!("main window not found");
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    match toggle_action(visible, focused) {
        ToggleAction::Hide => hide_main_window(&window.as_ref().window()),
        ToggleAction::Show => show_main_window(app),
    }
}

/// Hide the main window and remove the Dock icon when no other window is
/// visible. The close button and the show shortcut both hide this way.
fn hide_main_window(window: &Window) {
    if let Err(e) = window.hide() {
        log::error!("failed to hide main window: {e}");
    }
    hide_dock_icon_if_no_window(window);
}

/// How `open_or_build` got the window.
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    /// The window was open, and was shown and focused.
    Focused,
    /// No window was open, so a new one was built.
    Built,
}

/// Focus `existing` when there is one, or else build a new window. A second
/// call while the window is open thus makes no second window.
pub fn open_or_build<W, E>(
    existing: Option<W>,
    focus: impl FnOnce(&W),
    build: impl FnOnce() -> Result<W, E>,
) -> Result<(W, Opened), E> {
    match existing {
        Some(window) => {
            focus(&window);
            Ok((window, Opened::Focused))
        }
        None => Ok((build()?, Opened::Built)),
    }
}

/// Show the Settings window, and build it first when it is not open (KTD3).
/// It loads the same page as the main window, with a query that tells the
/// page which window it is in.
pub fn open_settings(app: &AppHandle) {
    let opened = open_or_build(
        app.get_webview_window(SETTINGS_WINDOW),
        |window| front(app, window),
        || {
            // Before the build, so that the window's delegate has the method
            // when AppKit reads it.
            #[cfg(target_os = "macos")]
            crate::zoom::stop_zoom_of_windows_that_cannot_maximize();
            SETTINGS_OPENS.opened();
            WebviewWindowBuilder::new(
                app,
                SETTINGS_WINDOW,
                WebviewUrl::App("index.html?window=settings".into()),
            )
            .title("Settings")
            .inner_size(760.0, 540.0)
            .min_inner_size(640.0, 440.0)
            .maximizable(false)
            .minimizable(false)
            .center()
            .build()
        },
    );
    match opened {
        Ok((window, Opened::Built)) => front(app, &window),
        Ok((_, Opened::Focused)) => {}
        Err(e) => log::error!("failed to open the settings window: {e}"),
    }
}

/// Show the window with the full licence list, and build it first when it
/// is not open (R9, KTD7). It loads the same page with its own query. The
/// page registers no close listener, so a close always destroys the window.
pub fn open_licences(app: &AppHandle) {
    let opened = open_or_build(
        app.get_webview_window(LICENCES_WINDOW),
        |window| front(app, window),
        || {
            WebviewWindowBuilder::new(
                app,
                LICENCES_WINDOW,
                WebviewUrl::App("index.html?window=licences".into()),
            )
            .title("Open Source Licences")
            .inner_size(680.0, 640.0)
            .min_inner_size(480.0, 320.0)
            .center()
            .build()
        },
    );
    match opened {
        Ok((window, Opened::Built)) => front(app, &window),
        Ok((_, Opened::Focused)) => {}
        Err(e) => log::error!("failed to open the licences window: {e}"),
    }
}

/// The time from a close request of the Settings window to the destroy
/// here. The page's close takes at most 5 s (2 s for the save and 1 s for
/// each of three later steps), so this limit applies only when the page did
/// not close the window.
///
/// The page's close listener is the usual cause. Tauri 2.11 keeps the
/// listener records of a webview after the webview is destroyed, and it
/// stops the close of each later window with the same label that has a
/// record. The page removes its listener before it closes the window, but
/// a page that stops (a crash, a reload) cannot.
const SETTINGS_CLOSE_LIMIT: Duration = Duration::from_secs(6);

/// Counts the Settings windows that were built, so that a late destroy does
/// not close a new window that opened after the close request.
struct OpenCount(AtomicU64);

impl OpenCount {
    const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// Record a new window.
    fn opened(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    /// The number of the window that is open now.
    fn current(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }

    /// Whether window `number` is still the last window built.
    fn is_current(&self, number: u64) -> bool {
        self.current() == number
    }
}

static SETTINGS_OPENS: OpenCount = OpenCount::new();

/// Show and focus `window`. A visible window keeps the Dock icon, so the app
/// becomes Regular again if a hide made it Accessory.
fn front(app: &AppHandle, window: &WebviewWindow) {
    if let Err(e) = window.show() {
        log::error!("failed to show the {} window: {e}", window.label());
    }
    if let Err(e) = window.set_focus() {
        log::error!("failed to focus the {} window: {e}", window.label());
    }
    // Showing promotes the app from Accessory to Regular, which is the
    // direction Apple supports at runtime.
    #[cfg(target_os = "macos")]
    if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
        log::error!("failed to set activation policy to Regular: {e}");
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    match event {
        WindowEvent::CloseRequested { api, .. } if window.label() == MAIN_WINDOW => {
            // The process stays alive in the tray. This is an optimization only:
            // processing survives a real quit through checkpoints, not through this.
            // `RunEvent::ExitRequested` + `prevent_exit()` is deliberately not used
            // (tauri-apps/tauri#13511: it makes the app unkillable).
            api.prevent_close();
            hide_main_window(window);
        }
        // The Settings page stops the close while it saves, and then
        // destroys the window. If the window is open after
        // `SETTINGS_CLOSE_LIMIT`, it is destroyed here.
        WindowEvent::CloseRequested { .. } if window.label() == SETTINGS_WINDOW => {
            let open = SETTINGS_OPENS.current();
            let app = window.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(SETTINGS_CLOSE_LIMIT).await;
                if !SETTINGS_OPENS.is_current(open) {
                    return;
                }
                let Some(window) = app.get_webview_window(SETTINGS_WINDOW) else {
                    return;
                };
                log::warn!(
                    "the Settings window is open {} s after a close request; destroying it",
                    SETTINGS_CLOSE_LIMIT.as_secs()
                );
                if let Err(e) = window.destroy() {
                    log::error!("failed to destroy the settings window: {e}");
                }
            });
        }
        // The Dock icon goes when the Settings window was the last visible
        // window.
        WindowEvent::Destroyed if window.label() == SETTINGS_WINDOW => {
            hide_dock_icon_if_no_window(window);
            // A capture that was in progress when the window closed must not
            // leave the shortcuts paused (KTD6). Off the main thread, since a
            // change in progress holds the lock and waits on the main thread.
            let app = window.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || crate::shortcut::resume(&app));
        }
        // The Dock icon goes when the licences window was the last visible
        // window.
        WindowEvent::Destroyed if window.label() == LICENCES_WINDOW => {
            hide_dock_icon_if_no_window(window);
        }
        _ => {}
    }
}

/// Make the app Accessory (no Dock icon) when no window other than `closing`
/// is visible. The activation policy stays Regular while any window is
/// visible.
fn hide_dock_icon_if_no_window(closing: &Window) {
    #[cfg(target_os = "macos")]
    {
        let app = closing.app_handle();
        let other_visible = app
            .webview_windows()
            .values()
            .any(|w| w.label() != closing.label() && w.is_visible().unwrap_or(false));
        if other_visible {
            return;
        }
        if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Accessory) {
            log::error!("failed to set activation policy to Accessory: {e}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = closing;
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    /// A stand-in for the app's window set: a second open call focuses the
    /// window the first call built.
    #[test]
    fn a_second_open_focuses_the_window_and_builds_no_second_one() {
        let windows: RefCell<Vec<u32>> = RefCell::new(Vec::new());
        let builds = Cell::new(0);
        let focuses = Cell::new(0);
        let open = || {
            let existing = windows.borrow().first().copied();
            open_or_build(
                existing,
                |_| focuses.set(focuses.get() + 1),
                || {
                    builds.set(builds.get() + 1);
                    windows.borrow_mut().push(7);
                    Ok::<_, ()>(7)
                },
            )
        };
        assert_eq!(open(), Ok((7, Opened::Built)));
        assert_eq!(open(), Ok((7, Opened::Focused)));
        assert_eq!(builds.get(), 1);
        assert_eq!(focuses.get(), 1);
        assert_eq!(windows.borrow().len(), 1);
    }

    #[test]
    fn a_window_in_front_is_hidden() {
        assert_eq!(toggle_action(true, true), ToggleAction::Hide);
    }

    #[test]
    fn a_window_behind_another_app_is_shown() {
        assert_eq!(toggle_action(true, false), ToggleAction::Show);
    }

    #[test]
    fn a_hidden_window_is_shown_whatever_its_focus_reads() {
        assert_eq!(toggle_action(false, false), ToggleAction::Show);
        assert_eq!(toggle_action(false, true), ToggleAction::Show);
    }

    #[test]
    fn a_window_that_opens_after_a_close_request_is_not_the_closed_window() {
        let opens = OpenCount::new();
        opens.opened();
        let closing = opens.current();
        assert!(opens.is_current(closing));
        opens.opened();
        assert!(!opens.is_current(closing));
    }

    #[test]
    fn a_build_error_returns_the_error() {
        let result = open_or_build(None::<u32>, |_| {}, || Err("no window"));
        assert_eq!(result, Err("no window"));
    }
}
