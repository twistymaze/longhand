//! The macOS app menu (KTD3). The menu holds "Check for Updates…",
//! "Settings…" (⌘,) and the standard Edit submenu: without the Edit items,
//! copy, paste and undo do not work in the app's text fields.

use tauri::AppHandle;
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};

use crate::commands::build_info::build_info;
use crate::window::open_settings;

/// The menu ID of the app menu's Settings item. The tray menu uses its own
/// IDs, so a menu event handler never acts on the other menu's item.
const SETTINGS: &str = "app-settings";
const CHECK_FOR_UPDATES: &str = "app-check-updates";

/// The data for the standard macOS About panel (R9–R13 of the About Longhand
/// plan). AppKit shows "Version <version> (<short version>)", so the short
/// version holds the commit, as in the About pane. The credits text is plain
/// text, so the website address cannot be a link there.
fn about_metadata(version: &str, commit: Option<String>) -> AboutMetadata<'static> {
    AboutMetadata {
        version: Some(version.to_string()),
        short_version: Some(commit.unwrap_or_else(|| "local build".to_string())),
        copyright: Some("Copyright © 2026 Erv Walter".to_string()),
        credits: Some(
            "Longhand is a note-taking app for meetings and conversations.\n\nlonghand.md".to_string(),
        ),
        ..Default::default()
    }
}

/// Set the app menu, and open Settings when the user selects its item.
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
    app.on_menu_event(|app, event| match event.id.as_ref() {
        SETTINGS => open_settings(app),
        CHECK_FOR_UPDATES => crate::managers::updates::check_for_updates(app),
        _ => {}
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_shows_the_version_and_the_commit() {
        let m = about_metadata("1.0.4", Some("abc1234".to_string()));
        assert_eq!(m.version.as_deref(), Some("1.0.4"));
        assert_eq!(m.short_version.as_deref(), Some("abc1234"));
    }

    #[test]
    fn a_build_with_no_commit_says_local_build() {
        let m = about_metadata("0.1.0", None);
        assert_eq!(m.short_version.as_deref(), Some("local build"));
    }

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
