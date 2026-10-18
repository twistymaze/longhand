//! The recording widget: a small pill that is on screen while a note is
//! open (R1). Rust decides when it shows (KTD1). The choice for each note
//! is one entry in memory, keyed by the session id (KTD2).
//!
//! The window is built once, hidden, and never destroyed (KTD9). On macOS it
//! is a non-activating panel (KTD3); on Windows a topmost window that takes
//! no focus (KTD4). The window settings and the placement are adapted from
//! Handy's `overlay.rs`; see src-tauri/PROVENANCE.md.

use tauri::{AppHandle, Manager, WebviewUrl};

/// The label of the widget window.
pub const WIDGET_WINDOW: &str = "widget";

/// The window size in logical points. The pill page draws the pill and the
/// × inside it.
const WIDTH: f64 = 288.0;
const HEIGHT: f64 = 64.0;

/// The space between the window and the bottom of the work area, in logical
/// points (R3). The work area ends above the Dock or the taskbar.
const BOTTOM_OFFSET: f64 = 12.0;

/// The widget choice for the open note (KTD2). It lives only in memory: a
/// relaunch forgets it (R16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub session_id: String,
    /// Whether the pill shows for this note.
    pub shown: bool,
    /// The display of the pill, from its first show for this note (R5).
    pub display: Option<DisplayKey>,
}

/// The entry after a sync. `open` is the open session, if one is open
/// (recording, paused or interrupted). `switch` is the Settings switch
/// (R11), which only a new note reads (R14, R16).
pub fn next_entry(open: Option<&str>, entry: Option<Entry>, switch: bool) -> Option<Entry> {
    let id = open?;
    match entry {
        Some(entry) if entry.session_id == id => Some(entry),
        _ => Some(Entry {
            session_id: id.to_string(),
            shown: switch,
            display: None,
        }),
    }
}

/// What the Show/Hide action does (R13, R15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressAction {
    /// No note is open: nothing changes, and nothing is kept (R15).
    None,
    Show,
    Hide,
}

/// The Show/Hide action for the current entry.
pub fn press_action(entry: Option<&Entry>) -> PressAction {
    match entry {
        None => PressAction::None,
        Some(entry) if entry.shown => PressAction::Hide,
        Some(_) => PressAction::Show,
    }
}

/// A request to the widget thread (KTD1, KTD11). Only the widget thread
/// changes the entry, so a request never waits for a lock. A choice names
/// the note that was open when the user made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// The capture state or the switch may have changed.
    Sync,
    /// The × on the pill: hide it until the note stops (R12).
    Hide(String),
    /// The Show/Hide action from the tray or the shortcut (R13).
    Toggle(String),
}

/// The entry after one request: the open note and the switch first, then
/// the user's choice. A choice with no open note changes nothing (R15). A
/// choice for a note that is no longer open changes nothing either: the
/// note stopped before the widget thread read the request.
pub fn step(
    open: Option<&str>,
    entry: Option<Entry>,
    switch: bool,
    request: Request,
) -> Option<Entry> {
    let mut entry = next_entry(open, entry, switch)?;
    let action = press_action(Some(&entry));
    match request {
        Request::Hide(id) if id == entry.session_id => entry.shown = false,
        Request::Toggle(id) if id == entry.session_id => {
            entry.shown = action == PressAction::Show;
        }
        _ => {}
    }
    Some(entry)
}

/// The sending end of the widget thread's requests.
struct Requests(std::sync::mpsc::Sender<Request>);

impl Requests {
    /// Send without waiting for the widget thread.
    fn send(&self, request: Request) {
        let _ = self.0.send(request);
    }
}

/// How often the widget thread checks while a note is open: for a state
/// change with no event (the auto-queue of an interrupted note, KTD1) and
/// for a display change (R5, KTD7).
const RECONCILE: std::time::Duration = std::time::Duration::from_secs(1);

/// Ask the widget thread to sync. It never waits: `capture::changed` calls
/// it while the capture lock is held (KTD1).
pub fn request(app: &AppHandle, request: Request) {
    if let Some(requests) = app.try_state::<Requests>() {
        requests.send(request);
    }
}

/// The Show/Hide action from the tray or the shortcut (R13): toggle the
/// pill of the note that is open now. With no open note it does nothing
/// (R15).
pub fn toggle_open_note(app: &AppHandle) {
    let Some(capture) = app.try_state::<std::sync::Arc<crate::managers::capture::CaptureManager>>()
    else {
        return;
    };
    match capture.status(app) {
        Ok(status) => {
            if let Some(id) = status.session_id {
                request(app, Request::Toggle(id));
            }
        }
        Err(e) => log::warn!("the recording widget could not read the recording status: {e}"),
    }
}

/// The pill window was shown. The page reads the status again, because a
/// hidden page can hold an old state (KTD10).
#[derive(serde::Serialize, Clone, specta::Type, tauri_specta::Event)]
pub struct WidgetShown;

/// Start the widget thread, and sync once for a note that is open at
/// launch (a note that a crash interrupted).
pub fn start(app: &AppHandle) {
    let (sender, receiver) = std::sync::mpsc::channel();
    app.manage(Requests(sender));
    let thread_app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("longhand-widget".into())
        .spawn(move || run(&thread_app, &receiver));
    if let Err(e) = spawned {
        log::error!("could not start the recording widget: {e}");
        return;
    }
    request(app, Request::Sync);
}

/// The widget thread: one sync for each request, and one each second while
/// a note is open.
fn run(app: &AppHandle, receiver: &std::sync::mpsc::Receiver<Request>) {
    use std::sync::mpsc::RecvTimeoutError;

    let mut entry: Option<Entry> = None;
    let mut visible = false;
    let mut item: Option<PressAction> = None;
    loop {
        let request = if entry.is_some() {
            match receiver.recv_timeout(RECONCILE) {
                Ok(request) => request,
                Err(RecvTimeoutError::Timeout) => Request::Sync,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match receiver.recv() {
                Ok(request) => request,
                Err(_) => return,
            }
        };
        entry = sync(app, entry, request, &mut visible);
        // The tray item names the next press of Show/Hide (R13).
        let action = press_action(entry.as_ref());
        if item != Some(action) {
            crate::tray::set_widget_item(app, action);
            item = Some(action);
        }
    }
}

/// Read the capture status and the switch, apply the request, then show,
/// place or hide the pill (KTD1).
fn sync(
    app: &AppHandle,
    entry: Option<Entry>,
    request: Request,
    visible: &mut bool,
) -> Option<Entry> {
    let Some(capture) = app.try_state::<std::sync::Arc<crate::managers::capture::CaptureManager>>()
    else {
        return entry;
    };
    let open = match capture.status(app) {
        Ok(status) => status.session_id,
        Err(e) => {
            log::warn!("the recording widget could not read the recording status: {e}");
            return entry;
        }
    };
    let switch = crate::managers::settings::get_settings(app).show_recording_widget;
    let mut entry = step(open.as_deref(), entry, switch, request);

    let show = entry.as_ref().is_some_and(|e| e.shown);
    if show {
        let remembered = entry.as_ref().and_then(|e| e.display);
        let display = on_main_thread(app, move |app| place_and_show(app, remembered)).flatten();
        if let Some(e) = entry.as_mut() {
            e.display = display.or(e.display);
        }
        if !*visible {
            use tauri_specta::Event as _;
            let _ = WidgetShown.emit(app);
        }
    } else if *visible {
        // The user can have dragged the pill to another display since the
        // last sync. A later show for this note uses that display.
        let display = on_main_thread(app, hide).flatten();
        if let Some(e) = entry.as_mut() {
            e.display = display.or(e.display);
        }
    }
    *visible = show;
    entry
}

/// Run `work` on the main thread and wait for its result. Only the widget
/// thread waits here; the main thread never waits for the widget thread.
fn on_main_thread<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(&AppHandle) -> T + Send + 'static,
) -> Option<T> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let main_app = app.clone();
    let queued = app.run_on_main_thread(move || {
        let _ = sender.send(work(&main_app));
    });
    if let Err(e) = queued {
        log::error!("the recording widget could not reach the main thread: {e}");
        return None;
    }
    receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .ok()
}

/// A display, identified by its bounds in physical pixels (KTD7). An index
/// or a name can change when a display goes away; the bounds do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayKey {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A display and its work area, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub key: DisplayKey,
    /// The bottom edge of the work area: above the Dock or the taskbar.
    pub work_bottom: i32,
    pub scale: f64,
}

impl Screen {
    fn of(monitor: &tauri::Monitor) -> Self {
        let work = monitor.work_area();
        Screen {
            key: DisplayKey {
                x: monitor.position().x,
                y: monitor.position().y,
                width: monitor.size().width,
                height: monitor.size().height,
            },
            work_bottom: work.position.y + work.size.height as i32,
            scale: monitor.scale_factor(),
        }
    }
}

/// The display for the pill (R5, KTD7): the remembered display while it is
/// there, else the display under the pointer, else the primary display.
pub fn choose_screen(
    screens: &[Screen],
    remembered: Option<DisplayKey>,
    under_pointer: Option<DisplayKey>,
    primary: Option<DisplayKey>,
) -> Option<Screen> {
    [remembered, under_pointer, primary]
        .into_iter()
        .flatten()
        .find_map(|key| screens.iter().find(|s| s.key == key).copied())
        .or_else(|| screens.first().copied())
}

/// The window origin in logical points: the bottom centre of the work area
/// (R3). macOS places windows in logical points of the target display,
/// because tao converts a physical position with the scale of the display
/// the window is on now (KTD7).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn logical_origin(screen: &Screen) -> (f64, f64) {
    let s = screen.scale;
    let x = f64::from(screen.key.x) / s + (f64::from(screen.key.width) / s - WIDTH) / 2.0;
    let y = f64::from(screen.work_bottom) / s - HEIGHT - BOTTOM_OFFSET;
    (x, y)
}

/// The window bounds in physical pixels of the target display: x, y, width,
/// height. Windows places windows in physical pixels (KTD7). WebView2
/// applies the Windows text size `text_scale` as a zoom of the page, so the
/// window grows with it. The offset under the window uses the display scale
/// only, so the bottom edge does not move.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn physical_bounds(screen: &Screen, text_scale: f64) -> (i32, i32, i32, i32) {
    let s = screen.scale;
    let content = s * text_scale;
    let width = (WIDTH * content).round() as i32;
    let height = (HEIGHT * content).round() as i32;
    let x = screen.key.x + (screen.key.width as i32 - width) / 2;
    let y = screen.work_bottom - height - (BOTTOM_OFFSET * s).round() as i32;
    (x, y, width, height)
}

/// The Windows text size from its `TextScaleFactor` percentage, from 1.0 to
/// 2.25. The value is absent until the user changes the text size.
#[cfg_attr(not(windows), allow(dead_code))]
fn text_scale_of(percent: Option<u32>) -> f64 {
    percent.map_or(1.0, |p| (f64::from(p) / 100.0).clamp(1.0, 2.25))
}

/// The Windows text size (Settings > Accessibility > Text size). It is
/// separate from the display scale.
#[cfg(windows)]
fn windows_text_scale_factor() -> f64 {
    text_scale_of(
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Accessibility")
            .and_then(|key| key.get_value::<u32, _>("TextScaleFactor"))
            .ok(),
    )
}

/// The window origin on a display in physical pixels, when the pill is at
/// its default place. A position that the user dragged the pill to is
/// compared with this.
pub fn default_position(screen: &Screen) -> (i32, i32) {
    #[cfg(not(windows))]
    {
        let (x, y) = logical_origin(screen);
        (
            (x * screen.scale).round() as i32,
            (y * screen.scale).round() as i32,
        )
    }
    #[cfg(windows)]
    {
        let (x, y, _, _) = physical_bounds(screen, windows_text_scale_factor());
        (x, y)
    }
}

/// A place that the user dragged the pill to: the display and the window
/// origin in physical pixels. It applies to later notes until Longhand
/// quits, while that display is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moved {
    pub display: DisplayKey,
    pub x: i32,
    pub y: i32,
}

/// The place that the user dragged the pill to, kept while Longhand runs.
/// Only the main thread reads and writes it.
#[derive(Default)]
struct MovedPlace(std::sync::Mutex<Option<Moved>>);

/// Where the pill goes when the widget thread shows it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// The pill is visible on a display that is there: leave it, because
    /// the user can have dragged it.
    Keep(DisplayKey),
    /// Put the pill at this window origin in physical pixels.
    At { screen: Screen, x: i32, y: i32 },
}

/// Decide where the pill goes (R3, R5). A visible pill stays where it is
/// while its display is there. A pill that shows, or whose display is gone,
/// goes to the place that the user dragged it to if that display is there,
/// else to the bottom centre of the chosen display.
pub fn placement(
    screens: &[Screen],
    visible_on: Option<DisplayKey>,
    remembered: Option<DisplayKey>,
    under_pointer: Option<DisplayKey>,
    primary: Option<DisplayKey>,
    moved: Option<Moved>,
) -> Option<Placement> {
    if let Some(key) = visible_on
        && screens.iter().any(|s| s.key == key)
    {
        return Some(Placement::Keep(key));
    }
    let preferred = remembered.or(moved.map(|m| m.display));
    let screen = choose_screen(screens, preferred, under_pointer, primary)?;
    let (x, y) = match moved {
        Some(m) if m.display == screen.key => (m.x, m.y),
        _ => default_position(&screen),
    };
    Some(Placement::At { screen, x, y })
}

/// The place to keep when the pill hides at `(x, y)` on `screen`: `None`
/// when the pill is at its default place, so a later note uses the
/// default place of its display.
pub fn moved_place(screen: &Screen, x: i32, y: i32) -> Option<Moved> {
    let (dx, dy) = default_position(screen);
    // A small difference comes from rounding, not from a drag.
    let slack = (2.0 * screen.scale).ceil() as i32;
    if (x - dx).abs() <= slack && (y - dy).abs() <= slack {
        return None;
    }
    Some(Moved {
        display: screen.key,
        x,
        y,
    })
}

/// The displays now. Call on the main thread.
fn screens(app: &AppHandle) -> Vec<Screen> {
    app.available_monitors()
        .map(|monitors| monitors.iter().map(Screen::of).collect())
        .unwrap_or_default()
}

/// The display under the pointer and the primary display. Call on the main
/// thread.
fn pointer_and_primary(app: &AppHandle) -> (Option<DisplayKey>, Option<DisplayKey>) {
    let under_pointer = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .map(|m| Screen::of(&m).key);
    let primary = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| Screen::of(&m).key);
    (under_pointer, primary)
}

/// The macOS panel: its class, its event handler and its build.
/// `panel_event!` requires a return type on each event, also `()`.
#[cfg(target_os = "macos")]
#[allow(clippy::unused_unit)]
mod panel {
    use tauri::{AppHandle, WebviewUrl};
    use tauri_nspanel::{CollectionBehavior, PanelBuilder, PanelLevel, StyleMask};
    use tauri_specta::Event as _;

    use super::{HEIGHT, WIDGET_WINDOW, WIDTH, WidgetPointer};

    /// Build the hidden panel, with a tracking area that sends
    /// `WidgetPointer` on pointer enter and exit (KTD6).
    pub(super) fn build(app: &AppHandle, url: WebviewUrl) -> tauri::Result<()> {
        let panel = PanelBuilder::<_, WidgetPanel>::new(app, WIDGET_WINDOW)
            .url(url)
            .title("Recording")
            .level(PanelLevel::Status)
            .size(tauri::Size::Logical(tauri::LogicalSize {
                width: WIDTH,
                height: HEIGHT,
            }))
            .has_shadow(false)
            .transparent(true)
            .no_activate(true)
            .corner_radius(0.0)
            .style_mask(StyleMask::empty().borderless().nonactivating_panel())
            .with_window(|w| {
                w.decorations(false)
                    .transparent(true)
                    .focusable(false)
                    .accept_first_mouse(true)
                    .zoom_hotkeys_enabled(false)
                    .content_protected(true)
                    .visible(false)
            })
            .collection_behavior(
                CollectionBehavior::new()
                    .can_join_all_spaces()
                    .full_screen_auxiliary(),
            )
            .build()?;
        // Hide (Cmd-H) does not hide the pill. A show of a window in a
        // hidden app unhides the app, so the sync would show the main
        // window again in one second.
        panel.as_panel().setCanHide(false);
        panel.hide();

        let events = WidgetPanelEvents::new();
        let entered = app.clone();
        events.on_mouse_entered(move |event| {
            let _ = pointer(event, true).emit(&entered);
        });
        let moved = app.clone();
        events.on_mouse_moved(move |event| {
            let _ = pointer(event, true).emit(&moved);
        });
        let exited = app.clone();
        events.on_mouse_exited(move |event| {
            let _ = pointer(event, false).emit(&exited);
        });
        panel.set_event_handler(Some(events.as_ref()));
        // The panel keeps a weak reference to its handler, and the window
        // lives as long as the app, so the handler does too.
        std::mem::forget(events);
        Ok(())
    }

    /// The pointer position in page pixels. AppKit counts from the bottom
    /// left of the window, and the page from the top left. The page has no
    /// interface zoom, so one page pixel is one point.
    fn pointer(event: &tauri_nspanel::objc2_app_kit::NSEvent, inside: bool) -> WidgetPointer {
        let location = event.locationInWindow();
        WidgetPointer {
            inside,
            x: location.x,
            y: HEIGHT - location.y,
        }
    }

    tauri_nspanel::tauri_panel! {
        panel!(WidgetPanel {
            config: {
                can_become_key_window: false,
                is_floating_panel: true
            }
            with: {
                tracking_area: {
                    options: tauri_nspanel::TrackingAreaOptions::new()
                        .active_always()
                        .mouse_entered_and_exited()
                        .mouse_moved(),
                    auto_resize: true
                }
            }
        })

        panel_event!(WidgetPanelEvents {
            window_did_resize(notification: &NSNotification) -> ()
        })
    }
}

/// The pointer entered, moved on or left the pill (KTD6). WebKit sends
/// hover events late to a panel that is never the key window, so a native
/// tracking area sends this on macOS, and the page sets the hover itself.
#[derive(serde::Serialize, Clone, specta::Type, tauri_specta::Event)]
pub struct WidgetPointer {
    pub inside: bool,
    /// The pointer position in page pixels, from the top left.
    pub x: f64,
    pub y: f64,
}

/// Build the hidden widget window at startup (KTD9).
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let url = WebviewUrl::App("index.html?window=widget".into());
    app.manage(MovedPlace::default());

    #[cfg(target_os = "macos")]
    panel::build(app, url)?;

    #[cfg(not(target_os = "macos"))]
    {
        tauri::WebviewWindowBuilder::new(app, WIDGET_WINDOW, url)
            .title("Recording")
            .inner_size(WIDTH, HEIGHT)
            .resizable(false)
            .shadow(false)
            .maximizable(false)
            .minimizable(false)
            .closable(false)
            .accept_first_mouse(true)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .transparent(true)
            .focusable(false)
            .focused(false)
            .zoom_hotkeys_enabled(false)
            .content_protected(true)
            .visible(false)
            .build()?;
    }

    Ok(())
}

/// Place the pill on its display and show it. Returns the display, which
/// the entry remembers (R5). Call on the main thread; the window never goes
/// through `window::front`, which would take the focus and show the Dock
/// icon (KTD9).
pub fn place_and_show(app: &AppHandle, remembered: Option<DisplayKey>) -> Option<DisplayKey> {
    let window = app.get_webview_window(WIDGET_WINDOW)?;
    let all = screens(app);
    let visible_on = if window.is_visible().unwrap_or(false) {
        window
            .current_monitor()
            .ok()
            .flatten()
            .map(|m| Screen::of(&m).key)
    } else {
        None
    };
    // Most syncs keep a visible pill where it is, and need no other display.
    let keeps = visible_on.is_some_and(|key| all.iter().any(|s| s.key == key));
    let (under_pointer, primary) = if keeps {
        (None, None)
    } else {
        pointer_and_primary(app)
    };
    let moved = app
        .try_state::<MovedPlace>()
        .and_then(|m| *m.0.lock().unwrap_or_else(|e| e.into_inner()));
    let (screen, x, y) =
        match placement(&all, visible_on, remembered, under_pointer, primary, moved)? {
            Placement::Keep(key) => return Some(key),
            Placement::At { screen, x, y } => (screen, x, y),
        };

    #[cfg(not(windows))]
    {
        // Logical points of the target display (KTD7).
        let position = tauri::LogicalPosition {
            x: f64::from(x) / screen.scale,
            y: f64::from(y) / screen.scale,
        };
        if let Err(e) = window.set_position(tauri::Position::Logical(position)) {
            log::error!("failed to place the recording widget: {e}");
        }
        if !window.is_visible().unwrap_or(false)
            && let Err(e) = window.show()
        {
            log::error!("failed to show the recording widget: {e}");
        }
    }

    #[cfg(windows)]
    {
        if !window.is_visible().unwrap_or(false)
            && let Err(e) = window.show()
        {
            log::error!("failed to show the recording widget: {e}");
        }
        // After the show: a move across displays makes tao reflow the window
        // for the new DPI, so the bounds and the topmost order are set last.
        place_windows(&window, &screen, x, y);
    }

    Some(screen.key)
}

/// Set the bounds and keep the window topmost without activating it (KTD4).
#[cfg(windows)]
fn place_windows(window: &tauri::WebviewWindow, screen: &Screen, x: i32, y: i32) {
    use windows::Win32::UI::WindowsAndMessaging::{HWND_TOPMOST, SWP_NOACTIVATE, SetWindowPos};

    let (_, _, width, height) = physical_bounds(screen, windows_text_scale_factor());
    let Ok(hwnd) = window.hwnd() else {
        log::error!("failed to get the recording widget window handle");
        return;
    };
    // SAFETY: `hwnd` is the live handle of the widget window, and this runs
    // on the main thread.
    let placed = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE,
        )
    };
    if let Err(e) = placed {
        log::error!("failed to place the recording widget: {e}");
    }
}

/// Hide the pill, and keep the place that the user dragged it to for the
/// next note. Call on the main thread.
pub fn hide(app: &AppHandle) -> Option<DisplayKey> {
    let window = app.get_webview_window(WIDGET_WINDOW)?;
    if !window.is_visible().unwrap_or(false) {
        return None;
    }
    let screen = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| Screen::of(&m));
    if let (Ok(position), Some(screen), Some(kept)) = (
        window.outer_position(),
        screen,
        app.try_state::<MovedPlace>(),
    ) {
        let place = moved_place(&screen, position.x, position.y);
        *kept.0.lock().unwrap_or_else(|e| e.into_inner()) = place;
    }
    if let Err(e) = window.hide() {
        log::error!("failed to hide the recording widget: {e}");
    }
    screen.map(|s| s.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, shown: bool) -> Option<Entry> {
        Some(Entry {
            session_id: id.into(),
            shown,
            display: None,
        })
    }

    #[test]
    fn a_new_note_reads_the_switch() {
        assert_eq!(next_entry(Some("a"), None, true), entry("a", true));
        assert_eq!(next_entry(Some("a"), None, false), entry("a", false));
    }

    // Covers R14: a switch change during a note does not change the note.
    #[test]
    fn the_same_note_keeps_its_choice_when_the_switch_changes() {
        assert_eq!(
            next_entry(Some("a"), entry("a", false), true),
            entry("a", false)
        );
        assert_eq!(
            next_entry(Some("a"), entry("a", true), false),
            entry("a", true)
        );
    }

    // Covers AE9: an interruption and a resume keep the session open, so a
    // hidden pill stays hidden.
    #[test]
    fn an_interruption_and_a_resume_keep_a_hidden_pill_hidden() {
        let interrupted = next_entry(Some("a"), entry("a", false), true);
        assert_eq!(next_entry(Some("a"), interrupted, true), entry("a", false));
    }

    #[test]
    fn the_same_note_keeps_its_display() {
        let display = Some(LAPTOP.key);
        let kept = next_entry(
            Some("a"),
            Some(Entry {
                session_id: "a".into(),
                shown: true,
                display,
            }),
            true,
        );
        assert_eq!(kept.unwrap().display, display);
    }

    // Covers R16: Stop clears the entry, so Resume of the stopped note is a
    // new start.
    #[test]
    fn stop_clears_the_entry_and_resume_reads_the_switch_again() {
        let stopped = next_entry(None, entry("a", false), true);
        assert_eq!(stopped, None);
        assert_eq!(next_entry(Some("a"), stopped, true), entry("a", true));
    }

    #[test]
    fn a_second_note_replaces_the_entry_and_reads_the_switch() {
        assert_eq!(
            next_entry(Some("b"), entry("a", false), true),
            entry("b", true)
        );
    }

    // Covers AE8 and R15: a press with no note open keeps nothing for the
    // next note.
    #[test]
    fn a_press_with_no_open_note_does_nothing() {
        assert_eq!(press_action(None), PressAction::None);
        assert_eq!(next_entry(Some("a"), None, true), entry("a", true));
    }

    #[test]
    fn a_press_with_an_open_note_toggles_the_pill() {
        assert_eq!(press_action(entry("a", true).as_ref()), PressAction::Hide);
        assert_eq!(press_action(entry("a", false).as_ref()), PressAction::Show);
    }

    fn shown(entry: &Option<Entry>) -> Option<bool> {
        entry.as_ref().map(|e| e.shown)
    }

    #[test]
    fn a_recording_note_with_the_switch_on_shows_the_pill_and_stop_hides_it() {
        let started = step(Some("a"), None, true, Request::Sync);
        assert_eq!(shown(&started), Some(true));
        assert_eq!(step(None, started, true, Request::Sync), None);
    }

    // Covers AE4: the × hides the pill for the note, a pause and a continue
    // keep it hidden, and the next note shows it.
    #[test]
    fn the_x_hides_the_pill_until_the_note_stops() {
        let started = step(Some("a"), None, true, Request::Sync);
        let hidden = step(Some("a"), started, true, Request::Hide("a".into()));
        assert_eq!(shown(&hidden), Some(false));
        let paused = step(Some("a"), hidden, true, Request::Sync);
        let continued = step(Some("a"), paused, true, Request::Sync);
        assert_eq!(shown(&continued), Some(false));
        let stopped = step(None, continued, true, Request::Sync);
        let next = step(Some("b"), stopped, true, Request::Sync);
        assert_eq!(shown(&next), Some(true));
    }

    // Covers AE5: with the switch off, Show/Hide shows the pill for this
    // note only.
    #[test]
    fn show_hide_with_the_switch_off_shows_the_pill_for_this_note_only() {
        let started = step(Some("a"), None, false, Request::Sync);
        assert_eq!(shown(&started), Some(false));
        let toggled = step(Some("a"), started, false, Request::Toggle("a".into()));
        assert_eq!(shown(&toggled), Some(true));
        let stopped = step(None, toggled, false, Request::Sync);
        let next = step(Some("b"), stopped, false, Request::Sync);
        assert_eq!(shown(&next), Some(false));
    }

    // Covers R15: Show/Hide and the × with no open note change nothing.
    #[test]
    fn a_choice_with_no_open_note_changes_nothing() {
        assert_eq!(step(None, None, true, Request::Toggle("a".into())), None);
        assert_eq!(step(None, None, true, Request::Hide("a".into())), None);
    }

    // A Hide or a Show/Hide sent for a note that stopped before the widget
    // thread read it must not change the next note.
    #[test]
    fn a_choice_for_another_note_changes_nothing() {
        let started = step(Some("b"), None, true, Request::Sync);
        let hidden = step(Some("b"), started.clone(), true, Request::Hide("a".into()));
        assert_eq!(hidden, started);
        let toggled = step(
            Some("b"),
            started.clone(),
            true,
            Request::Toggle("a".into()),
        );
        assert_eq!(toggled, started);
        // A choice that arrives with the first sync of the next note too.
        let first = step(Some("b"), None, true, Request::Hide("a".into()));
        assert_eq!(shown(&first), Some(true));
    }

    // `capture::changed` asks for a sync while it holds the capture lock.
    // The request returns at once, and the sync runs after the lock goes.
    #[test]
    fn a_request_under_the_capture_lock_does_not_wait_for_the_sync() {
        use std::sync::{Arc, Mutex, mpsc};

        let capture_lock = Arc::new(Mutex::new(()));
        let (sender, receiver) = mpsc::channel();
        let requests = Requests(sender);
        let worker_lock = Arc::clone(&capture_lock);
        let worker = std::thread::spawn(move || {
            let request = receiver.recv().unwrap();
            let _held = worker_lock.lock().unwrap();
            request
        });
        {
            let _held = capture_lock.lock().unwrap();
            requests.send(Request::Sync);
        }
        assert_eq!(worker.join().unwrap(), Request::Sync);
    }

    // A note that a crash interrupted is open at launch, so the first sync
    // shows the pill when the switch is on.
    #[test]
    fn an_interrupted_note_at_launch_shows_the_pill() {
        assert_eq!(
            shown(&step(Some("a"), None, true, Request::Sync)),
            Some(true)
        );
    }

    /// A Retina laptop display at the origin with a 70-pixel Dock.
    const LAPTOP: Screen = Screen {
        key: DisplayKey {
            x: 0,
            y: 0,
            width: 3024,
            height: 1964,
        },
        work_bottom: 1964 - 70,
        scale: 2.0,
    };

    /// An external display to the right of the laptop, with a taskbar.
    const EXTERNAL: Screen = Screen {
        key: DisplayKey {
            x: 3024,
            y: 0,
            width: 1920,
            height: 1080,
        },
        work_bottom: 1080 - 48,
        scale: 1.0,
    };

    #[test]
    fn the_remembered_display_wins_while_it_is_there() {
        let screens = [LAPTOP, EXTERNAL];
        let chosen = choose_screen(&screens, Some(LAPTOP.key), Some(EXTERNAL.key), None);
        assert_eq!(chosen, Some(LAPTOP));
    }

    // Covers AE3: the laptop display goes away, and the pill moves to the
    // display under the pointer.
    #[test]
    fn a_gone_display_gives_the_display_under_the_pointer() {
        let chosen = choose_screen(&[EXTERNAL], Some(LAPTOP.key), Some(EXTERNAL.key), None);
        assert_eq!(chosen, Some(EXTERNAL));
    }

    #[test]
    fn with_no_display_under_the_pointer_the_primary_display_is_used() {
        let chosen = choose_screen(&[LAPTOP, EXTERNAL], None, None, Some(EXTERNAL.key));
        assert_eq!(chosen, Some(EXTERNAL));
    }

    #[test]
    fn displays_match_by_bounds_not_by_order() {
        let moved = Screen {
            key: DisplayKey {
                x: -1920,
                ..EXTERNAL.key
            },
            ..EXTERNAL
        };
        let chosen = choose_screen(&[moved, LAPTOP], Some(EXTERNAL.key), None, None);
        assert_eq!(
            chosen,
            Some(moved),
            "the old bounds are gone, so the first display"
        );
    }

    #[test]
    fn the_logical_origin_is_the_bottom_centre_of_the_work_area() {
        // 1512 x 982 points; the work area ends at 947 points.
        assert_eq!(logical_origin(&LAPTOP), (612.0, 947.0 - 64.0 - 12.0));
        assert_eq!(logical_origin(&EXTERNAL), (3024.0 + 816.0, 1032.0 - 76.0));
    }

    // The 1-second sync must not move a pill that the user dragged.
    #[test]
    fn a_visible_pill_on_a_display_that_is_there_stays_where_it_is() {
        let placed = placement(
            &[LAPTOP, EXTERNAL],
            Some(EXTERNAL.key),
            Some(LAPTOP.key),
            None,
            None,
            None,
        );
        assert_eq!(placed, Some(Placement::Keep(EXTERNAL.key)));
    }

    #[test]
    fn a_visible_pill_whose_display_is_gone_goes_to_the_default_place() {
        let placed = placement(&[LAPTOP], Some(EXTERNAL.key), None, None, None, None);
        let (x, y) = default_position(&LAPTOP);
        assert_eq!(
            placed,
            Some(Placement::At {
                screen: LAPTOP,
                x,
                y
            })
        );
    }

    // A hide records the display the pill is on (sync sets the entry's
    // display from it), so a show for the same note after a drag to another
    // display goes to the dragged place. A note whose display is another
    // one gets the default place of its own display.
    #[test]
    fn a_show_uses_the_dragged_place_only_on_the_note_display() {
        let moved = Moved {
            display: EXTERNAL.key,
            x: 3100,
            y: 40,
        };
        let same = placement(
            &[LAPTOP, EXTERNAL],
            None,
            Some(EXTERNAL.key),
            None,
            None,
            Some(moved),
        );
        assert_eq!(
            same,
            Some(Placement::At {
                screen: EXTERNAL,
                x: 3100,
                y: 40
            })
        );
        let other = placement(
            &[LAPTOP, EXTERNAL],
            None,
            Some(LAPTOP.key),
            None,
            None,
            Some(moved),
        );
        let (x, y) = default_position(&LAPTOP);
        assert_eq!(
            other,
            Some(Placement::At {
                screen: LAPTOP,
                x,
                y
            })
        );
    }

    #[test]
    fn a_pill_that_shows_goes_to_the_place_the_user_dragged_it_to() {
        let moved = Moved {
            display: EXTERNAL.key,
            x: 3100,
            y: 40,
        };
        let placed = placement(
            &[LAPTOP, EXTERNAL],
            None,
            None,
            Some(LAPTOP.key),
            None,
            Some(moved),
        );
        assert_eq!(
            placed,
            Some(Placement::At {
                screen: EXTERNAL,
                x: 3100,
                y: 40
            })
        );
    }

    #[test]
    fn a_dragged_place_on_a_display_that_is_gone_is_not_used() {
        let moved = Moved {
            display: EXTERNAL.key,
            x: 3100,
            y: 40,
        };
        let placed = placement(&[LAPTOP], None, None, None, None, Some(moved));
        let (x, y) = default_position(&LAPTOP);
        assert_eq!(
            placed,
            Some(Placement::At {
                screen: LAPTOP,
                x,
                y
            })
        );
    }

    #[test]
    fn a_pill_at_its_default_place_keeps_no_dragged_place() {
        let (x, y) = default_position(&LAPTOP);
        assert_eq!(moved_place(&LAPTOP, x + 1, y - 2), None);
        assert_eq!(
            moved_place(&LAPTOP, x + 200, y),
            Some(Moved {
                display: LAPTOP.key,
                x: x + 200,
                y
            })
        );
    }

    #[test]
    fn the_physical_bounds_are_the_bottom_centre_of_the_work_area() {
        assert_eq!(
            physical_bounds(&EXTERNAL, 1.0),
            (3024 + 816, 1032 - 76, 288, 64)
        );
        assert_eq!(
            physical_bounds(&LAPTOP, 1.0),
            (1224, 1894 - 128 - 24, 576, 128)
        );
    }

    #[test]
    fn a_text_scale_grows_the_window_and_keeps_its_bottom_edge() {
        let (_, y, _, height) = physical_bounds(&EXTERNAL, 1.0);
        let bottom = y + height;
        let (x, y, width, height) = physical_bounds(&EXTERNAL, 1.5);
        assert_eq!((x, y, width, height), (3024 + 744, 924, 432, 96));
        assert_eq!(y + height, bottom);
        // The offset under the window grows with the display scale only.
        assert_eq!(
            physical_bounds(&LAPTOP, 1.25),
            (1152, 1894 - 160 - 24, 720, 160)
        );
    }

    #[test]
    fn the_text_scale_is_clamped_and_is_one_with_no_value() {
        assert_eq!(text_scale_of(None), 1.0);
        assert_eq!(text_scale_of(Some(100)), 1.0);
        assert_eq!(text_scale_of(Some(150)), 1.5);
        assert_eq!(text_scale_of(Some(80)), 1.0);
        assert_eq!(text_scale_of(Some(225)), 2.25);
        assert_eq!(text_scale_of(Some(300)), 2.25);
    }

    // A move from a scale-2 display to a scale-1 display uses the scale of
    // the target display.
    #[test]
    fn a_move_between_scales_uses_the_target_scale() {
        let (x, _) = logical_origin(&EXTERNAL);
        assert_eq!(x, 3024.0 + (1920.0 - 288.0) / 2.0);
    }
}
