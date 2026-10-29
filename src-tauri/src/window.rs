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
pub const CHANGELOG_WINDOW: &str = "changelog";
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
        #[cfg(target_os = "windows")]
        let builder = builder.menu(crate::menu::main_window_menu(app)?);
        builder.build()
    })?;
    #[cfg(target_os = "macos")]
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        crate::file_promise::accept_promised_files(app, &window);
    }
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

/// A pane of the Settings window. The values agree with `PaneId` in
/// src/settings/Status.tsx.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SettingsPane {
    Status,
    Recording,
    Destination,
    Types,
    Provider,
    Summarization,
    Recognition,
    About,
}

impl SettingsPane {
    /// The name of the pane in the page and in the window URL.
    fn id(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Recording => "recording",
            Self::Destination => "destination",
            Self::Types => "types",
            Self::Provider => "provider",
            Self::Summarization => "summarization",
            Self::Recognition => "recognition",
            Self::About => "about",
        }
    }
}

/// An open Settings window must show this pane (KTD10). A new Settings
/// window reads its pane from its URL instead.
#[derive(serde::Serialize, Clone, specta::Type, tauri_specta::Event)]
pub struct SettingsPaneRequested {
    pub pane: SettingsPane,
}

/// Show the Settings window, and build it first when it is not open (KTD3).
/// It loads the same page as the main window, with a query that tells the
/// page which window it is in.
pub fn open_settings(app: &AppHandle) {
    open_settings_on(app, None);
}

/// Show the Settings window on `pane` (KTD10). With no pane, a new window
/// shows its first pane and an open window keeps its pane.
pub fn open_settings_on(app: &AppHandle, pane: Option<SettingsPane>) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || open_settings_window(&app, pane));
}

/// The page URL of a new Settings window. The `pane` query names the pane
/// that the window shows first.
fn settings_url(pane: Option<SettingsPane>) -> String {
    match pane {
        Some(pane) => format!("index.html?window=settings&pane={}", pane.id()),
        None => "index.html?window=settings".into(),
    }
}

/// Send `pane` to the open Settings window and focus it, or else build a
/// new window from the URL that names `pane`.
fn open_settings_with<W, E>(
    existing: Option<W>,
    pane: Option<SettingsPane>,
    focus: impl FnOnce(&W),
    send_pane: impl FnOnce(&W, SettingsPane),
    build: impl FnOnce(String) -> Result<W, E>,
) -> Result<(W, Opened), E> {
    open_or_build(
        existing,
        |window| {
            if let Some(pane) = pane {
                send_pane(window, pane);
            }
            focus(window);
        },
        || build(settings_url(pane)),
    )
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

fn open_settings_window(app: &AppHandle, pane: Option<SettingsPane>) {
    let opened = open_settings_with(
        app.get_webview_window(SETTINGS_WINDOW),
        pane,
        |window| front(app, window),
        |_, pane| {
            let event = SettingsPaneRequested { pane };
            if let Err(e) = tauri_specta::Event::emit_to(&event, app, SETTINGS_WINDOW) {
                log::error!("failed to send the pane to the settings window: {e}");
            }
        },
        |url| {
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
                WebviewWindowBuilder::new(app, SETTINGS_WINDOW, WebviewUrl::App(url.into()))
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
    open_page_window(
        app,
        LICENCES_WINDOW,
        "index.html?window=licences",
        "Open Source Licences",
        (680.0, 640.0),
        (480.0, 320.0),
        "licences",
    );
}

/// Show the Recent changes window, and build it first when it is not open
/// (U5 and KTD8 of the monthly changelog plan). It loads the same page with
/// its own query. The page registers no close listener, so a close always
/// destroys the window.
pub fn open_changelog(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || open_changelog_window(&app));
}

fn open_changelog_window(app: &AppHandle) {
    open_page_window(
        app,
        CHANGELOG_WINDOW,
        "index.html?window=changelog",
        "Recent Changes",
        (560.0, 640.0),
        (400.0, 320.0),
        "Recent changes",
    );
}

/// Show the window with this label, and build it first when it is not open.
/// The window loads `url` and starts hidden; `front` shows it. `name` is the
/// name of the window in the log.
fn open_page_window(
    app: &AppHandle,
    label: &str,
    url: &str,
    title: &str,
    size: (f64, f64),
    min_size: (f64, f64),
    name: &str,
) {
    let opened = open_or_build(
        app.get_webview_window(label),
        |window| front(app, window),
        || {
            crate::interface_zoom::build(app, |percent| {
                WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
                    .visible(false)
                    .zoom_hotkeys_enabled(false)
                    .initialization_script(crate::interface_zoom::initialization(percent))
                    .title(title)
                    .inner_size(size.0, size.1)
                    .min_inner_size(min_size.0, min_size.1)
                    .center()
                    .build()
            })
        },
    );
    match opened {
        Ok((window, Opened::Built)) => front(app, &window),
        Ok((_, Opened::Focused)) => {}
        Err(e) => log::error!("failed to open the {name} window: {e}"),
    }
}

/// Show or remove the update window to match the update state (KTD9). A
/// window that shows stays as it is: the page follows the state event, and
/// a state change does not bring it to the front. Only `focus_now` (the
/// user asked again) does. The page registers no close listener, and a
/// close by the user counts as "Remind Me Later" or "Later". `with_key`
/// gives a new window the focus (KTD6).
pub fn show_update_window(app: &AppHandle, visible: bool, with_key: bool, focus_now: bool) {
    let existing = app.get_webview_window(UPDATE_WINDOW);
    match (visible, existing) {
        (true, Some(window)) if focus_now => front(app, &window),
        (true, None) => {
            UPDATE_WITH_KEY.store(with_key, Ordering::SeqCst);
            // As a macOS alert, the window has no interface zoom. The page
            // then sets the size that fits its content.
            let built = WebviewWindowBuilder::new(
                app,
                UPDATE_WINDOW,
                WebviewUrl::App("index.html?window=update".into()),
            )
            .visible(false)
            .zoom_hotkeys_enabled(false)
            .title("Software Update")
            .inner_size(480.0, 200.0)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .center()
            // An automatic notice opens when a Longhand window is in front,
            // maybe while the user types in a note. It opens without the
            // focus (`show` with `focus` false), so the key presses stay
            // where the user types. It is a normal window, and goes behind
            // another app that the user brings to the front (R10).
            .focused(false)
            // The first click on a button chooses, also when Longhand is
            // not the active app.
            .accept_first_mouse(true)
            .build();
            match built {
                // The page shows the window when it has the size of the
                // content (`show_built_update_window`). If the page does not
                // do this in `UPDATE_SHOW_LIMIT`, the window shows here.
                // The limit applies only to this window: a window that is
                // built after this one closes waits for its own page.
                Ok(_) => {
                    UPDATE_OPENS.opened();
                    let open = UPDATE_OPENS.current();
                    let app = app.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(UPDATE_SHOW_LIMIT);
                        if UPDATE_OPENS.is_current(open) {
                            show_built_update_window(&app);
                        }
                    });
                }
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

/// The time from the build of the update window to its show when the page
/// does not show it.
const UPDATE_SHOW_LIMIT: Duration = Duration::from_secs(1);

/// Show the update window if it is open and hidden. It takes the focus only
/// when the user asked for it.
pub fn show_built_update_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(UPDATE_WINDOW)
        && !window.is_visible().unwrap_or(true)
    {
        show(app, &window, UPDATE_WITH_KEY.load(Ordering::SeqCst));
    }
}

/// True when the update window that is open was built with the focus.
static UPDATE_WITH_KEY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The windows that show that the user uses Longhand (KTD3). The widget
/// panel never becomes key, and the update window does not count.
const ACTIVE_WINDOWS: [&str; 4] = [
    MAIN_WINDOW,
    SETTINGS_WINDOW,
    LICENCES_WINDOW,
    CHANGELOG_WINDOW,
];

/// True when a Longhand window is the key window (R1). A key window also
/// means that the app is not hidden.
pub fn longhand_window_active(app: &AppHandle) -> bool {
    ACTIVE_WINDOWS.iter().any(|label| {
        app.get_webview_window(label)
            .is_some_and(|w| w.is_focused().unwrap_or(false))
    })
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

/// Counts the windows with one label that were built, so that a late action
/// does not change a new window: a late destroy of a Settings window, or a
/// late show of an update window.
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
static UPDATE_OPENS: OpenCount = OpenCount::new();

/// Put the window in front of the other windows, also when Longhand is not
/// the active app, but do not make it the key window. The key presses then
/// stay in the window where the user types. A click on the window makes it
/// the key window as usual. tao's show always makes the window the key
/// window, so this calls AppKit directly. Call it on the main thread.
#[cfg(target_os = "macos")]
fn order_front_without_key(window: &WebviewWindow) -> tauri::Result<()> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    let ns_window = window.ns_window()?.cast::<AnyObject>();
    // SAFETY: `ns_window` is the live NSWindow of `window`, and the caller
    // runs on the main thread, as AppKit requires.
    unsafe {
        let _: () = msg_send![ns_window, orderFrontRegardless];
    }
    Ok(())
}

/// Show and focus `window`. A visible window keeps the Dock icon, so the app
/// becomes Regular again if a hide made it Accessory.
fn front(app: &AppHandle, window: &WebviewWindow) {
    show(app, window, true);
}

/// Show the window, and with `focus`, make it the focused window.
///
/// On macOS the window-state plugin moves and resizes a new window through
/// tao, which queues the change on the main dispatch queue. A show at once
/// would draw the window at its default place first. The show goes on the
/// same queue, so it comes after the change.
fn show(app: &AppHandle, window: &WebviewWindow, focus: bool) {
    #[cfg(target_os = "macos")]
    {
        let app = app.clone();
        let window = window.clone();
        dispatch2::DispatchQueue::main().exec_async(move || show_now(&app, &window, focus));
    }
    #[cfg(not(target_os = "macos"))]
    show_now(app, window, focus);
}

fn show_now(app: &AppHandle, window: &WebviewWindow, focus: bool) {
    #[cfg(target_os = "macos")]
    let shown = if focus {
        window.show()
    } else {
        order_front_without_key(window)
    };
    #[cfg(not(target_os = "macos"))]
    let shown = window.show();
    if let Err(e) = shown {
        log::error!("failed to show the {} window: {e}", window.label());
    }
    if focus && let Err(e) = window.set_focus() {
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
        // The Dock icon goes when the licences window or the Recent changes
        // window was the last visible window.
        WindowEvent::Destroyed
            if window.label() == LICENCES_WINDOW || window.label() == CHANGELOG_WINDOW =>
        {
            hide_dock_icon_if_no_window(window);
        }
        // A Longhand window in front can show a notice that waits (R1).
        WindowEvent::Focused(true) if ACTIVE_WINDOWS.contains(&window.label()) => {
            crate::managers::updates::window_activated(window.app_handle());
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
        let other_visible = app.webview_windows().values().any(|w| {
            keeps_dock_icon(w.label(), closing.label()) && w.is_visible().unwrap_or(false)
        });
        let hide_from_dock = crate::managers::settings::get_settings(app).hide_from_dock;
        set_dock_policy(app, dock_policy(hide_from_dock, other_visible));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = closing;
}

/// Whether a visible window `label` keeps the Dock icon while `closing`
/// closes. The recording widget does not: it is a panel, not a window that
/// the user opened (KTD9).
#[cfg(any(target_os = "macos", test))]
fn keeps_dock_icon(label: &str, closing: &str) -> bool {
    label != closing && label != crate::widget::WIDGET_WINDOW
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

    // With "Hide from Dock" on, the main window closed and the pill visible,
    // the Dock icon goes: the pill does not count as a window (KTD9).
    #[test]
    fn the_recording_widget_does_not_keep_the_dock_icon() {
        assert!(!keeps_dock_icon(crate::widget::WIDGET_WINDOW, MAIN_WINDOW));
        assert!(keeps_dock_icon(SETTINGS_WINDOW, MAIN_WINDOW));
        assert!(!keeps_dock_icon(MAIN_WINDOW, MAIN_WINDOW));
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
    fn a_new_settings_window_reads_its_pane_from_the_url() {
        assert_eq!(settings_url(None), "index.html?window=settings");
        let mut urls = Vec::new();
        let sent = Cell::new(0);
        let opened = open_settings_with(
            None,
            Some(SettingsPane::Provider),
            |_| {},
            |_, _| sent.set(sent.get() + 1),
            |url| {
                urls.push(url);
                Ok::<_, ()>(7)
            },
        );
        assert_eq!(opened, Ok((7, Opened::Built)));
        assert_eq!(urls, ["index.html?window=settings&pane=provider"]);
        assert_eq!(sent.get(), 0);
    }

    #[test]
    fn an_open_settings_window_receives_the_pane() {
        let sent = RefCell::new(Vec::new());
        let focuses = Cell::new(0);
        let send = |window: &u32, pane| sent.borrow_mut().push((*window, pane));
        let opened = open_settings_with(
            Some(7),
            Some(SettingsPane::Provider),
            |_| focuses.set(focuses.get() + 1),
            send,
            |_| Err("no second window"),
        );
        assert_eq!(opened, Ok((7, Opened::Focused)));
        assert_eq!(*sent.borrow(), [(7, SettingsPane::Provider)]);
        assert_eq!(focuses.get(), 1);

        let opened = open_settings_with(Some(7), None, |_| {}, send, |_| Err("no second window"));
        assert_eq!(opened, Ok((7, Opened::Focused)));
        assert_eq!(sent.borrow().len(), 1);
    }

    #[test]
    fn a_build_error_returns_the_error() {
        let result = open_or_build(None::<u32>, |_| {}, || Err("no window"));
        assert_eq!(result, Err("no window"));
    }
}
