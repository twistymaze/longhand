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

pub fn staging_name(session_id: &str) -> String {
    format!("{STAGING_PREFIX}{session_id}")
}
