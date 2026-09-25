//! Renames that never replace (KTD17). A plain `rename(2)` onto an empty
//! directory replaces it, so the final rename asks the platform to refuse
//! any existing target: `renamex_np(RENAME_EXCL)` on macOS,
//! `renameat2(RENAME_NOREPLACE)` on Linux. Windows' `MoveFileEx` refuses an
//! existing directory target by itself.

use std::io;
use std::path::Path;

/// The filesystem operations that saving is built on, so a test can make a
/// rename cross devices.
pub trait Fs {
    /// Move `from` to `to`; may replace an empty `to` (the staging rename).
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Move `from` to `to` only if nothing is at `to`: `AlreadyExists` or
    /// `DirectoryNotEmpty` otherwise.
    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()>;
}

pub struct RealFs;

impl Fs for RealFs {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        rename_noreplace(from, to)
    }
}

#[cfg(unix)]
fn c_path(path: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

#[cfg(target_os = "macos")]
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let (from, to) = (c_path(from)?, c_path(to)?);
    // SAFETY: both are valid NUL-terminated paths for the call's duration.
    let rc = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let (from, to) = (c_path(from)?, c_path(to)?);
    // SAFETY: both are valid NUL-terminated paths for the call's duration.
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    // `MoveFileExW` refuses to replace a directory; the check covers a file
    // squatting on the name.
    if to.symlink_metadata().is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    std::fs::rename(from, to)
}

/// Whether a rename failed because the name is taken.
pub fn is_taken(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
    ) || (cfg!(windows) && e.kind() == io::ErrorKind::PermissionDenied)
}

/// Whether a rename of a home folder failed because another process holds
/// the folder or a file in it (KTD8). Windows reports a sharing violation
/// or `PermissionDenied`; a sync client on macOS can report
/// `PermissionDenied` too, and Linux reports `EBUSY`. A home folder rename
/// retries these and never takes them for a name that is taken, unlike
/// [`is_taken`], which the first save uses.
pub fn is_in_use(e: &io::Error) -> bool {
    /// `ERROR_SHARING_VIOLATION` and `ERROR_LOCK_VIOLATION`.
    const WINDOWS_IN_USE: [i32; 2] = [32, 33];
    matches!(
        e.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ResourceBusy
    ) || (cfg!(windows)
        && e.raw_os_error()
            .is_some_and(|c| WINDOWS_IN_USE.contains(&c)))
}

/// Whether a home folder rename failed because the name is taken. Only
/// these two kinds: `PermissionDenied` is [`is_in_use`].
pub fn is_name_taken(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
    )
}
