//! The update window's commands (KTD9). The window shows the state and
//! sends the user's choice. The update manager does the work.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::managers::updates::{Choice, UpdateManager, UpdateState};

/// The current update state, which the window reads once at open.
#[tauri::command]
#[specta::specta]
pub fn update_state(manager: State<'_, Arc<UpdateManager>>) -> UpdateState {
    manager.state()
}

/// "Check for Updates…" (R11).
#[tauri::command]
#[specta::specta]
pub fn check_for_updates(manager: State<'_, Arc<UpdateManager>>) {
    manager.check_now();
}

/// The update window's page has set the size that fits its content, so the
/// window can show. Async because a synchronous command that changes a
/// window can deadlock on Windows.
#[tauri::command]
#[specta::specta]
pub async fn show_update_window(app: AppHandle) {
    crate::window::show_built_update_window(&app);
}

/// A button in the update window, or a close of the window.
#[tauri::command]
#[specta::specta]
pub fn choose_update(manager: State<'_, Arc<UpdateManager>>, choice: Choice) {
    manager.choose(choice);
}
