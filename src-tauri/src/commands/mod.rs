pub mod alerts;
pub mod build_info;
pub mod capture;
pub mod licences;
pub mod models;
pub mod processing;
pub mod readiness;
pub mod session;
pub mod settings;
pub mod updates;
pub mod webview_log;
pub mod widget;
pub mod window;

/// Run `f` on a blocking thread and wait for its result. A command that
/// can wait for a lock, the capture state or a folder runs its work here,
/// so the wait does not stop the main thread or a worker of the async
/// runtime.
pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}
