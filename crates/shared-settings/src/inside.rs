//! The rule for links in the hidden Longhand folder. A synced folder can
//! bring a link to a file outside the storage folder. Longhand follows a
//! link only when its target is in the storage folder, so a file that the
//! user did not put in the storage folder never becomes the Background or
//! a prompt, and is never sent to the summary service.

use std::io;
use std::path::{Path, PathBuf};

/// The reason for a file that resolves outside the storage folder.
pub(crate) const OUTSIDE_FILE: &str = "it links to a file outside the storage folder";

/// The reason for a folder that resolves outside the storage folder.
pub(crate) const OUTSIDE_FOLDER: &str = "it is in a folder outside the storage folder";

/// The canonical path of the file `path` when it is in the storage folder
/// `root`. `canonicalize` resolves each link on the path, so a linked
/// `.longhand` or `prompts` folder is examined too. A link to a file that
/// does not exist gives the error of `canonicalize`. A path outside the
/// storage folder gives an error with [`OUTSIDE_FILE`].
pub(crate) fn inside_file(root: &Path, path: &Path) -> io::Result<PathBuf> {
    let root = root.canonicalize()?;
    let path = path.canonicalize()?;
    if path.starts_with(&root) {
        Ok(path)
    } else {
        Err(io::Error::other(OUTSIDE_FILE))
    }
}

/// Refuse a write into `folder` when it resolves outside the storage
/// folder `root`. The folder does not have to exist: the check examines
/// the nearest part of the path that exists, which is the folder where a
/// write would make the first new folder.
pub(crate) fn check_write_folder(root: &Path, folder: &Path) -> io::Result<()> {
    let root = root.canonicalize()?;
    let mut existing = folder;
    while std::fs::symlink_metadata(existing).is_err() {
        match existing.parent() {
            Some(parent) => existing = parent,
            None => return Err(io::Error::other(OUTSIDE_FOLDER)),
        }
    }
    let existing = existing
        .canonicalize()
        .map_err(|_| io::Error::other(OUTSIDE_FOLDER))?;
    if existing.starts_with(&root) {
        Ok(())
    } else {
        Err(io::Error::other(OUTSIDE_FOLDER))
    }
}
