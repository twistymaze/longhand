//! Commands of the recording widget page (KTD11).

use tauri::AppHandle;

use crate::widget::{self, Request};

/// The × on the pill: hide it until the note stops (R12). The note does not
/// change. `session_id` is the note that the pill showed when the user
/// clicked, so a click does not hide the pill of a later note.
#[tauri::command]
#[specta::specta]
pub fn widget_hide(app: AppHandle, session_id: String) {
    widget::request(&app, Request::Hide(session_id));
}

/// A click on the pill outside its controls: show the main window with the
/// open note in front, also when the window is closed or hidden (R10). The
/// tray's open path does the same. Async, because a command that shows a
/// window must not run on the main thread.
#[tauri::command(async)]
#[specta::specta]
pub fn widget_open_note(app: AppHandle) {
    crate::tray::open_longhand(&app);
}
