//! Window commands (KTD3).

use tauri::AppHandle;

/// Show the Settings window, and build it first when it is not open. Async
/// because a synchronous command that builds a window deadlocks on Windows.
#[tauri::command]
#[specta::specta]
pub async fn open_settings(app: AppHandle) {
    crate::window::open_settings(&app);
}
