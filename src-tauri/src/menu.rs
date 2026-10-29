//! The app menus. The macOS app menu (KTD3) holds "Check for Updates…",
//! "Settings…" (⌘,) and the standard Edit submenu: without the Edit items,
//! copy, paste and undo do not work in the app's text fields. On Windows,
//! the main window has a menu bar with File and Help, so that Settings and
//! "Check for Updates…" show without the tray or a shortcut.

use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tauri::menu::AboutMetadata;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};

#[cfg(target_os = "macos")]
use crate::commands::build_info::build_info;
use crate::window::{SettingsPane, open_settings, open_settings_on};

/// The menu ID of the app menu's Settings item. The tray menu uses its own
/// IDs, so a menu event handler never acts on the other menu's item.
const SETTINGS: &str = "app-settings";
const CHECK_FOR_UPDATES: &str = "app-check-updates";
/// The About item of the Windows menu bar. It opens the About pane of
/// Settings.
const ABOUT: &str = "app-about";
const EXIT: &str = "app-exit";

/// The data for the standard macOS About panel (R9–R13 of the About Longhand
/// plan). AppKit shows "Version <version> (<short version>)", so the short
/// version holds the commit, as in the About pane. The credits text is plain
/// text, so the website address cannot be a link there.
#[cfg(target_os = "macos")]
fn about_metadata(version: &str, commit: Option<String>) -> AboutMetadata<'static> {
    AboutMetadata {
        version: Some(version.to_string()),
        short_version: Some(commit.unwrap_or_else(|| "local build".to_string())),
        copyright: Some("Copyright © 2026 Erv Walter".to_string()),
        credits: Some(
            "Longhand is a note-taking app for meetings and conversations.\n\nlonghand.md"
                .to_string(),
        ),
        ..Default::default()
    }
}

/// Set the macOS app menu.
#[cfg(target_os = "macos")]
pub fn set_app_menu(app: &AppHandle) -> tauri::Result<()> {
    let name = app.package_info().name.clone();
    let info = build_info(app.clone());
    let about = about_metadata(&info.version, info.commit);
    let app_submenu = Submenu::with_items(
        app,
        &name,
        true,
        &[
            &PredefinedMenuItem::about(app, Some(&format!("About {name}")), Some(about))?,
            &MenuItem::with_id(
                app,
                CHECK_FOR_UPDATES,
                "Check for Updates…",
                true,
                None::<&str>,
            )?,
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
    Ok(())
}

/// The submenus of the Windows menu bar: each title, with the IDs of its
/// items in order. `None` is a separator.
#[cfg(any(target_os = "windows", test))]
fn windows_menu_layout() -> [(&'static str, &'static [Option<&'static str>]); 2] {
    [
        ("&File", &[Some(SETTINGS), None, Some(EXIT)]),
        ("&Help", &[Some(CHECK_FOR_UPDATES), None, Some(ABOUT)]),
    ]
}

/// The text and the shortcut of an item of the Windows menu bar.
#[cfg(any(target_os = "windows", test))]
fn windows_item(id: &str) -> (&'static str, Option<&'static str>) {
    match id {
        SETTINGS => ("&Settings…", Some("Ctrl+,")),
        EXIT => ("E&xit", None),
        CHECK_FOR_UPDATES => ("&Check for Updates…", None),
        ABOUT => ("&About Longhand", None),
        _ => unreachable!("the Windows menu bar has no item {id}"),
    }
}

/// The menu bar of the Windows main window. Only the main window gets it,
/// so the Settings window and the recording widget have no menu bar.
#[cfg(target_os = "windows")]
pub fn main_window_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for (title, ids) in windows_menu_layout() {
        let submenu = Submenu::new(app, title, true)?;
        for id in ids {
            match id {
                Some(id) => {
                    let (text, shortcut) = windows_item(id);
                    submenu.append(&MenuItem::with_id(app, *id, text, true, shortcut)?)?;
                }
                None => submenu.append(&PredefinedMenuItem::separator(app)?)?,
            }
        }
        menu.append(&submenu)?;
    }
    Ok(menu)
}

/// Act on the items of the macOS app menu and the Windows menu bar.
pub fn handle_menu_events(app: &AppHandle) {
    app.on_menu_event(|app, event| match event.id.as_ref() {
        SETTINGS => open_settings(app),
        CHECK_FOR_UPDATES => crate::managers::updates::check_for_updates(app),
        ABOUT => open_settings_on(app, Some(SettingsPane::About)),
        EXIT => app.exit(0),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_menu_bar_offers_settings_and_check_for_updates() {
        let ids: Vec<&str> = windows_menu_layout()
            .iter()
            .flat_map(|(_, ids)| ids.iter().flatten().copied())
            .collect();
        assert_eq!(ids, [SETTINGS, EXIT, CHECK_FOR_UPDATES, ABOUT]);
        // Each item has a text: `windows_item` panics on an unknown ID.
        for id in ids {
            windows_item(id);
        }
        assert_eq!(windows_item(SETTINGS), ("&Settings…", Some("Ctrl+,")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_panel_shows_the_version_and_the_commit() {
        let m = about_metadata("1.0.4", Some("abc1234".to_string()));
        assert_eq!(m.version.as_deref(), Some("1.0.4"));
        assert_eq!(m.short_version.as_deref(), Some("abc1234"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_build_with_no_commit_says_local_build() {
        let m = about_metadata("0.1.0", None);
        assert_eq!(m.short_version.as_deref(), Some("local build"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_panel_shows_the_copyright_the_description_and_the_address() {
        let m = about_metadata("1.0.4", None);
        assert_eq!(m.copyright.as_deref(), Some("Copyright © 2026 Erv Walter"));
        assert_eq!(
            m.credits.as_deref(),
            Some("Longhand is a note-taking app for meetings and conversations.\n\nlonghand.md")
        );
    }
}
