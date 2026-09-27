//! Conflict copies (KTD5, R15): a file or folder that a sync service made
//! when two computers changed one note. The name is the real name plus a
//! suffix:
//!
//! - `notes (1)` (Google Drive and others) and
//!   `notes (Erv's conflicted copy 2026-09-25)` (Dropbox). Longhand's own
//!   `notes (Longhand conflict 2026-09-25 1015)` has this form too.
//! - `notes-DESKTOP-ABC` (OneDrive adds the computer name).
//! - `notes 2` (iCloud).

/// The four documents of a note folder.
pub const DOCUMENTS: [&str; 4] = ["session.md", "transcript.md", "summary.md", "notes.md"];

/// Whether `name` is a conflict copy of `base`: `base` plus one suffix of
/// the forms above. Both are file names without the `.md` extension, or
/// folder names.
pub fn is_copy_name(name: &str, base: &str) -> bool {
    let Some(rest) = name.strip_prefix(base) else {
        return false;
    };
    if let Some(inner) = rest.strip_prefix(" (").and_then(|r| r.strip_suffix(')')) {
        return !inner.is_empty();
    }
    if let Some(digits) = rest.strip_prefix(' ') {
        return !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit());
    }
    if let Some(computer) = rest.strip_prefix('-') {
        return !computer.is_empty() && computer.chars().all(|c| c.is_alphanumeric() || c == '-');
    }
    false
}

/// The document that the file `file_name` is a conflict copy of, such as
/// `notes.md`. `None` for any other file.
pub fn document_copy(file_name: &str) -> Option<&'static str> {
    let stem = file_name.strip_suffix(".md")?;
    DOCUMENTS
        .into_iter()
        .find(|document| is_copy_name(stem, document.trim_end_matches(".md")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_names() {
        assert!(is_copy_name("notes (1)", "notes"));
        assert!(is_copy_name(
            "notes (Erv's conflicted copy 2026-09-25)",
            "notes"
        ));
        assert!(is_copy_name("notes-DESKTOP-ABC", "notes"));
        assert!(is_copy_name("notes 2", "notes"));
        assert!(!is_copy_name("notes", "notes"));
        assert!(!is_copy_name("notes ()", "notes"));
        assert!(!is_copy_name("notes 2b", "notes"));
        assert!(!is_copy_name("notesX", "notes"));
        assert!(!is_copy_name("notes-", "notes"));
        assert!(!is_copy_name("notes-a b", "notes"));
    }

    #[test]
    fn document_copies() {
        assert_eq!(document_copy("summary (1).md"), Some("summary.md"));
        assert_eq!(document_copy("session-LAPTOP.md"), Some("session.md"));
        assert_eq!(document_copy("notes.md"), None);
        assert_eq!(document_copy("notes (1).txt"), None);
    }
}
