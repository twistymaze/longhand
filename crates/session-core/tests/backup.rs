//! Move the database and the working folder aside (U3, KTD4).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use session_core::Store;
use session_core::backup::{
    BACKUPS_DIR, BackupError, BackupKind, Cleanup, INCOMPLETE_FLAG, REBUILD_MARKER,
    apply_retention, clean_backups, move_aside, move_aside_with, move_orphan_wal, undo_incomplete,
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
fn an_undo_removes_a_backup_folder_with_no_flag_and_no_entries() {
    let app = tempfile::tempdir().unwrap();
    // A crash after the folder was made, before the flag was written.
    let empty = app.path().join(BACKUPS_DIR).join("2026-09-30T15-28-00Z");
    fs::create_dir_all(&empty).unwrap();

    let undone = undo_incomplete(app.path()).unwrap();

    assert_eq!(undone, vec![empty.clone()]);
    assert!(!empty.exists());
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
fn a_lone_shm_moves_aside() {
    let app = tempfile::tempdir().unwrap();
    fs::write(app.path().join(SHM), b"shm bytes").unwrap();

    let backup = move_orphan_wal(app.path()).unwrap().expect("a backup");

    assert!(!app.path().join(SHM).exists());
    assert_eq!(fs::read(backup.join(SHM)).unwrap(), b"shm bytes");
    assert!(!backup.join(WAL).exists());
    assert!(!backup.join(INCOMPLETE_FLAG).exists());
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

// ------------------------------------------------ cleanup of note content (KTD15)

/// Text that only the recognition rows and the summary cache hold.
const TRANSCRIPT_TEXT: &str = "the budget for the spring term is final";

/// The checkpoints that [`add_note_content`] writes, and whether the
/// cleanup deletes each one of a saved note.
const CONTENT_CHECKPOINTS: [(&str, &str, bool); 8] = [
    ("assemble", "done", true),
    ("transcribe", "w1", true),
    ("diarize", "d1", true),
    ("previous_transcribe", "w1", true),
    ("previous_summarize", "chunk-0", true),
    ("summarize", "chunk-0", true),
    ("summarize", "failed", false),
    ("join", "done", false),
];

/// The rows that the cleanup deletes for one saved note with recognition
/// tables: six checkpoints, one recognition output row and one epoch row.
const SAVED_NOTE_ROWS: usize = 8;

const SAVED: &str = r#"{"state":"saved"}"#;
const CAPTURING: &str = r#"{"state":"capturing"}"#;

/// Make `<app>/longhand.db` as a build with `version` migrations left it,
/// in WAL mode as the store keeps it, with a saved note `saved` and an
/// unfinished note `open`. Then move it aside into a backup, as a rebuild
/// does. Returns the backup folder.
fn old_backup(app: &Path, version: usize) -> PathBuf {
    {
        let mut conn = Connection::open(app.join(DB)).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        session_core::schema::migrations()
            .to_version(&mut conn, version)
            .unwrap();
        let note = if version >= 22 {
            "note_id"
        } else {
            "session_id"
        };
        let recognition = version >= 10;
        add_note_content(&conn, note, recognition, "saved", SAVED);
        add_note_content(&conn, note, recognition, "open", CAPTURING);
    }
    move_aside(app, BackupKind::Automatic).unwrap()
}

/// Add the note `id` in `state` with a row of note content in each table,
/// with the note column `note`. The rows that the cleanup deletes hold
/// [`TRANSCRIPT_TEXT`].
fn add_note_content(conn: &Connection, note: &str, recognition: bool, id: &str, state: &str) {
    conn.execute(
        "INSERT INTO sessions (id, created_at, state) VALUES (?1, ?2, ?3)",
        rusqlite::params![id, "2026-09-22T10:00:00-05:00", state],
    )
    .unwrap();
    let text = serde_json::to_string(&TRANSCRIPT_TEXT.repeat(30)).unwrap();
    for (stage, key, deleted) in CONTENT_CHECKPOINTS {
        conn.execute(
            &format!(
                "INSERT INTO checkpoints ({note}, stage, key, payload) VALUES (?1, ?2, ?3, ?4)"
            ),
            rusqlite::params![id, stage, key, if deleted { text.as_str() } else { "{}" }],
        )
        .unwrap();
    }
    if recognition {
        conn.execute(
            &format!(
                "INSERT INTO recognition_output ({note}, generation, key, revision, payload)
                 VALUES (?1, 'g', 'window-0', 1, ?2)"
            ),
            rusqlite::params![id, text],
        )
        .unwrap();
        conn.execute(
            &format!(
                "INSERT INTO recognition_epochs ({note}, generation, next_epoch)
                 VALUES (?1, 'g', 1)"
            ),
            [id],
        )
        .unwrap();
    }
}

/// The number of rows of `id` in each table of note content that `db` has.
fn content_rows(db: &Path, id: &str) -> usize {
    let conn = Connection::open(db).unwrap();
    let mut rows = 0;
    for table in ["checkpoints", "recognition_output", "recognition_epochs"] {
        let note: Option<String> = conn
            .query_row(
                "SELECT name FROM pragma_table_info(?1)
                 WHERE name IN ('note_id', 'session_id')",
                [table],
                |r| r.get(0),
            )
            .ok();
        if let Some(note) = note {
            rows += conn
                .query_row(
                    &format!("SELECT count(*) FROM {table} WHERE {note} = ?1"),
                    [id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap() as usize;
        }
    }
    rows
}

/// Whether a file of the backup database holds `text`.
fn backup_holds(folder: &Path, text: &str) -> bool {
    [DB, WAL].iter().any(|name| {
        fs::read(folder.join(name))
            .map(|bytes| bytes.windows(text.len()).any(|w| w == text.as_bytes()))
            .unwrap_or(false)
    })
}

#[test]
fn a_backup_from_the_previous_schema_loses_the_content_of_its_saved_notes() {
    // AE13: one rebuild backup from before the update.
    let app = tempfile::tempdir().unwrap();
    let backup = old_backup(app.path(), 24);
    let open_rows = content_rows(&backup.join(DB), "open");
    assert!(backup_holds(&backup, TRANSCRIPT_TEXT));

    let results = clean_backups(app.path(), true).unwrap();

    assert_eq!(
        results,
        vec![(
            backup.clone(),
            Cleanup::Cleaned {
                removed: SAVED_NOTE_ROWS,
                vacuumed: true
            }
        )]
    );
    // The summary failure and the Join row stay.
    assert_eq!(content_rows(&backup.join(DB), "saved"), 2);
    assert_eq!(content_rows(&backup.join(DB), "open"), open_rows);
    // The VACUUM left no free pages.
    let conn = Connection::open(backup.join(DB)).unwrap();
    let free: i64 = conn
        .pragma_query_value(None, "freelist_count", |r| r.get(0))
        .unwrap();
    assert_eq!(free, 0);
}

#[test]
fn a_second_launch_deletes_no_rows_and_runs_no_vacuum() {
    let app = tempfile::tempdir().unwrap();
    let backup = old_backup(app.path(), 24);
    clean_backups(app.path(), true).unwrap();

    let results = clean_backups(app.path(), true).unwrap();

    assert_eq!(
        results,
        vec![(
            backup,
            Cleanup::Cleaned {
                removed: 0,
                vacuumed: false
            }
        )]
    );
}

#[test]
fn after_the_cleanup_no_file_of_the_backup_holds_the_transcript() {
    let app = tempfile::tempdir().unwrap();
    {
        let mut conn = Connection::open(app.path().join(DB)).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        session_core::schema::migrations()
            .to_version(&mut conn, 24)
            .unwrap();
        add_note_content(&conn, "note_id", true, "saved", SAVED);
    }
    let backup = move_aside(app.path(), BackupKind::Automatic).unwrap();
    assert!(backup_holds(&backup, TRANSCRIPT_TEXT));

    clean_backups(app.path(), true).unwrap();

    assert!(!backup_holds(&backup, TRANSCRIPT_TEXT));
}

#[test]
fn a_backup_from_before_the_note_id_columns_is_cleaned() {
    let app = tempfile::tempdir().unwrap();
    let backup = old_backup(app.path(), 21);
    let open_rows = content_rows(&backup.join(DB), "open");

    let results = clean_backups(app.path(), true).unwrap();

    assert_eq!(
        results[0].1,
        Cleanup::Cleaned {
            removed: SAVED_NOTE_ROWS,
            vacuumed: true
        }
    );
    assert_eq!(content_rows(&backup.join(DB), "saved"), 2);
    assert_eq!(content_rows(&backup.join(DB), "open"), open_rows);
}

#[test]
fn a_backup_with_no_recognition_tables_is_cleaned() {
    // Migration 0010 made recognition_output and recognition_epochs.
    let app = tempfile::tempdir().unwrap();
    let backup = old_backup(app.path(), 9);

    let results = clean_backups(app.path(), true).unwrap();

    assert_eq!(
        results[0].1,
        Cleanup::Cleaned {
            removed: SAVED_NOTE_ROWS - 2,
            vacuumed: true
        }
    );
    assert_eq!(content_rows(&backup.join(DB), "saved"), 2);
}

/// A backup folder whose database is not a database.
fn unreadable_backup(app: &Path) -> PathBuf {
    fs::write(app.join(DB), b"these bytes are not a database".repeat(200)).unwrap();
    move_aside(app, BackupKind::ChosenInAlert).unwrap()
}

#[test]
fn a_backup_that_cannot_be_opened_is_deleted_when_the_store_passed_its_check() {
    let app = tempfile::tempdir().unwrap();
    let backup = unreadable_backup(app.path());

    let results = clean_backups(app.path(), true).unwrap();

    assert!(matches!(results[..], [(ref f, Cleanup::Deleted { .. })] if *f == backup));
    assert!(!backup.exists());
}

#[test]
fn a_backup_that_cannot_be_opened_stays_when_the_store_did_not_pass_its_check() {
    let app = tempfile::tempdir().unwrap();
    let backup = unreadable_backup(app.path());

    let results = clean_backups(app.path(), false).unwrap();

    assert!(matches!(results[..], [(ref f, Cleanup::Kept { .. })] if *f == backup));
    assert!(backup.join(DB).exists());
}

#[test]
fn a_backup_with_a_wal_and_no_database_is_deleted_when_the_store_passed_its_check() {
    let app = tempfile::tempdir().unwrap();
    fs::write(app.path().join(WAL), TRANSCRIPT_TEXT).unwrap();
    let backup = move_orphan_wal(app.path()).unwrap().unwrap();

    let results = clean_backups(app.path(), true).unwrap();

    assert!(matches!(results[..], [(_, Cleanup::Deleted { .. })]));
    assert!(!backup.exists());
}

#[test]
fn a_backup_with_no_database_and_no_wal_is_not_touched() {
    let app = tempfile::tempdir().unwrap();
    fs::create_dir_all(app.path().join(SESSIONS).join("s1")).unwrap();
    let backup = move_aside(app.path(), BackupKind::Automatic).unwrap();

    assert!(clean_backups(app.path(), true).unwrap().is_empty());
    assert!(backup.join(SESSIONS).exists());
}

#[test]
fn a_new_store_passes_its_check() {
    let app = tempfile::tempdir().unwrap();
    let store = Store::open(&app.path().join(DB)).unwrap();
    assert!(store.quick_check().unwrap());
}
