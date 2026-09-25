//! Log commands: the webview writes its errors and close steps to the app
//! log (Longhand.log). The webview console does not reach the log.

use serde::Deserialize;
use specta::Type;
use tauri::Webview;

/// The log target of the lines that the webview sends.
pub const WEBVIEW_LOG_TARGET: &str = "webview";

/// The longest message that one call writes, in characters. A longer message
/// is cut, so that one large value cannot fill the log.
const MESSAGE_LIMIT: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WebviewLogLevel {
    Error,
    Warn,
    Info,
}

impl From<WebviewLogLevel> for log::Level {
    fn from(level: WebviewLogLevel) -> Self {
        match level {
            WebviewLogLevel::Error => log::Level::Error,
            WebviewLogLevel::Warn => log::Level::Warn,
            WebviewLogLevel::Info => log::Level::Info,
        }
    }
}

/// The log line for `message` from the webview `label`: the label first,
/// and the message cut at `MESSAGE_LIMIT` characters.
fn log_line(label: &str, message: &str) -> String {
    match message.char_indices().nth(MESSAGE_LIMIT) {
        Some((end, _)) => format!("{label}: {} [cut]", &message[..end]),
        None => format!("{label}: {message}"),
    }
}

/// Write one line from the webview to the app log. Async, so that the line
/// does not wait for the main thread.
#[tauri::command]
#[specta::specta]
pub async fn log_from_webview(webview: Webview, level: WebviewLogLevel, message: String) {
    log::log!(
        target: WEBVIEW_LOG_TARGET,
        level.into(),
        "{}",
        log_line(webview.label(), &message)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_starts_with_the_webview_label() {
        assert_eq!(
            log_line("settings", "close requested"),
            "settings: close requested"
        );
    }

    #[test]
    fn a_long_message_is_cut_at_the_limit_in_characters() {
        let message = "é".repeat(MESSAGE_LIMIT + 10);
        let line = log_line("main", &message);
        assert_eq!(line, format!("main: {} [cut]", "é".repeat(MESSAGE_LIMIT)));
    }

    #[test]
    fn the_levels_map_to_the_log_levels() {
        assert_eq!(log::Level::from(WebviewLogLevel::Error), log::Level::Error);
        assert_eq!(log::Level::from(WebviewLogLevel::Warn), log::Level::Warn);
        assert_eq!(log::Level::from(WebviewLogLevel::Info), log::Level::Info);
    }
}
