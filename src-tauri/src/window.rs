//! Main window lifecycle: closing hides rather than exits (KTD11).
//! Adapted from Handy's `lib.rs` (`show_main_window`, the `CloseRequested`
//! handler); see src-tauri/PROVENANCE.md.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};

pub const MAIN_WINDOW: &str = "main";

pub fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::App("/".into()))
        .title("Longhand")
        .inner_size(720.0, 600.0)
        .min_inner_size(560.0, 480.0)
        .visible(false)
        .build()?;
    Ok(())
}

pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        log::error!("main window not found");
        return;
    };
    if let Err(e) = window.unminimize() {
        log::error!("failed to unminimize main window: {e}");
    }
    if let Err(e) = window.show() {
        log::error!("failed to show main window: {e}");
    }
    if let Err(e) = window.set_focus() {
        log::error!("failed to focus main window: {e}");
    }
    // Hiding demoted the app to Accessory (no Dock icon); showing promotes it
    // back, which is the direction Apple supports at runtime.
    #[cfg(target_os = "macos")]
    if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
        log::error!("failed to set activation policy to Regular: {e}");
    }
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if window.label() != MAIN_WINDOW {
            return;
        }
        // The process stays alive in the tray. This is an optimization only:
        // processing survives a real quit through checkpoints, not through this.
        // `RunEvent::ExitRequested` + `prevent_exit()` is deliberately not used
        // (tauri-apps/tauri#13511: it makes the app unkillable).
        api.prevent_close();
        if let Err(e) = window.hide() {
            log::error!("failed to hide main window: {e}");
        }
        #[cfg(target_os = "macos")]
        if let Err(e) = window
            .app_handle()
            .set_activation_policy(tauri::ActivationPolicy::Accessory)
        {
            log::error!("failed to set activation policy to Accessory: {e}");
        }
    }
}
