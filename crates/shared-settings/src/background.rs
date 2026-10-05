//! The Background: one markdown file, `.longhand/background.md`, with the
//! text that the user wants each summary model to know (R1, R5, R6, KTD1,
//! KTD2, KTD3).
//!
//! The file is the user's text as it is, with no front matter. Only the
//! exact name counts: a sync conflict copy such as `background 2.md` has no
//! effect. A read never writes. Only a save that the user makes, or the
//! join (KTD2), writes the file. A save of an empty text writes an empty
//! file, so that the join does not fill the file again.

use std::io;
use std::path::{Path, PathBuf};

pub use summarize::prompts::BACKGROUND_TEXT_LIMIT;

use crate::file::{FileRead, Unavailable, check_root, longhand_folder, read_settings, write_new};
use crate::prompts::format_count;

/// The name of the Background file in the hidden Longhand folder.
pub const BACKGROUND_FILE: &str = "background.md";

/// The largest Background file that a read opens. A text of
/// [`BACKGROUND_TEXT_LIMIT`] characters of four bytes each fits. A larger
/// file is too long, and the read does not open it.
const MAX_FILE_BYTES: u64 = BACKGROUND_TEXT_LIMIT as u64 * 4;

/// The Background file of the storage folder `root`.
pub fn background_path(root: &Path) -> PathBuf {
    longhand_folder(root).join(BACKGROUND_FILE)
}

/// Why the Background file cannot be used (KTD3). The Background is then
/// read-only, and the app uses the cached text. The messages are the UI
/// text of the Settings window.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackgroundUnavailable {
    /// The storage folder is not available.
    #[error("{0}")]
    Folder(Unavailable),
    #[error(
        "The Background file is too long. Use {} characters or fewer.",
        format_count(BACKGROUND_TEXT_LIMIT)
    )]
    TooLong,
    /// The file is not UTF-8 text, for example because a sync delivered
    /// only a part of it.
    #[error("The Background file is not text.")]
    NotText,
    #[error("The Background file cannot be read: {0}")]
    Unreadable(String),
}

/// The result of a read of the Background file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundRead {
    /// The text of the file. An empty file gives an empty text.
    Text(String),
    /// The storage folder has no Background file. In a folder with a
    /// settings file, this is an empty Background that the user can edit.
    Missing,
    /// The file exists and cannot be used. Never write over it.
    NotAvailable(BackgroundUnavailable),
}

/// Why a save of the Background failed. The messages are the UI text of
/// the Settings window.
#[derive(Debug, thiserror::Error)]
pub enum BackgroundError {
    #[error(
        "The Background is too long. Use {} characters or fewer.",
        format_count(BACKGROUND_TEXT_LIMIT)
    )]
    TooLong,
    /// The shared settings are read-only. The save wrote nothing.
    #[error("{0}")]
    NotAvailable(Unavailable),
    /// The storage folder has no shared settings file. The join fills it
    /// first.
    #[error("The storage folder has no shared settings yet.")]
    NoSettingsFile,
    /// The Background file exists and cannot be used. The save wrote
    /// nothing (KTD3).
    #[error("{0}")]
    FileNotAvailable(BackgroundUnavailable),
    #[error("The Background file cannot be written: {0}")]
    Io(#[from] io::Error),
}

/// Read the Background file of the storage folder `root`. This never
/// writes.
pub fn read_background(root: &Path) -> BackgroundRead {
    if let Err(cause) = check_root(root) {
        return BackgroundRead::NotAvailable(BackgroundUnavailable::Folder(cause));
    }
    let path = background_path(root);
    let meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return BackgroundRead::Missing,
        Err(e) => return unreadable(e.to_string()),
    };
    if !meta.is_file() {
        return unreadable("it is not a file".into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return BackgroundRead::NotAvailable(BackgroundUnavailable::TooLong);
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return BackgroundRead::Missing,
        Err(e) => return unreadable(e.to_string()),
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return BackgroundRead::NotAvailable(BackgroundUnavailable::NotText);
    };
    if text.chars().count() > BACKGROUND_TEXT_LIMIT {
        return BackgroundRead::NotAvailable(BackgroundUnavailable::TooLong);
    }
    BackgroundRead::Text(text)
}

fn unreadable(cause: String) -> BackgroundRead {
    BackgroundRead::NotAvailable(BackgroundUnavailable::Unreadable(cause))
}

/// Save `text` as the Background of the storage folder `root`. Only a
/// change that the user made calls this. The write is atomic, and an
/// empty text writes an empty file (KTD2). The save writes nothing while
/// the shared settings are read-only or the Background file cannot be
/// used (KTD3).
pub fn save_background(root: &Path, text: &str) -> Result<(), BackgroundError> {
    match read_settings(root) {
        FileRead::Parsed(_) => {}
        FileRead::Missing => return Err(BackgroundError::NoSettingsFile),
        FileRead::NotAvailable(cause) => return Err(BackgroundError::NotAvailable(cause)),
    }
    if text.chars().count() > BACKGROUND_TEXT_LIMIT {
        return Err(BackgroundError::TooLong);
    }
    if let BackgroundRead::NotAvailable(cause) = read_background(root) {
        return Err(BackgroundError::FileNotAvailable(cause));
    }
    session_save::home_write::write_atomic(
        &longhand_folder(root),
        BACKGROUND_FILE,
        text.as_bytes(),
    )?;
    Ok(())
}

/// Write `text` as the Background file of `root` only when the folder has
/// no Background file and the text is not empty. Only the join calls
/// this (KTD2). A file that a sync delivers in the meantime stays.
/// Returns true when it wrote the file.
pub(crate) fn write_new_background(root: &Path, text: &str) -> io::Result<bool> {
    if text.trim().is_empty() || text.chars().count() > BACKGROUND_TEXT_LIMIT {
        return Ok(false);
    }
    let folder = longhand_folder(root);
    std::fs::create_dir_all(&folder)?;
    write_new(&folder, BACKGROUND_FILE, text.as_bytes())
}
