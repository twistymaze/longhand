//! Move the database and the working folder aside (U3, KTD4).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use session_core::Store;
use session_core::backup::{
    BACKUPS_DIR, BackupError, BackupKind, INCOMPLETE_FLAG, REBUILD_MARKER, apply_retention,
    move_aside, move_aside_with, move_orphan_wal, undo_incomplete,
};

const DB: &str = "longhand.db";
const WAL: &str = "longhand.db-wal";
const SHM: &str = "longhand.db-shm";
const SESSIONS: &str = "sessions";

/// Write the four entries of an app data folder with known bytes.
fn populate(dir: &Path) {
    fs::write(dir.join(DB), b"database bytes").unwrap();
    fs::write(dir.join(WAL), b"wal bytes").unwrap();
    fs::write(dir.join(SHM), b"shm bytes").unwrap();
    fs::create_dir_all(dir.join(SESSIONS).join("s1")).unwrap();
    fs::write(
        dir.join(SESSIONS).join("s1").join("audio.wav"),
        b"audio bytes",
    )
    .unwrap();
}

fn assert_populated(dir: &Path) {
    assert_eq!(fs::read(dir.join(DB)).unwrap(), b"database bytes");
    assert_eq!(fs::read(dir.join(WAL)).unwrap(), b"wal bytes");
    assert_eq!(fs::read(dir.join(SHM)).unwrap(), b"shm bytes");
    assert_eq!(
        fs::read(dir.join(SESSIONS).join("s1").join("audio.wav")).unwrap(),
        b"audio bytes"
    );
}

fn assert_empty_of_entries(dir: &Path) {
    for name in [DB, WAL, SHM, SESSIONS] {
        assert!(
            !dir.join(name).exists(),
            "{name} is still in {}",
            dir.display()
        );
    }
}

fn backup_folders(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir.join(BACKUPS_DIR)) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = read.map(|e| e.unwrap().path()).collect();
    out.sort();
    out
}

/// Make `<app>/longhand.db` and `<app>/longhand.db-wal` where the rows of
/// table `t` are only in the WAL: the files are copied while the writer is
/// open, before any checkpoint.
fn db_with_rows_only_in_wal(app: &Path) {
    let source = tempfile::tempdir().unwrap();
    let conn = Connection::open(source.path().join(DB)).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    conn.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1), (2), (3);")
        .unwrap();
    fs::copy(source.path().join(DB), app.join(DB)).unwrap();
    fs::copy(source.path().join(WAL), app.join(WAL)).unwrap();
    drop(conn);
}

fn rows_in_t(db: &Path) -> i64 {
    let conn = Connection::open(db).unwrap();
    let tables: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 't'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    if tables == 0 {
        return 0;
    }
    conn.query_row("SELECT count(*) FROM t", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_move_takes_all_four_entries_with_the_same_bytes() {
    let app = tempfile::tempdir().unwrap();
    populate(app.path());

    let backup = move_aside(app.path(), BackupKind::Automatic).unwrap();

    assert_empty_of_entries(app.path());
    assert_eq!(backup.parent().unwrap(), app.path().join(BACKUPS_DIR));
    assert_populated(&backup);
    assert!(!backup.join(INCOMPLETE_FLAG).exists());
    assert!(!backup.join(REBUILD_MARKER).exists());

    // The next Store::open starts from an empty database.
    let store = Store::open(&app.path().join(DB)).unwrap();
    assert!(store.sessions().unwrap().is_empty());
}

#[test]
fn a_backup_chosen_in_an_alert_has_the_marker() {
    let app = tempfile::tempdir().unwrap();
    populate(app.path());
    let backup = move_aside(app.path(), BackupKind::ChosenInAlert).unwrap();
    assert!(backup.join(REBUILD_MARKER).is_file());
    assert!(!backup.join(INCOMPLETE_FLAG).exists());
}

#[test]
fn a_move_with_no_working_folder_or_wal_succeeds() {
    let app = tempfile::tempdir().unwrap();
    fs::write(app.path().join(DB), b"database bytes").unwrap();

    let backup = move_aside(app.path(), BackupKind::Automatic).unwrap();

    assert_empty_of_entries(app.path());
    assert_eq!(fs::read(backup.join(DB)).unwrap(), b"database bytes");
    assert!(!backup.join(WAL).exists());
    assert!(!backup.join(SESSIONS).exists());
}

#[test]
fn a_failure_at_the_database_rename_undoes_the_other_renames() {
    let app = tempfile::tempdir().unwrap();
    populate(app.path());

    let mut rename = |from: &Path, to: &Path| -> io::Result<()> {
        if from.file_name().unwrap() == DB {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected"));
        }
        fs::rename(from, to)
    };
    let result = move_aside_with(
        app.path(),
        BackupKind::Automatic,
        &mut rename,
        Duration::from_millis(20),
    );

    assert!(result.is_err(), "the move should fail");
    assert_populated(app.path());
    assert!(backup_folders(app.path()).is_empty());
}

#[test]
fn a_crashed_move_is_undone_at_the_next_launch() {
    let app = tempfile::tempdir().unwrap();
    db_with_rows_only_in_wal(app.path());
    fs::create_dir_all(app.path().join(SESSIONS).join("s1")).unwrap();
    fs::write(app.path().join(SESSIONS).join("s1").join("a.wav"), b"a").unwrap();

    // A crash after the renames of sessions/ and -wal, before longhand.db.
    let half = app.path().join(BACKUPS_DIR).join("2026-09-30T15-28-00Z");
    fs::create_dir_all(&half).unwrap();
    fs::write(half.join(INCOMPLETE_FLAG), b"").unwrap();
    fs::rename(app.path().join(SESSIONS), half.join(SESSIONS)).unwrap();
    fs::rename(app.path().join(WAL), half.join(WAL)).unwrap();

    let undone = undo_incomplete(app.path()).unwrap();

    assert_eq!(undone, vec![half.clone()]);
    assert!(!half.exists());
    assert!(app.path().join(DB).is_file());
    assert!(app.path().join(WAL).is_file());
    assert_eq!(
        fs::read(app.path().join(SESSIONS).join("s1").join("a.wav")).unwrap(),
        b"a"
    );
    assert_eq!(rows_in_t(&app.path().join(DB)), 3);
}

#[test]
fn an_undo_never_overwrites_an_entry_in_the_app_data_folder() {
    let app = tempfile::tempdir().unwrap();
    let half = app.path().join(BACKUPS_DIR).join("2026-09-30T15-28-00Z");
    fs::create_dir_all(&half).unwrap();
    fs::write(half.join(INCOMPLETE_FLAG), b"").unwrap();
    fs::write(half.join(WAL), b"old wal").unwrap();
    fs::write(app.path().join(WAL), b"new wal").unwrap();

    let error = undo_incomplete(app.path()).unwrap_err();

    assert!(matches!(error, BackupError::TargetExists(_)), "{error:?}");
    assert_eq!(fs::read(app.path().join(WAL)).unwrap(), b"new wal");
    assert_eq!(fs::read(half.join(WAL)).unwrap(), b"old wal");
    assert!(half.join(INCOMPLETE_FLAG).exists());
}

#[test]
fn an_undo_leaves_complete_backups_alone() {
    let app = tempfile::tempdir().unwrap();
    populate(app.path());
    let backup = move_aside(app.path(), BackupKind::Automatic).unwrap();

    assert!(undo_incomplete(app.path()).unwrap().is_empty());
    assert_populated(&backup);
    assert_empty_of_entries(app.path());
}

#[test]
fn an_orphan_wal_moves_aside_and_a_new_database_has_none_of_its_rows() {
    let app = tempfile::tempdir().unwrap();
    db_with_rows_only_in_wal(app.path());
    fs::remove_file(app.path().join(DB)).unwrap();

    let backup = move_orphan_wal(app.path()).unwrap().expect("a backup");

    assert!(!app.path().join(WAL).exists());
    assert!(backup.join(WAL).is_file());
    assert!(!backup.join(INCOMPLETE_FLAG).exists());
    drop(Store::open(&app.path().join(DB)).unwrap());
    assert_eq!(rows_in_t(&app.path().join(DB)), 0);
}

#[test]
fn a_wal_with_its_database_is_not_an_orphan() {
    let app = tempfile::tempdir().unwrap();
    populate(app.path());
    assert!(move_orphan_wal(app.path()).unwrap().is_none());
    assert_populated(app.path());

    let empty = tempfile::tempdir().unwrap();
    assert!(move_orphan_wal(empty.path()).unwrap().is_none());
    assert!(backup_folders(empty.path()).is_empty());
}

#[test]
fn two_moves_in_the_same_second_get_two_folders() {
    let app = tempfile::tempdir().unwrap();
    fs::write(app.path().join(DB), b"first").unwrap();
    let first = move_aside(app.path(), BackupKind::Automatic).unwrap();
    fs::write(app.path().join(DB), b"second").unwrap();
    let second = move_aside(app.path(), BackupKind::Automatic).unwrap();

    assert_ne!(first, second);
    assert_eq!(fs::read(first.join(DB)).unwrap(), b"first");
    assert_eq!(fs::read(second.join(DB)).unwrap(), b"second");
}

#[test]
fn retention_keeps_the_newest_three_automatic_backups_and_every_marked_one() {
    let app = tempfile::tempdir().unwrap();
    let kinds = [
        BackupKind::ChosenInAlert,
        BackupKind::Automatic,
        BackupKind::Automatic,
        BackupKind::Automatic,
        BackupKind::ChosenInAlert,
        BackupKind::Automatic,
        BackupKind::Automatic,
    ];
    let mut made = Vec::new();
    for kind in kinds {
        fs::write(app.path().join(DB), b"db").unwrap();
        made.push((kind, move_aside(app.path(), kind).unwrap()));
    }
    // An incomplete backup is never touched.
    let incomplete = app.path().join(BACKUPS_DIR).join("0000-incomplete");
    fs::create_dir_all(&incomplete).unwrap();
    fs::write(incomplete.join(INCOMPLETE_FLAG), b"").unwrap();

    let mut deleted = apply_retention(app.path()).unwrap();
    deleted.sort();

    let automatic: Vec<&PathBuf> = made
        .iter()
        .filter(|(k, _)| *k == BackupKind::Automatic)
        .map(|(_, p)| p)
        .collect();
    let mut expected_deleted = vec![automatic[0].clone(), automatic[1].clone()];
    expected_deleted.sort();
    assert_eq!(deleted, expected_deleted);

    let mut expected_kept: Vec<PathBuf> = made
        .iter()
        .map(|(_, p)| p.clone())
        .filter(|p| !expected_deleted.contains(p))
        .collect();
    expected_kept.push(incomplete);
    expected_kept.sort();
    assert_eq!(backup_folders(app.path()), expected_kept);
}

#[cfg(unix)]
#[test]
fn a_failed_rename_in_a_read_only_folder_leaves_the_files_in_place() {
    use std::os::unix::fs::PermissionsExt;

    let app = tempfile::tempdir().unwrap();
    populate(app.path());
    fs::create_dir(app.path().join(BACKUPS_DIR)).unwrap();
    fs::set_permissions(app.path(), fs::Permissions::from_mode(0o555)).unwrap();

    let mut rename = |from: &Path, to: &Path| fs::rename(from, to);
    let result = move_aside_with(
        app.path(),
        BackupKind::Automatic,
        &mut rename,
        Duration::from_millis(20),
    );

    fs::set_permissions(app.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err(), "the move should fail");
    assert_populated(app.path());
    assert!(backup_folders(app.path()).is_empty());
}
