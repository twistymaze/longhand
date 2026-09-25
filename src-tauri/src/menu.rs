//! The macOS app menu (KTD3). The menu holds "Settings…" (⌘,) and the
//! standard Edit submenu: without the Edit items, copy, paste and undo do
//! not work in the app's text fields.

use tauri::AppHandle;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};

use crate::window::open_settings;

/// The menu ID of the app menu's Settings item. The tray menu uses its own
/// IDs, so a menu event handler never acts on the other menu's item.
const SETTINGS: &str = "app-settings";

/// Set the app menu, and open Settings when the user selects its item.
pub fn set_app_menu(app: &AppHandle) -> tauri::Result<()> {
    let name = app.package_info().name.clone();
    let app_submenu = Submenu::with_items(
        app,
        &name,
        true,
        &[
            &PredefinedMenuItem::about(app, Some(&format!("About {name}")), None)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, SETTINGS, "Settings…", true, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let edit_submenu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window_submenu = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    let menu = Menu::with_items(app, &[&app_submenu, &edit_submenu, &window_submenu])?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id.as_ref() == SETTINGS {
            open_settings(app);
        }
    });
    Ok(())
}
