//! The storage folder: its default path, the checks for a new one, and
//! whether it can take a note (R1, R2, KTD11, KTD12).
//!
//! This is the status the surface shows. The binding checks at stop and
//! immediately before saving belong to the save stage (U12), which adds
//! the session's own size to the floor below.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use specta::Type;

/// Free space below which the destination is reported as low, as anarlog's
/// recorder reserves for its own writes.
pub const FREE_SPACE_FLOOR_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DestinationStatus {
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

/// Why a new note cannot start, with or without a recording, as New Note
/// shows it (R7).
pub const NOTE_CANNOT_START: &str = "The storage folder is not available, so a new note can’t start. Connect its disk or choose another folder in Settings.";

/// Why a new note cannot start while the storage folder has `status`
/// (R7), or `None` when it can. A folder with little free space takes a
/// note: the save waits for space.
pub fn note_start_refusal(status: &DestinationStatus) -> Option<&'static str> {
    match status {
        DestinationStatus::Ready { .. } | DestinationStatus::LowSpace { .. } => None,
        DestinationStatus::Missing { .. }
        | DestinationStatus::NotADirectory { .. }
        | DestinationStatus::NotWritable { .. } => Some(NOTE_CANNOT_START),
    }
}

/// The status of the storage folder `folder`. An empty path is missing:
/// the launch sets a storage folder before any reader asks.
pub fn status(folder: &str) -> DestinationStatus {
    check(Path::new(folder), FREE_SPACE_FLOOR_BYTES)
}

/// The default storage folder: `Longhand/Notes` under `local_data`, the
/// local data folder of the operating system (KTD12). On macOS that is
/// `~/Library/Application Support`, and on Windows `%LOCALAPPDATA%`. The
/// folder is outside the app's own data folder, so an uninstall that
/// deletes the app data keeps the notes (R2).
pub fn default_storage_folder(local_data: &Path) -> PathBuf {
    local_data.join("Longhand").join("Notes")
}

/// Whether `new` can become the storage folder in place of `current`
/// (KTD11). The error is the cause, as Settings shows it. The same folder,
/// a folder inside the storage folder, a folder that contains it, and a
/// folder that Longhand cannot write are refused. An empty `current`
/// means that no storage folder is set.
pub fn check_new_folder(current: &Path, new: &Path) -> Result<(), String> {
    let resolve = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !current.as_os_str().is_empty() {
        let (current, new) = (resolve(current), resolve(new));
        if new == current {
            return Err("This folder is already the storage folder.".into());
        }
        if new.starts_with(&current) {
            return Err(
                "This folder is inside the storage folder. Choose a folder outside it.".into(),
            );
        }
        if current.starts_with(&new) {
            return Err(
                "This folder contains the storage folder. Choose a folder that does not contain it."
                    .into(),
            );
        }
    }
    match new.metadata() {
        Err(_) => return Err("This folder is not available.".into()),
        Ok(meta) if !meta.is_dir() => return Err("This is a file, not a folder.".into()),
        Ok(_) => {}
    }
    probe_writable(new).map_err(|e| format!("Longhand cannot save notes in this folder: {e}"))
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
    let free_bytes = recorder::storage::available_space(dir);
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
/// The name has a count, because checks can run at the same time and the
/// macOS clock changes only once per microsecond.
fn probe_writable(dir: &Path) -> std::io::Result<()> {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let count = COUNT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let probe: PathBuf = dir.join(format!(
        ".longhand-write-probe-{}-{nanos}-{count}",
        std::process::id()
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    std::fs::remove_file(&probe)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundle identifier in `tauri.conf.json`.
    const APP_IDENTIFIER: &str = "net.ewal.longhand";

    // Covers AE8 (R7): no new note starts while the storage folder is not
    // available, with or without a recording.
    #[test]
    fn a_new_note_starts_only_while_the_storage_folder_is_available() {
        let path = String::from("/notes");
        let refused = [
            DestinationStatus::Missing { path: path.clone() },
            DestinationStatus::NotADirectory { path: path.clone() },
            DestinationStatus::NotWritable {
                path: path.clone(),
                message: "denied".into(),
            },
        ];
        for status in &refused {
            assert_eq!(
                note_start_refusal(status),
                Some(NOTE_CANNOT_START),
                "{status:?}"
            );
        }
        assert_eq!(note_start_refusal(&status("")), Some(NOTE_CANNOT_START));
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            note_start_refusal(&status(dir.path().to_str().unwrap())),
            None
        );
        let low = DestinationStatus::LowSpace {
            path,
            free_bytes: 1,
            floor_bytes: FREE_SPACE_FLOOR_BYTES,
        };
        assert_eq!(note_start_refusal(&low), None);
        assert!(NOTE_CANNOT_START.contains("note"));
    }

    #[test]
    fn no_storage_folder_is_missing() {
        assert_eq!(
            status(""),
            DestinationStatus::Missing {
                path: String::new()
            }
        );
    }

    #[test]
    fn the_default_folder_is_longhand_notes_under_the_local_data_folder() {
        let mac = Path::new("/Users/someone/Library/Application Support");
        assert_eq!(
            default_storage_folder(mac),
            Path::new("/Users/someone/Library/Application Support/Longhand/Notes")
        );
    }

    #[test]
    fn the_default_folder_on_windows_is_under_local_app_data_and_outside_the_app_folder() {
        // `local_data_dir()` is `%LOCALAPPDATA%` on Windows, not the roaming
        // `%APPDATA%`. The uninstaller's "Delete the application data"
        // removes only the folders named for the identifier (KTD12).
        let local = PathBuf::from(r"C:\Users\someone\AppData\Local");
        let folder = default_storage_folder(&local);
        assert!(folder.starts_with(&local));
        assert!(
            folder
                .components()
                .all(|c| c.as_os_str() != APP_IDENTIFIER && c.as_os_str() != "Roaming")
        );
        assert!(!folder.starts_with(local.join(APP_IDENTIFIER)));
        assert!(folder.ends_with(Path::new("Longhand").join("Notes")));
    }

    #[test]
    fn the_same_folder_a_folder_inside_it_and_a_folder_that_contains_it_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("Notes");
        let inside = current.join("Archive");
        std::fs::create_dir_all(&inside).unwrap();
        let same = check_new_folder(&current, &current).unwrap_err();
        assert!(same.contains("already"), "{same}");
        // The same folder by another spelling is refused too.
        let spelled = dir.path().join("Notes/../Notes");
        assert!(check_new_folder(&current, &spelled).is_err());
        let within = check_new_folder(&current, &inside).unwrap_err();
        assert!(within.contains("inside the storage folder"), "{within}");
        let contains = check_new_folder(&current, dir.path()).unwrap_err();
        assert!(
            contains.contains("contains the storage folder"),
            "{contains}"
        );
    }

    #[test]
    fn a_sibling_folder_with_a_longer_name_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("Notes");
        let sibling = dir.path().join("Notes 2");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        assert_eq!(check_new_folder(&current, &sibling), Ok(()));
        // A storage folder that is not available is no reason to refuse.
        assert_eq!(check_new_folder(&dir.path().join("gone"), &sibling), Ok(()));
        assert_eq!(check_new_folder(Path::new(""), &sibling), Ok(()));
        assert_eq!(std::fs::read_dir(&sibling).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_folder_and_a_file_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("Notes");
        let file = dir.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(check_new_folder(&current, &dir.path().join("gone")).is_err());
        assert!(check_new_folder(&current, &file).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_longhand_cannot_write_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = check_new_folder(&dir.path().join("Notes"), &locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cause = result.unwrap_err();
        assert!(cause.contains("cannot save"), "{cause}");
    }

    #[test]
    fn a_writable_folder_is_ready_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            check(dir.path(), 0),
            DestinationStatus::Ready {
                free_bytes: Some(_),
                ..
            }
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

    // Checks that run at the same time must not use the same probe file.
    // On macOS the clock changes only once per microsecond.
    #[test]
    fn checks_at_the_same_time_all_find_the_folder_writable() {
        let dir = tempfile::tempdir().unwrap();
        let barrier = std::sync::Barrier::new(16);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..16)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        (0..200)
                            .map(|_| probe_writable(dir.path()))
                            .find(Result::is_err)
                    })
                })
                .collect();
            for handle in handles {
                assert!(handle.join().unwrap().is_none());
            }
        });
    }
}
