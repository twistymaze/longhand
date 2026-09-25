//! Window commands (KTD3).

use tauri::{AppHandle, WebviewWindow};

/// Show the Settings window, and build it first when it is not open. Async
/// because a synchronous command that builds a window deadlocks on Windows.
#[tauri::command]
#[specta::specta]
pub async fn open_settings(app: AppHandle) {
    crate::window::open_settings(&app);
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
