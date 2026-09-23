//! Folder names in the destination.

use chrono::{DateTime, FixedOffset};

/// `YYYY-MM-DD HHMM Longhand session`, in the session's own wall clock.
pub fn folder_name(started: DateTime<FixedOffset>) -> String {
    format!("{} Longhand session", started.format("%Y-%m-%d %H%M"))
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
pub fn staging_name(session_id: &str) -> String {
    format!(".longhand-staging-{session_id}")
}
