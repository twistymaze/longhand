//! The rule for links in the hidden Longhand folder. A synced folder can
//! bring a link to a file outside the storage folder. Longhand follows a
//! link only when its target is in the storage folder, so a file that the
//! user did not put in the storage folder never becomes the Background or
//! a prompt, and is never sent to the summary service.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// The reason for a file that resolves outside the storage folder.
pub(crate) const OUTSIDE_FILE: &str = "it links to a file outside the storage folder";

/// The reason for a folder that resolves outside the storage folder.
pub(crate) const OUTSIDE_FOLDER: &str = "it is in a folder outside the storage folder";

/// The reason for an opened entry that is not a file.
const NOT_A_FILE: &str = "it is not a file";

/// The canonical path of the file `path` when it is in the storage folder
/// `root`. `canonicalize` resolves each link on the path, so a linked
/// `.longhand` or `prompts` folder is examined too. A link to a file that
/// does not exist gives the error of `canonicalize`. A path outside the
/// storage folder gives an error with [`OUTSIDE_FILE`].
pub(crate) fn inside_file(root: &Path, path: &Path) -> io::Result<PathBuf> {
    // A path with no link under the storage folder is in the storage
    // folder. `canonicalize` is then not used: it fails on some network
    // and virtual drives of Windows.
    if !has_link(root, path) {
        return Ok(path.to_path_buf());
    }
    let root = root.canonicalize()?;
    let path = path.canonicalize()?;
    if path.starts_with(&root) {
        Ok(path)
    } else {
        Err(io::Error::other(OUTSIDE_FILE))
    }
}

/// The bytes of the file `checked`. `checked` is the path that
/// [`inside_file`] gave for the storage folder `root`. The read uses
/// [`open_checked`].
pub(crate) fn read_checked(root: &Path, checked: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open_checked(root, checked)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Open the file `checked` for a read. `checked` is the path that
/// [`inside_file`] gave for the storage folder `root`. A sync can put a
/// link at a part of the path after that check. On Unix, the open thus
/// starts at the storage folder and refuses a link at each part of the
/// path, so the open file is in the storage folder. An entry that is not
/// a file gives an error.
pub(crate) fn open_checked(root: &Path, checked: &Path) -> io::Result<File> {
    let file = open_no_link(root, checked)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other(NOT_A_FILE));
    }
    Ok(file)
}

/// Open `checked` from the folder `root` one part at a time, and refuse a
/// link at each part under `root`.
#[cfg(unix)]
fn open_no_link(root: &Path, checked: &Path) -> io::Result<File> {
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Component;

    // `inside_file` gives the path as it is, under `root`, or the
    // canonical path, under the canonical root.
    let canonical;
    let (base, rest) = match checked.strip_prefix(root) {
        Ok(rest) => (root, rest),
        Err(_) => {
            canonical = root.canonicalize()?;
            let rest = checked
                .strip_prefix(&canonical)
                .map_err(|_| io::Error::other(OUTSIDE_FILE))?;
            (canonical.as_path(), rest)
        }
    };
    let parts: Vec<Component> = rest.components().collect();
    let Some((name, folders)) = parts.split_last() else {
        return Err(io::Error::other(NOT_A_FILE));
    };
    let mut dir: OwnedFd = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(base)?
        .into();
    for folder in folders {
        dir = open_at(&dir, folder, libc::O_DIRECTORY)?;
    }
    Ok(File::from(open_at(&dir, name, 0)?))
}

/// Open `part` in the open folder `dir` for a read. The open refuses a
/// link and each part other than a name.
#[cfg(unix)]
fn open_at(
    dir: &std::os::fd::OwnedFd,
    part: &std::path::Component,
    flags: libc::c_int,
) -> io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    let std::path::Component::Normal(name) = part else {
        return Err(io::Error::other(OUTSIDE_FILE));
    };
    let name = std::ffi::CString::new(name.as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let flags = flags | libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    loop {
        // SAFETY: `dir` is an open folder, and `name` is a valid
        // NUL-terminated name for the call's duration.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
        if fd >= 0 {
            // SAFETY: `openat` returned a new descriptor that nothing else
            // owns.
            return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Windows has no `openat`. This open follows a link that a sync puts in
/// place after the check of [`inside_file`].
#[cfg(not(unix))]
fn open_no_link(_root: &Path, checked: &Path) -> io::Result<File> {
    File::open(checked)
}

/// Refuse a write into `folder` when it resolves outside the storage
/// folder `root`. The folder does not have to exist: the check examines
/// the nearest part of the path that exists, which is the folder where a
/// write would make the first new folder.
pub(crate) fn check_write_folder(root: &Path, folder: &Path) -> io::Result<()> {
    let mut existing = folder;
    while std::fs::symlink_metadata(existing).is_err() {
        match existing.parent() {
            Some(parent) => existing = parent,
            None => return Err(io::Error::other(OUTSIDE_FOLDER)),
        }
    }
    if !has_link(root, existing) {
        return Ok(());
    }
    let root = root.canonicalize()?;
    let existing = existing
        .canonicalize()
        .map_err(|_| io::Error::other(OUTSIDE_FOLDER))?;
    if existing.starts_with(&root) {
        Ok(())
    } else {
        Err(io::Error::other(OUTSIDE_FOLDER))
    }
}

/// Whether a part of `path` under the storage folder `root` is a link, or
/// cannot be examined. A path that is not under `root` gives true, so the
/// caller examines it with `canonicalize`.
fn has_link(root: &Path, path: &Path) -> bool {
    let Ok(rest) = path.strip_prefix(root) else {
        return true;
    };
    let mut at = root.to_path_buf();
    for part in rest.components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(meta) if !meta.file_type().is_symlink() => {}
            _ => return true,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A storage folder with the file `.longhand/background.md` that holds
    /// `text`. Returns the folder and the file.
    fn folder_with_file(text: &str) -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join(".longhand");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("background.md");
        std::fs::write(&path, text).unwrap();
        (root, path)
    }

    #[test]
    fn a_read_after_the_check_gives_the_file() {
        let (root, path) = folder_with_file("Inside text.");
        let checked = inside_file(root.path(), &path).unwrap();
        assert_eq!(
            read_checked(root.path(), &checked).unwrap(),
            b"Inside text."
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_read_after_the_check_follows_a_link_to_a_file_in_the_folder() {
        let (root, path) = folder_with_file("Inside text.");
        let link = path.with_file_name("link.md");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        let checked = inside_file(root.path(), &link).unwrap();
        assert_eq!(
            read_checked(root.path(), &checked).unwrap(),
            b"Inside text."
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_read_after_the_check_follows_a_linked_folder_in_the_folder() {
        let (root, path) = folder_with_file("Inside text.");
        let real = root.path().join("real");
        std::fs::rename(path.parent().unwrap(), &real).unwrap();
        std::os::unix::fs::symlink(&real, root.path().join(".longhand")).unwrap();
        let checked = inside_file(root.path(), &path).unwrap();
        assert_eq!(
            read_checked(root.path(), &checked).unwrap(),
            b"Inside text."
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_becomes_a_link_after_the_check_is_not_read() {
        let (root, path) = folder_with_file("Inside text.");
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.md");
        std::fs::write(&secret, "Outside text.").unwrap();
        let checked = inside_file(root.path(), &path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&secret, &path).unwrap();
        // A read by path now gives the file outside the storage folder.
        assert_eq!(std::fs::read(&checked).unwrap(), b"Outside text.");
        assert!(read_checked(root.path(), &checked).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_becomes_a_link_after_the_check_is_not_read() {
        let (root, path) = folder_with_file("Inside text.");
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("background.md"), "Outside text.").unwrap();
        let checked = inside_file(root.path(), &path).unwrap();
        let folder = path.parent().unwrap();
        std::fs::rename(folder, root.path().join("old")).unwrap();
        std::os::unix::fs::symlink(outside.path(), folder).unwrap();
        assert_eq!(std::fs::read(&checked).unwrap(), b"Outside text.");
        assert!(read_checked(root.path(), &checked).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_on_a_checked_link_that_becomes_a_link_is_not_read() {
        // The check resolves the link to `notes/target.md` in the folder.
        // Then `notes` becomes a link to a folder outside.
        let (root, path) = folder_with_file("Inside text.");
        let notes = root.path().join("notes");
        std::fs::create_dir(&notes).unwrap();
        std::fs::rename(&path, notes.join("target.md")).unwrap();
        std::os::unix::fs::symlink(notes.join("target.md"), &path).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("target.md"), "Outside text.").unwrap();
        let checked = inside_file(root.path(), &path).unwrap();
        std::fs::rename(&notes, root.path().join("old")).unwrap();
        std::os::unix::fs::symlink(outside.path(), &notes).unwrap();
        assert_eq!(std::fs::read(&checked).unwrap(), b"Outside text.");
        assert!(read_checked(root.path(), &checked).is_err());
    }

    #[test]
    fn a_file_that_becomes_a_folder_after_the_check_is_not_read() {
        let (root, path) = folder_with_file("Inside text.");
        let checked = inside_file(root.path(), &path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(read_checked(root.path(), &checked).is_err());
    }
}
