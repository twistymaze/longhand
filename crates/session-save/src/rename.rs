//! Renames that never replace (KTD17). A plain `rename(2)` onto an empty
//! directory replaces it, so the final rename asks the platform to refuse
//! any existing target: `renamex_np(RENAME_EXCL)` on macOS,
//! `renameat2(RENAME_NOREPLACE)` on Linux. Windows' `MoveFileEx` refuses an
//! existing directory target by itself.

use std::io;
use std::path::Path;
use std::time::Duration;

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

/// A rename that is refused because the folder is in use is tried again
/// this many times, after [`IN_USE_DELAY`] each time.
pub const IN_USE_RETRIES: usize = 3;
pub const IN_USE_DELAY: Duration = Duration::from_millis(200);

/// [`Fs::rename_new`], tried again while the folder is in use
/// ([`is_in_use`]). Returns the last error.
pub fn rename_new_waiting(fs: &dyn Fs, from: &Path, to: &Path) -> io::Result<()> {
    let mut tries = 0;
    loop {
        match fs.rename_new(from, to) {
            Err(e) if is_in_use(&e) && tries < IN_USE_RETRIES => {
                tries += 1;
                log::info!(
                    "{} is in use; trying the rename again ({tries} of {IN_USE_RETRIES})",
                    from.display()
                );
                std::thread::sleep(IN_USE_DELAY);
            }
            result => return result,
        }
    }
}

/// The error for a rename of `path` that stayed in use after the tries.
/// Its kind is the kind of `e`.
pub fn in_use_error(path: &Path, e: &io::Error) -> io::Error {
    io::Error::new(
        e.kind(),
        format!(
            "{} is in use by another app, or access was refused ({e})",
            path.display()
        ),
    )
}

/// Whether a rename failed because another process holds the folder or a
/// file in it (KTD8). Windows reports a sharing violation or
/// `PermissionDenied`; a sync client on macOS can report `PermissionDenied`
/// too, and Linux reports `EBUSY`. A rename tries these again
/// ([`rename_new_waiting`]) and never takes them for a name that is taken.
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

/// Whether a rename failed because the name is taken. Only these two
/// kinds: `PermissionDenied` is [`is_in_use`]. The Windows
/// [`rename_noreplace`] checks the target first, so a taken name gives
/// `AlreadyExists` there too.
pub fn is_name_taken(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
    )
}
