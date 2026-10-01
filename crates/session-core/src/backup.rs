//! Move the database and the working folder aside (KTD4).
//!
//! A rebuild moves four entries of the app data folder into one backup
//! folder, `database-backups/<UTC time>/`: the working folder `sessions/`,
//! `longhand.db-shm`, `longhand.db-wal` and `longhand.db`, in that order.
//! Every step is a rename on one volume. Nothing is copied.
//!
//! A backup folder can hold two small files next to the moved entries:
//!
//! - `move-incomplete` ([`INCOMPLETE_FLAG`]): the move into this folder did
//!   not finish. The move writes it before the first rename and removes it
//!   after the last rename. At launch, [`undo_incomplete`] moves the entries
//!   of each folder with this flag back and removes the folder.
//! - `rebuild-chosen` ([`REBUILD_MARKER`]): the user chose Rebuild in an
//!   alert. Retention never deletes a folder with this marker.
//!
//! The steps of a move, and what the disk shows after a crash at each one:
//!
//! 1. Make the backup folder. On disk: a folder with no entries and no flag.
//!    [`undo_incomplete`] removes it.
//! 2. Write the flag, then the marker (if any). Sync the two files, the
//!    backup folder, `database-backups/` and the app data folder. On disk:
//!    the flag and no entries. [`undo_incomplete`] removes the folder.
//! 3. Rename the entries. On disk: the flag and some entries.
//!    [`undo_incomplete`] moves them back and removes the folder.
//! 4. Sync the backup folder and the app data folder, then remove the flag.
//!    On disk: a complete backup.
//!
//! The syncs make sure that a power loss cannot keep a rename but lose the
//! flag or the marker. On Windows, a directory sync does nothing.
//!
//! Recovery is manual: put back both the database and the working folder,
//! then open the build that owns the database.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The folder in the app data folder that holds every backup.
pub const BACKUPS_DIR: &str = "database-backups";
/// The flag file of a backup whose move did not finish.
pub const INCOMPLETE_FLAG: &str = "move-incomplete";
/// The marker file of a backup that the user chose in an alert.
pub const REBUILD_MARKER: &str = "rebuild-chosen";
/// Retention keeps this number of the newest automatic backups (R11).
pub const KEEP_AUTOMATIC: usize = 3;
/// A failed rename retries for this time (Windows sharing violations).
pub const RETRY_FOR: Duration = Duration::from_secs(2);

const DB: &str = "longhand.db";
const WAL: &str = "longhand.db-wal";
const SHM: &str = "longhand.db-shm";
const SESSIONS: &str = "sessions";

/// The entries of a move, in move order. An undo uses the reverse order.
const MOVE_ORDER: [&str; 4] = [SESSIONS, SHM, WAL, DB];
/// The entries of an orphan move, in move order.
const ORPHAN_ORDER: [&str; 2] = [SHM, WAL];

const INCOMPLETE_TEXT: &str = "Longhand did not finish moving files into this folder. \
At the next launch, Longhand moves them back into the app data folder.\n";
const MARKER_TEXT: &str = "The user chose Rebuild in an alert. \
Longhand does not delete this backup.\n";

/// Why a backup is made. Retention (R11) keeps every `ChosenInAlert` backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    /// No alert was shown: the database held no unfinished work.
    Automatic,
    /// The user chose Rebuild in an alert (R6 or R7).
    ChosenInAlert,
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("{action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot move {} back: the app data folder already has it", .0.display())]
    TargetExists(PathBuf),
}

fn io_error(action: &'static str, path: &Path) -> impl FnOnce(io::Error) -> BackupError {
    let path = path.to_path_buf();
    move |source| BackupError::Io {
        action,
        path,
        source,
    }
}

/// The rename function that a move uses. Production uses `std::fs::rename`;
/// a test gives a function that fails.
pub type Rename<'a> = &'a mut dyn FnMut(&Path, &Path) -> io::Result<()>;

fn system_rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// Rename `from` to `to`. If the rename fails, try again until `retry_for`
/// is over, then return the last error.
fn rename_with_retry(
    rename: Rename<'_>,
    from: &Path,
    to: &Path,
    retry_for: Duration,
) -> io::Result<()> {
    let start = Instant::now();
    loop {
        match rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let elapsed = start.elapsed();
                if elapsed >= retry_for {
                    return Err(error);
                }
                std::thread::sleep((retry_for - elapsed).min(Duration::from_millis(50)));
            }
        }
    }
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Undo each backup folder that still has the "incomplete" flag: move its
/// entries back into the app data folder in reverse move order, then remove
/// the folder. A backup folder with no flag and no entries (a crash before
/// the flag) is removed too. Call this at launch before anything else.
///
/// If the app data folder already has an entry that the undo would move
/// back, nothing of that folder moves and the result is
/// [`BackupError::TargetExists`]. Returns the folders that were undone.
pub fn undo_incomplete(app_data: &Path) -> Result<Vec<PathBuf>, BackupError> {
    let mut undone = Vec::new();
    for folder in backup_folders(app_data)? {
        if folder.join(INCOMPLETE_FLAG).exists() {
            move_back(
                app_data,
                &folder,
                &MOVE_ORDER,
                &mut system_rename,
                RETRY_FOR,
            )?;
        } else if MOVE_ORDER.iter().any(|name| exists(&folder.join(name))) {
            continue;
        }
        fs::remove_dir_all(&folder).map_err(io_error("remove", &folder))?;
        undone.push(folder);
    }
    Ok(undone)
}

/// Move the entries named in `order` from `folder` back into `app_data`,
/// in reverse order. Refuses before the first rename if a target exists.
fn move_back(
    app_data: &Path,
    folder: &Path,
    order: &[&str],
    rename: Rename<'_>,
    retry_for: Duration,
) -> Result<(), BackupError> {
    let present: Vec<&str> = order
        .iter()
        .rev()
        .copied()
        .filter(|name| exists(&folder.join(name)))
        .collect();
    if let Some(name) = present.iter().find(|name| exists(&app_data.join(name))) {
        return Err(BackupError::TargetExists(app_data.join(name)));
    }
    for name in present {
        let from = folder.join(name);
        rename_with_retry(rename, &from, &app_data.join(name), retry_for)
            .map_err(io_error("move back", &from))?;
    }
    Ok(())
}

/// Move a `longhand.db-wal` or `longhand.db-shm` that has no `longhand.db`
/// into a new automatic backup, so SQLite never applies a stale WAL to a
/// new database. A zero-byte `longhand.db` holds nothing and counts as no
/// database. The working folder stays. Returns the backup path, or `None`
/// when there is no orphan.
pub fn move_orphan_wal(app_data: &Path) -> Result<Option<PathBuf>, BackupError> {
    let db_holds_data = fs::metadata(app_data.join(DB)).is_ok_and(|m| m.len() > 0);
    let orphan = ORPHAN_ORDER.iter().any(|name| exists(&app_data.join(name)));
    if db_holds_data || !orphan {
        return Ok(None);
    }
    move_entries(
        app_data,
        BackupKind::Automatic,
        &ORPHAN_ORDER,
        &mut system_rename,
        RETRY_FOR,
    )
    .map(Some)
}

/// Move the working folder and the database aside into a new backup folder
/// (R4). Entries that do not exist are skipped. Returns the backup path.
pub fn move_aside(app_data: &Path, kind: BackupKind) -> Result<PathBuf, BackupError> {
    move_aside_with(app_data, kind, &mut system_rename, RETRY_FOR)
}

/// [`move_aside`] with a given rename function and retry time. A rename
/// that still fails after `retry_for` undoes the renames that succeeded,
/// removes the backup folder and returns the error.
pub fn move_aside_with(
    app_data: &Path,
    kind: BackupKind,
    rename: Rename<'_>,
    retry_for: Duration,
) -> Result<PathBuf, BackupError> {
    move_entries(app_data, kind, &MOVE_ORDER, rename, retry_for)
}

fn move_entries(
    app_data: &Path,
    kind: BackupKind,
    order: &[&str],
    rename: Rename<'_>,
    retry_for: Duration,
) -> Result<PathBuf, BackupError> {
    let folder = make_unique_folder(app_data)?;
    if let Err(error) = fill(app_data, &folder, kind, order, rename, retry_for) {
        // Undo what moved. If the undo fails, the flag stays, and the undo
        // at the next launch finishes it.
        if move_back(app_data, &folder, order, rename, retry_for).is_ok() {
            let _ = fs::remove_dir_all(&folder);
        }
        return Err(error);
    }
    Ok(folder)
}

fn fill(
    app_data: &Path,
    folder: &Path,
    kind: BackupKind,
    order: &[&str],
    rename: Rename<'_>,
    retry_for: Duration,
) -> Result<(), BackupError> {
    let flag = folder.join(INCOMPLETE_FLAG);
    write_synced(&flag, INCOMPLETE_TEXT)?;
    if kind == BackupKind::ChosenInAlert {
        write_synced(&folder.join(REBUILD_MARKER), MARKER_TEXT)?;
    }
    // Put the flag, the marker and the new folders on the disk before the
    // first rename. Otherwise, after a power loss, the disk can show the
    // renamed entries without the flag, and the undo at the next launch
    // skips the folder.
    sync_dir(folder)?;
    sync_dir(&app_data.join(BACKUPS_DIR))?;
    sync_dir(app_data)?;
    for name in order {
        let from = app_data.join(name);
        if !exists(&from) {
            continue;
        }
        rename_with_retry(rename, &from, &folder.join(name), retry_for)
            .map_err(io_error("move", &from))?;
    }
    // Put the renames on the disk before the flag goes. Otherwise, after a
    // power loss, the disk can show no flag and only some of the entries.
    sync_dir(folder)?;
    sync_dir(app_data)?;
    fs::remove_file(&flag).map_err(io_error("remove", &flag))
}

/// Write `text` to a new file at `path` and put the file on the disk.
fn write_synced(path: &Path, text: &str) -> Result<(), BackupError> {
    let mut file = fs::File::create(path).map_err(io_error("write", path))?;
    file.write_all(text.as_bytes())
        .map_err(io_error("write", path))?;
    file.sync_all().map_err(io_error("sync", path))
}

/// Put the entries of the directory `dir` on the disk, so that a new or
/// renamed entry stays after a power loss.
#[cfg(not(windows))]
fn sync_dir(dir: &Path) -> Result<(), BackupError> {
    fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(io_error("sync", dir))
}

/// On Windows, `File::open` cannot open a directory, and NTFS records
/// directory changes in its journal. This function does nothing there.
#[cfg(windows)]
fn sync_dir(_dir: &Path) -> Result<(), BackupError> {
    Ok(())
}

/// Make `database-backups/<UTC time>/`. A second folder in the same second
/// gets the suffix `-2`, then `-3`, and so on.
fn make_unique_folder(app_data: &Path) -> Result<PathBuf, BackupError> {
    let root = app_data.join(BACKUPS_DIR);
    fs::create_dir_all(&root).map_err(io_error("make", &root))?;
    let time = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string();
    for n in 1u32.. {
        let name = if n == 1 {
            time.clone()
        } else {
            format!("{time}-{n}")
        };
        let folder = root.join(name);
        match fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_error("make", &folder)(e)),
        }
    }
    unreachable!("the suffixes do not run out")
}

/// Every folder in `database-backups/`. A missing `database-backups/` has
/// no folders.
fn backup_folders(app_data: &Path) -> Result<Vec<PathBuf>, BackupError> {
    let root = app_data.join(BACKUPS_DIR);
    let read = match fs::read_dir(&root) {
        Ok(read) => read,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_error("read", &root)(e)),
    };
    let mut folders = Vec::new();
    for entry in read {
        let entry = entry.map_err(io_error("read", &root))?;
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            folders.push(entry.path());
        }
    }
    Ok(folders)
}

/// The sort key of a backup folder name: the UTC time, then the numeric
/// suffix (none is 1). Thus `-10` sorts after `-9`.
fn age_key(folder: &Path) -> (String, u32) {
    let name = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match name.rsplit_once('-') {
        Some((time, n)) if time.ends_with('Z') => match n.parse() {
            Ok(n) => (time.to_string(), n),
            Err(_) => (name, 1),
        },
        _ => (name, 1),
    }
}

/// Delete the automatic backups beyond the newest [`KEEP_AUTOMATIC`] (R11).
/// A backup with the marker is never deleted. A backup with the
/// "incomplete" flag is never touched. Returns the deleted folders.
pub fn apply_retention(app_data: &Path) -> Result<Vec<PathBuf>, BackupError> {
    let mut automatic: Vec<PathBuf> = backup_folders(app_data)?
        .into_iter()
        .filter(|f| !f.join(INCOMPLETE_FLAG).exists() && !f.join(REBUILD_MARKER).exists())
        .collect();
    automatic.sort_by_key(|f| std::cmp::Reverse(age_key(f)));
    let old = automatic.split_off(KEEP_AUTOMATIC.min(automatic.len()));
    for folder in &old {
        fs::remove_dir_all(folder).map_err(io_error("remove", folder))?;
    }
    Ok(old)
}
