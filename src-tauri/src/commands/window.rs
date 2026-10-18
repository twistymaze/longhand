//! Window commands (KTD3).

use tauri::{AppHandle, WebviewWindow};

use crate::window::SettingsPane;

/// Show the Settings window, and build it first when it is not open. With a
/// pane, the window shows that pane (KTD10). Async because a synchronous
/// command that builds a window deadlocks on Windows.
#[tauri::command]
#[specta::specta]
pub async fn open_settings(app: AppHandle, pane: Option<SettingsPane>) {
    crate::window::open_settings_on(&app, pane);
}

/// Show the window with the full licence list, and build it first when it
/// is not open. Async for the same reason as `open_settings`.
#[tauri::command]
#[specta::specta]
pub async fn open_licences(app: AppHandle) {
    crate::window::open_licences(&app);
}

/// Destroy the Settings window that calls this. The page calls this when its
/// own destroy call fails, so that the window still closes. The call needs
/// no window permission, and it does not send a close request that a
/// listener can stop. It acts only on the calling window, so a late call
/// cannot close a Settings window that opened after it.
#[tauri::command]
#[specta::specta]
pub async fn close_settings_window(window: WebviewWindow) -> Result<(), String> {
    if window.label() != crate::window::SETTINGS_WINDOW {
        return Err(format!(
            "the {} window is not the Settings window",
            window.label()
        ));
    }
    window.destroy().map_err(|e| e.to_string())
}
