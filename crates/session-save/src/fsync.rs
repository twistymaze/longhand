//! Durability (KTD17). A file is flushed through a writable handle, which
//! Windows requires; a directory is flushed where the platform can, so a
//! rename into it survives power loss. `sync_all` is `F_FULLFSYNC` on macOS.

use std::fs::OpenOptions;
use std::io;
use std::path::Path;

pub fn sync_file(path: &Path) -> io::Result<()> {
    OpenOptions::new().write(true).open(path)?.sync_all()
}

pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Every file under `dir`, then every directory bottom-up.
pub fn sync_tree(dir: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            sync_tree(&entry.path())?;
        } else if kind.is_file() {
            sync_file(&entry.path())?;
        }
    }
    sync_dir(dir)
}
