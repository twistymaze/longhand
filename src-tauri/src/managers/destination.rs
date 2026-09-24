//! The destination folder: choosing it and reporting whether it can take a
//! session (R22c display, R23, R25a).
//!
//! This is the status the surface shows. The binding checks at stop and
//! immediately before saving belong to the save stage (U12), which adds
//! the session's own size to the floor below.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use specta::Type;

/// Free space below which the destination is reported as low, as anarlog's
/// recorder reserves for its own writes.
pub const FREE_SPACE_FLOOR_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DestinationStatus {
    Unset,
    Ready {
        path: String,
        free_bytes: Option<u64>,
    },
    /// Deleted, renamed, or on a volume that is not mounted.
    Missing {
        path: String,
    },
    NotADirectory {
        path: String,
    },
    NotWritable {
        path: String,
        message: String,
    },
    LowSpace {
        path: String,
        free_bytes: u64,
        floor_bytes: u64,
    },
}

pub fn status(current: Option<&str>) -> DestinationStatus {
    let Some(path) = current else {
        return DestinationStatus::Unset;
    };
    check(Path::new(path), FREE_SPACE_FLOOR_BYTES)
}

pub fn check(dir: &Path, floor_bytes: u64) -> DestinationStatus {
    let path = dir.to_string_lossy().into_owned();
    match dir.metadata() {
        Err(_) => return DestinationStatus::Missing { path },
        Ok(meta) if !meta.is_dir() => return DestinationStatus::NotADirectory { path },
        Ok(_) => {}
    }
    if let Err(e) = probe_writable(dir) {
        return DestinationStatus::NotWritable {
            path,
            message: e.to_string(),
        };
    }
    let free_bytes = available_space(dir);
    match free_bytes {
        Some(free) if free < floor_bytes => DestinationStatus::LowSpace {
            path,
            free_bytes: free,
            floor_bytes,
        },
        _ => DestinationStatus::Ready { path, free_bytes },
    }
}

/// Create and remove a uniquely named file. `create_new` refuses an existing
/// name, symlink or not, so the probe never writes through a planted link.
fn probe_writable(dir: &Path) -> std::io::Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let probe: PathBuf = dir.join(format!(
        ".longhand-write-probe-{}-{nanos}",
        std::process::id()
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    std::fs::remove_file(&probe)
}

/// Free space on the volume holding `dir`: the deepest mount point that
/// contains it, as anarlog's `check_storage` resolves it. `None` when no
/// mount matches, which callers treat as unknown rather than full.
pub fn available_space(dir: &Path) -> Option<u64> {
    let canonical = dir.canonicalize().ok()?;
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|disk| {
            disk.mount_point()
                .canonicalize()
                .is_ok_and(|mount| canonical.starts_with(mount))
        })
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(|disk| disk.available_space())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_destination_is_unset() {
        assert_eq!(status(None), DestinationStatus::Unset);
    }

    #[test]
    fn a_writable_folder_is_ready_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            check(dir.path(), 0),
            DestinationStatus::Ready { .. }
        ));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_vanished_folder_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        assert!(matches!(check(&gone, 0), DestinationStatus::Missing { .. }));
    }

    #[test]
    fn a_file_is_not_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(matches!(
            check(&file, 0),
            DestinationStatus::NotADirectory { .. }
        ));
    }

    #[test]
    fn free_space_below_the_floor_is_low() {
        let dir = tempfile::tempdir().unwrap();
        // No volume has this much room, so the floor always trips when the
        // lookup resolves; an unresolved lookup reports ready with no figure.
        match check(dir.path(), u64::MAX) {
            DestinationStatus::LowSpace { .. } => {}
            DestinationStatus::Ready {
                free_bytes: None, ..
            } => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = check(&locked, 0);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(result, DestinationStatus::NotWritable { .. }));
    }
}
