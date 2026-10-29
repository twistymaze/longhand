//! The update window's commands (KTD9). The window shows the state and
//! sends the user's choice. The update manager does the work.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::commands::changelog::ChangelogEntry;
use crate::managers::updates::{Choice, UpdateManager, UpdateStateChanged};

/// The current update state, which the window reads once at open, and
/// whether the window has the focus.
#[tauri::command]
#[specta::specta]
pub fn update_state(manager: State<'_, Arc<UpdateManager>>) -> UpdateStateChanged {
    manager.snapshot()
}

/// "Check for Updates…" (R11).
#[tauri::command]
#[specta::specta]
pub fn check_for_updates(manager: State<'_, Arc<UpdateManager>>) {
    manager.check_now();
}

/// The version that the tray's update item offers, which the library
/// footer shows. `UpdateOfferChanged` sends each change.
#[tauri::command]
#[specta::specta]
pub fn update_offer(manager: State<'_, Arc<UpdateManager>>) -> Option<String> {
    manager.offer()
}

/// The footer's update badge: the same action as the tray's update item.
#[tauri::command]
#[specta::specta]
pub fn open_update_offer(manager: State<'_, Arc<UpdateManager>>) {
    manager.tray_open();
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

/// The changes that the offered `version` gives, which the update window
/// lists (R13). None when the app cannot get them (R14): the window then
/// shows the link to the full changelog. Async, because it can download.
#[tauri::command]
#[specta::specta]
pub async fn offered_changes(app: AppHandle, version: String) -> Option<Vec<ChangelogEntry>> {
    let manager = app.state::<Arc<UpdateManager>>().inner().clone();
    manager.offered_changes(&version).await
}
