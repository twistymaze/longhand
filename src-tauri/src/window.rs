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
pub const UPDATE_WINDOW: &str = "update";

pub fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    crate::interface_zoom::build(app, |percent| {
        let builder = WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::App("/".into()))
            .zoom_hotkeys_enabled(false)
            .initialization_script(crate::interface_zoom::initialization(percent))
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
        builder.build()
    })?;
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

/// The main window hid or closed (KTD7). The frontend then leaves the shown
/// note (R18) and reports that no note is shown.
#[derive(serde::Serialize, Clone, specta::Type, tauri_specta::Event)]
pub struct MainWindowHidden;

/// Hide the main window and remove the Dock icon when no other window is
/// visible. The close button and the show shortcut both hide this way.
fn hide_main_window(window: &Window) {
    if let Err(e) = window.hide() {
        log::error!("failed to hide main window: {e}");
    }
    let _ = tauri_specta::Event::emit(&MainWindowHidden, window.app_handle());
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
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || open_settings_window(&app));
}

fn settings_initial_size(percent: u16, work_area: Option<tauri::LogicalSize<f64>>) -> (f64, f64) {
    let Some(area) = work_area else {
        return (760.0, 540.0);
    };
    let scale = (f64::from(crate::interface_zoom::normalize(percent)) / 100.0).max(1.0);
    // Leave room for the title bar and screen edges. Saved geometry takes precedence.
    (
        (760.0 * scale).min((area.width - 32.0).max(640.0)),
        (540.0 * scale).min((area.height - 64.0).max(440.0)),
    )
}

fn open_settings_window(app: &AppHandle) {
    let opened = open_or_build(
        app.get_webview_window(SETTINGS_WINDOW),
        |window| front(app, window),
        || {
            // Before the build, so that the window's delegate has the method
            // when AppKit reads it.
            #[cfg(target_os = "macos")]
            crate::zoom::stop_zoom_of_windows_that_cannot_maximize();
            SETTINGS_OPENS.opened();
            crate::interface_zoom::build(app, |percent| {
                let monitor = app
                    .get_webview_window(MAIN_WINDOW)
                    .and_then(|window| window.current_monitor().ok().flatten())
                    .or_else(|| app.primary_monitor().ok().flatten());
                let work_area = monitor.map(|m| m.work_area().size.to_logical(m.scale_factor()));
                let (width, height) = settings_initial_size(percent, work_area);
                WebviewWindowBuilder::new(
                    app,
                    SETTINGS_WINDOW,
                    WebviewUrl::App("index.html?window=settings".into()),
                )
                .visible(false)
                .zoom_hotkeys_enabled(false)
                .initialization_script(crate::interface_zoom::initialization(percent))
                .title("Settings")
                .inner_size(width, height)
                .min_inner_size(640.0, 440.0)
                .maximizable(false)
                .minimizable(false)
                .center()
                .build()
            })
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
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || open_licences_window(&app));
}

fn open_licences_window(app: &AppHandle) {
    let opened = open_or_build(
        app.get_webview_window(LICENCES_WINDOW),
        |window| front(app, window),
        || {
            crate::interface_zoom::build(app, |percent| {
                WebviewWindowBuilder::new(
                    app,
                    LICENCES_WINDOW,
                    WebviewUrl::App("index.html?window=licences".into()),
                )
                .visible(false)
                .zoom_hotkeys_enabled(false)
                .initialization_script(crate::interface_zoom::initialization(percent))
                .title("Open Source Licences")
                .inner_size(680.0, 640.0)
                .min_inner_size(480.0, 320.0)
                .center()
                .build()
            })
        },
    );
    match opened {
        Ok((window, Opened::Built)) => front(app, &window),
        Ok((_, Opened::Focused)) => {}
        Err(e) => log::error!("failed to open the licences window: {e}"),
    }
}

/// Show or remove the update window to match the update state (KTD9). A
/// window that shows stays as it is: the page follows the state event, and
/// progress does not take the focus again. The page registers no close
/// listener, and a close by the user counts as "Remind Me Later" or "Later".
pub fn show_update_window(app: &AppHandle, visible: bool) {
    let existing = app.get_webview_window(UPDATE_WINDOW);
    match (visible, existing) {
        (true, None) => {
            let built = crate::interface_zoom::build(app, |percent| {
                // The page then sets the size that fits its content.
                let scale = f64::from(percent) / 100.0;
                WebviewWindowBuilder::new(
                    app,
                    UPDATE_WINDOW,
                    WebviewUrl::App("index.html?window=update".into()),
                )
                .visible(false)
                .zoom_hotkeys_enabled(false)
                .initialization_script(crate::interface_zoom::initialization(percent))
                .title("Software Update")
                .inner_size(480.0 * scale, 200.0 * scale)
                .resizable(false)
                .maximizable(false)
                .minimizable(false)
                .center()
                .build()
            });
            match built {
                Ok(window) => front(app, &window),
                Err(e) => log::error!("failed to open the update window: {e}"),
            }
        }
        (false, Some(window)) => {
            // The state closed the window, so the page's close is no choice.
            CLOSED_BY_STATE.store(true, Ordering::SeqCst);
            if let Err(e) = window.destroy() {
                CLOSED_BY_STATE.store(false, Ordering::SeqCst);
                log::error!("failed to close the update window: {e}");
            }
        }
        _ => {}
    }
}

/// True while the update manager closes the update window itself.
static CLOSED_BY_STATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    {
        let hide_from_dock = crate::managers::settings::get_settings(app).hide_from_dock;
        set_dock_policy(app, dock_policy(hide_from_dock, true));
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
        // A close by the user is a choice (KTD9). A close that the update
        // state made is not.
        WindowEvent::Destroyed if window.label() == UPDATE_WINDOW => {
            hide_dock_icon_if_no_window(window);
            if !CLOSED_BY_STATE.swap(false, Ordering::SeqCst) {
                let app = window.app_handle();
                if let Some(updates) =
                    app.try_state::<std::sync::Arc<crate::managers::updates::UpdateManager>>()
                {
                    updates.choose(crate::managers::updates::Choice::Close);
                }
            }
        }
        _ => {}
    }
}

/// Make the app Accessory (no Dock icon) when "Hide from Dock" is on and no
/// window other than `closing` is visible (R3). The activation policy stays
/// Regular while any window is visible, and always when the switch is off.
fn hide_dock_icon_if_no_window(closing: &Window) {
    #[cfg(target_os = "macos")]
    {
        let app = closing.app_handle();
        let other_visible = app
            .webview_windows()
            .values()
            .any(|w| w.label() != closing.label() && w.is_visible().unwrap_or(false));
        let hide_from_dock = crate::managers::settings::get_settings(app).hide_from_dock;
        set_dock_policy(app, dock_policy(hide_from_dock, other_visible));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = closing;
}

/// The macOS activation policy: Regular has a Dock icon, and Accessory has
/// none. The menu-bar icon shows in both (R3). Other platforms have no
/// Dock, so they use this type only in tests.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockPolicy {
    Regular,
    Accessory,
}

/// The policy for the "Hide from Dock" value and whether a window is
/// visible (KTD1). Every site that sets the activation policy calls this
/// function.
#[cfg(any(target_os = "macos", test))]
pub fn dock_policy(hide_from_dock: bool, window_visible: bool) -> DockPolicy {
    if hide_from_dock && !window_visible {
        DockPolicy::Accessory
    } else {
        DockPolicy::Regular
    }
}

/// The policy before the event loop starts. No window is visible yet
/// (KTD2).
#[cfg(any(target_os = "macos", test))]
pub fn startup_dock_policy(hide_from_dock: bool) -> DockPolicy {
    dock_policy(hide_from_dock, false)
}

/// Set the activation policy of the running app. A failure is logged.
#[cfg(target_os = "macos")]
pub fn set_dock_policy(app: &AppHandle, policy: DockPolicy) {
    if let Err(e) = app.set_activation_policy(policy.into()) {
        log::error!("failed to set activation policy to {policy:?}: {e}");
    }
}

#[cfg(target_os = "macos")]
impl From<DockPolicy> for tauri::ActivationPolicy {
    fn from(policy: DockPolicy) -> Self {
        match policy {
            DockPolicy::Regular => tauri::ActivationPolicy::Regular,
            DockPolicy::Accessory => tauri::ActivationPolicy::Accessory,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[test]
    fn new_settings_windows_allow_for_zoom_without_exceeding_the_work_area() {
        let large = Some(tauri::LogicalSize::new(2560.0, 1440.0));
        assert_eq!(settings_initial_size(100, large), (760.0, 540.0));
        assert_eq!(settings_initial_size(150, large), (1140.0, 810.0));
        assert_eq!(settings_initial_size(50, large), (760.0, 540.0));
        let laptop = Some(tauri::LogicalSize::new(1280.0, 800.0));
        assert_eq!(settings_initial_size(200, laptop), (1248.0, 736.0));
        assert_eq!(settings_initial_size(200, None), (760.0, 540.0));
    }

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
    fn with_the_switch_off_the_dock_icon_stays_when_no_window_is_visible() {
        assert_eq!(dock_policy(false, false), DockPolicy::Regular);
    }

    #[test]
    fn with_the_switch_on_the_dock_icon_goes_when_no_window_is_visible() {
        assert_eq!(dock_policy(true, false), DockPolicy::Accessory);
    }

    #[test]
    fn with_the_switch_on_a_visible_window_keeps_the_dock_icon() {
        assert_eq!(dock_policy(true, true), DockPolicy::Regular);
    }

    #[test]
    fn with_the_switch_off_a_visible_window_keeps_the_dock_icon() {
        assert_eq!(dock_policy(false, true), DockPolicy::Regular);
    }

    /// Startup has no visible window: a login launch with the switch on has
    /// no Dock icon, and with the switch off it has one (AE3).
    #[test]
    fn startup_policy_follows_the_switch() {
        assert_eq!(startup_dock_policy(true), DockPolicy::Accessory);
        assert_eq!(startup_dock_policy(false), DockPolicy::Regular);
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
