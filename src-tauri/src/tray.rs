//! Tray icon and menu. The builder shape and the Windows click convention are
//! Handy's (`lib.rs` tray setup); see src-tauri/PROVENANCE.md.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use crate::window::show_main_window;

const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");
const OPEN: &str = "Open Longhand";

/// The menu item that fronts the window, renamed while an alert is raised.
struct OpenItem(MenuItem<tauri::Wry>);

/// Show a failed-capture alert on the tray, or clear it (U15): a title
/// beside the menu-bar icon on macOS, the tooltip, and the menu's first
/// item, which fronts the window on the Resume and Stop controls.
pub fn set_alert(app: &AppHandle, headline: Option<&str>) {
    if let Some(tray) = app.tray_by_id("main") {
        // An empty title, not `None`: tray-icon ignores `None` on macOS and
        // would leave "Interrupted" in the menu bar after the alert ended.
        #[cfg(target_os = "macos")]
        let _ = tray.set_title(Some(if headline.is_some() {
            "Interrupted"
        } else {
            ""
        }));
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

pub fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", OPEN, true, None::<&str>)?;
    app.manage(OpenItem(open.clone()));
    let quit = MenuItem::with_id(app, "quit", "Quit Longhand", true, Some("CmdOrCtrl+Q"))?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &separator, &quit])?;

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
                    show_main_window(tray.app_handle());
                }
            });
    }
    #[cfg(not(target_os = "windows"))]
    {
        builder = builder.show_menu_on_left_click(true);
    }

    let tray = builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    app.manage(tray);
    Ok(())
}
