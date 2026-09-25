//! Window commands (KTD3).

use tauri::AppHandle;

/// Show the Settings window, and build it first when it is not open. Async
/// because a synchronous command that builds a window deadlocks on Windows.
#[tauri::command]
#[specta::specta]
pub async fn open_settings(app: AppHandle) {
    crate::window::open_settings(&app);
}

/// Destroy the Settings window. The page calls this when its own destroy
/// call fails, so that the window still closes. The call needs no window
/// permission, and it does not send a close request that a listener can
/// stop.
#[tauri::command]
#[specta::specta]
pub async fn close_settings_window(app: AppHandle) -> Result<(), String> {
    crate::window::destroy_settings_window(&app).map_err(|e| e.to_string())
}
