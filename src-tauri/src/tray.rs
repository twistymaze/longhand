//! Tray icon and menu. The builder shape and the Windows click convention are
//! Handy's (`lib.rs` tray setup); see src-tauri/PROVENANCE.md.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use crate::window::show_main_window;

const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");

pub fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Longhand", true, None::<&str>)?;
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
