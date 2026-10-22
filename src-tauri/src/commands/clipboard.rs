//! Clipboard command: the webview writes rich text to the clipboard
//! through the native pasteboard.
//!
//! WebKit changes each HTML item that a page writes to the clipboard. It
//! renders the HTML and adds the computed styles to each element. In dark
//! mode the text colour is white, so a paste into a light email shows white
//! text on a white background. The native pasteboard keeps the HTML as the
//! app makes it, so the paste takes the styles of the receiving app.

/// Write `text` and `html` to the clipboard as one item, and nothing else.
/// False when the platform has no native write; the webview then writes
/// the clipboard itself.
#[tauri::command]
#[specta::specta]
pub fn write_clipboard(text: String, html: String) -> bool {
    write(&text, &html)
}

/// The HTML for the pasteboard. AppKit reads HTML with no charset as
/// Windows-1252, so `é` pastes as `Ã©`; the meta element sets UTF-8.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn pasteboard_html(html: &str) -> String {
    format!("<meta charset=\"utf-8\">{html}")
}

#[cfg(target_os = "macos")]
fn write(text: &str, html: &str) -> bool {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: the type constants are static strings that AppKit defines.
    let (html_type, text_type) = unsafe { (NSPasteboardTypeHTML, NSPasteboardTypeString) };
    pasteboard.setString_forType(&NSString::from_str(&pasteboard_html(html)), html_type)
        && pasteboard.setString_forType(&NSString::from_str(text), text_type)
}

#[cfg(not(target_os = "macos"))]
fn write(_text: &str, _html: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::pasteboard_html;

    #[test]
    fn pasteboard_html_declares_utf8() {
        assert_eq!(
            pasteboard_html("<p>Café</p>"),
            "<meta charset=\"utf-8\"><p>Café</p>"
        );
    }
}
