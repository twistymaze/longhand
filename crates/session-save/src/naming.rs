//! Folder names in the destination.

use chrono::{DateTime, FixedOffset};

/// The most characters of a title that a folder name keeps.
pub const MAX_TITLE_CHARS: usize = 120;

/// The characters that some Destination refuses in a name: Windows and
/// OneDrive refuse these, and macOS refuses `/` and treats `:` as `/`.
const FORBIDDEN: &[char] = &['"', '*', ':', '<', '>', '?', '/', '\\', '|'];

/// Names that Windows keeps for devices, and names that Windows and
/// OneDrive keep for their own files. A name matches with any extension
/// and in any case.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
const RESERVED_FILES: &[&str] = &["desktop.ini", "thumbs.db", ".lock"];

/// `YYYY-MM-DD HHMM <title>`, in the session's own wall clock (R30, KTD7).
/// `started` is the note's start time, not the start of its first segment:
/// a note that opens before it records keeps its open time. The title goes
/// through [`sanitize_title`]. With no title, or a title that has nothing
/// left after that, the name is `YYYY-MM-DD HHMM Longhand session`. Every
/// Destination gets this name.
pub fn folder_name(started: DateTime<FixedOffset>, title: Option<&str>) -> String {
    let title = title
        .and_then(sanitize_title)
        .unwrap_or_else(|| "Longhand session".to_string());
    format!("{} {title}", started.format("%Y-%m-%d %H%M"))
}

/// The title as a safe part of a folder name, or `None` when nothing is
/// left. The function:
/// - removes the forbidden characters and control characters,
/// - changes each run of white space to one space,
/// - removes spaces and dots at either end, which also removes a leading dot,
/// - cuts the title to [`MAX_TITLE_CHARS`] characters, then removes spaces
///   and dots at the end again,
/// - adds `_` to a reserved name before its first dot, so the part is safe
///   as a name by itself.
pub fn sanitize_title(title: &str) -> Option<String> {
    let kept: String = title
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && !FORBIDDEN.contains(c))
        .collect();
    let collapsed = kept
        .split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let trim = |s: &str| s.trim_matches([' ', '.']).to_string();
    let cut: String = trim(&collapsed).chars().take(MAX_TITLE_CHARS).collect();
    let title = trim(&cut);
    if title.is_empty() {
        return None;
    }
    if is_reserved(&title) {
        // After the part before the first dot, where the reserved word is.
        let at = title.find('.').unwrap_or(title.len());
        return Some(format!("{}_{}", &title[..at], &title[at..]));
    }
    Some(title)
}

/// The name of the built-in note type of the notes whose home folder is
/// directly in the storage folder. No type folder can use it.
pub const GENERAL: &str = "General";

/// Why a note type name is refused (R28). The text is for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TypeNameError {
    #[error("Enter a name.")]
    Empty,
    #[error("General is a built-in type.")]
    General,
    #[error("A type name cannot start with a dot.")]
    StartsWithDot,
    #[error("A type name cannot end with a dot.")]
    EndsWithDot,
    #[error("A type name cannot contain control characters.")]
    ControlCharacter,
    #[error("A type name cannot contain \"{0}\".")]
    ForbiddenCharacter(char),
    #[error("Windows uses this name for its own files.")]
    Reserved,
    #[error("A type name can have at most {MAX_TITLE_CHARS} characters.")]
    TooLong,
}

/// The note type name with the spaces at its ends removed, or the reason
/// that it cannot be a type folder name on macOS or on Windows (R28, KTD8).
/// Every place that accepts a type name uses this check. The caller also
/// refuses a name that an existing home folder in the storage folder has,
/// because only the caller knows the storage folder.
pub fn check_type_name(name: &str) -> Result<String, TypeNameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(TypeNameError::Empty);
    }
    if same_type_name(name, GENERAL) {
        return Err(TypeNameError::General);
    }
    // This also refuses `.`, `..` and `.longhand`.
    if name.starts_with('.') {
        return Err(TypeNameError::StartsWithDot);
    }
    if name.chars().any(char::is_control) {
        return Err(TypeNameError::ControlCharacter);
    }
    if let Some(c) = name.chars().find(|c| FORBIDDEN.contains(c)) {
        return Err(TypeNameError::ForbiddenCharacter(c));
    }
    // Windows removes a dot at the end of a folder name.
    if name.ends_with('.') {
        return Err(TypeNameError::EndsWithDot);
    }
    if is_reserved(name) {
        return Err(TypeNameError::Reserved);
    }
    if name.chars().count() > MAX_TITLE_CHARS {
        return Err(TypeNameError::TooLong);
    }
    Ok(name.to_string())
}

/// Whether two note type names name the same type. The comparison ignores
/// case and the spaces at the ends, because macOS and Windows find a folder
/// without regard to case.
pub fn same_type_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Whether two optional note types are one type. `None` is General.
pub fn same_optional_type(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_type_name(a, b),
        _ => false,
    }
}

/// Whether `name` is a name that some Destination keeps for itself.
pub fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    RESERVED.iter().any(|r| stem.eq_ignore_ascii_case(r))
        || RESERVED_FILES.iter().any(|r| name.eq_ignore_ascii_case(r))
}

/// The `attempt`th candidate: the name itself, then `-2`, `-3`, and so on.
pub fn candidate(name: &str, attempt: usize) -> String {
    if attempt <= 1 {
        name.to_string()
    } else {
        format!("{name}-{attempt}")
    }
}

/// The hidden directory a session is assembled in inside its destination.
pub const STAGING_PREFIX: &str = ".longhand-staging-";

pub fn staging_name(note_id: &str) -> String {
    format!("{STAGING_PREFIX}{note_id}")
}
