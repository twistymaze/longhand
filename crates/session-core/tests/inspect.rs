//! U2: the read-only inspection of `longhand.db` at launch (KTD1, KTD3).
//! Each test builds the app data folder with `Store::open` or raw SQL.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use session_core::Store;
use session_core::inspect::{Inspection, NotOwnReason, UnfinishedWork, inspect};
use session_core::schema;

const OWN_COUNT: i64 = schema::MIGRATION_SQL.len() as i64;

fn db(dir: &Path) -> PathBuf {
    dir.join("longhand.db")
}

fn side(dir: &Path, suffix: &str) -> PathBuf {
    dir.join(format!("longhand.db{suffix}"))
}

/// A database that this build made, closed. Close removes `-wal` and `-shm`.
fn own_db(dir: &Path) {
    drop(Store::open(&db(dir)).unwrap());
}

/// A database that a newer build left: this build's schema, at a version
/// above this build's count.
fn newer_db(dir: &Path) -> Connection {
    own_db(dir);
    let conn = Connection::open(db(dir)).unwrap();
    conn.pragma_update(None, "user_version", OWN_COUNT + 1)
        .unwrap();
    conn
}

fn add_session(conn: &Connection, id: &str, state: &str) {
    conn.execute(
        "INSERT INTO sessions (id, created_at, state) VALUES (?1, ?2, ?3)",
        params![id, "2026-09-30T10:00:00-05:00", state],
    )
    .unwrap();
}

fn not_own(result: Inspection) -> (NotOwnReason, UnfinishedWork) {
    match result {
        Inspection::NotOwn { reason, work } => (reason, work),
        other => panic!("expected NotOwn, got {other:?}"),
    }
}

/// The unfinished work of a newer database after `fill` runs on it.
fn work_after(fill: impl FnOnce(&Path, &Connection)) -> UnfinishedWork {
    let dir = tempfile::tempdir().unwrap();
    let conn = newer_db(dir.path());
    fill(dir.path(), &conn);
    drop(conn);
    not_own(inspect(dir.path())).1
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// ------------------------------------------------------------ ownership

#[test]
fn a_newer_version_with_only_saved_notes_is_not_own_with_no_work() {
    // AE1.
    let dir = tempfile::tempdir().unwrap();
    let conn = newer_db(dir.path());
    add_session(&conn, "a", r#"{"state":"saved"}"#);
    add_session(&conn, "b", r#"{"state":"discarded"}"#);
    drop(conn);
    let (reason, work) = not_own(inspect(dir.path()));
    assert_eq!(
        reason,
        NotOwnReason::NewerVersion {
            found: OWN_COUNT + 1,
            own: OWN_COUNT
        }
    );
    assert_eq!(work, UnfinishedWork::default());
    assert!(work.is_empty());
}

#[test]
fn a_different_hash_at_the_last_version_is_not_own() {
    // AE2.
    let dir = tempfile::tempdir().unwrap();
    own_db(dir.path());
    let conn = Connection::open(db(dir.path())).unwrap();
    conn.execute(
        "UPDATE applied_migrations SET sha256 = 'other' WHERE version = ?1",
        [OWN_COUNT],
    )
    .unwrap();
    add_session(&conn, "a", r#"{"state":"saved"}"#);
    drop(conn);
    let (reason, work) = not_own(inspect(dir.path()));
    assert_eq!(
        reason,
        NotOwnReason::DifferentMigration { version: OWN_COUNT }
    );
    assert!(work.is_empty());
}

#[test]
fn a_database_that_this_build_made_is_own() {
    let dir = tempfile::tempdir().unwrap();
    own_db(dir.path());
    assert_eq!(inspect(dir.path()), Inspection::Own);
}

#[test]
fn a_database_with_no_applied_migrations_table_is_own() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(db(dir.path())).unwrap();
    schema::migrations().to_latest(&mut conn).unwrap();
    drop(conn);
    assert_eq!(inspect(dir.path()), Inspection::Own);
}

#[test]
fn an_older_version_is_own() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(db(dir.path())).unwrap();
    schema::migrations().to_version(&mut conn, 3).unwrap();
    drop(conn);
    assert_eq!(inspect(dir.path()), Inspection::Own);
}

#[test]
fn a_missing_file_is_own_and_no_file_is_made() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(inspect(dir.path()), Inspection::Own);
    assert!(files(dir.path()).is_empty());
}

#[test]
fn a_zero_byte_file_is_own_and_no_file_is_made() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(db(dir.path()), b"").unwrap();
    assert_eq!(inspect(dir.path()), Inspection::Own);
    assert_eq!(files(dir.path()), vec!["longhand.db".to_string()]);
    assert_eq!(fs::metadata(db(dir.path())).unwrap().len(), 0);
}

#[test]
fn a_file_that_is_not_a_database_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(db(dir.path()), vec![b'x'; 4096]).unwrap();
    match inspect(dir.path()) {
        Inspection::Unreadable { error } => assert!(!error.is_empty()),
        other => panic!("expected Unreadable, got {other:?}"),
    }
}

// ------------------------------------------------------------ unfinished work

#[test]
fn a_queued_note_is_one_unfinished_note() {
    // AE3.
    let work = work_after(|_, conn| {
        add_session(
            conn,
            "a",
            r#"{"state":"queued","stopped_at":"2026-09-30T10:00:00-05:00"}"#,
        );
        add_session(conn, "b", r#"{"state":"saved"}"#);
    });
    assert_eq!(
        work,
        UnfinishedWork {
            notes: 1,
            ..Default::default()
        }
    );
    assert!(!work.is_empty());
}

#[test]
fn each_unfinished_state_is_an_unfinished_note() {
    for state in [
        r#"{"state":"capturing"}"#,
        r#"{"state":"interrupted","reason":"device","since":"2026-09-30T10:00:00-05:00","acknowledged":false}"#,
        r#"{"state":"processing_failed","stopped_at":"2026-09-30T10:00:00-05:00","stage":"transcribe"}"#,
        r#"{"state":"awaiting_destination","stopped_at":"2026-09-30T10:00:00-05:00","problem":"x"}"#,
        r#"{"state":"a_state_from_a_newer_build"}"#,
    ] {
        let work = work_after(|_, conn| add_session(conn, "a", state));
        assert_eq!(work.notes, 1, "{state}");
        assert!(!work.is_empty(), "{state}");
    }
}

#[test]
fn a_jobs_row_is_one_job() {
    let work = work_after(|_, conn| {
        add_session(conn, "a", r#"{"state":"saved"}"#);
        conn.execute(
            "INSERT INTO jobs (note_id, stage) VALUES ('a', 'transcribe')",
            [],
        )
        .unwrap();
    });
    assert_eq!(
        work,
        UnfinishedWork {
            jobs: 1,
            ..Default::default()
        }
    );
}

#[test]
fn a_transcribe_again_model_is_one_job() {
    let work = work_after(|_, conn| {
        add_session(conn, "a", r#"{"state":"saved"}"#);
        conn.execute(
            "UPDATE sessions SET transcribe_again_model = 'm' WHERE id = 'a'",
            [],
        )
        .unwrap();
    });
    assert_eq!(
        work,
        UnfinishedWork {
            jobs: 1,
            ..Default::default()
        }
    );
}

#[test]
fn a_move_journal_row_is_an_open_move() {
    let work = work_after(|_, conn| {
        conn.execute(
            "INSERT INTO move_journal (id, old_folder, new_folder, state)
             VALUES (1, '/old', '/new', 'running')",
            [],
        )
        .unwrap();
    });
    assert_eq!(
        work,
        UnfinishedWork {
            open_move: true,
            ..Default::default()
        }
    );
}

#[test]
fn a_move_notes_row_is_an_open_move() {
    let work = work_after(|_, conn| {
        add_session(conn, "a", r#"{"state":"saved"}"#);
        conn.execute(
            "INSERT INTO move_notes (note_id, step) VALUES ('a', '\"pending\"')",
            [],
        )
        .unwrap();
    });
    assert!(work.open_move);
}

#[test]
fn a_rename_intents_row_is_one_rename() {
    let work = work_after(|_, conn| {
        add_session(conn, "a", r#"{"state":"saved"}"#);
        conn.execute(
            "INSERT INTO rename_intents (note_id, from_path, to_path)
             VALUES ('a', '/x', '/y')",
            [],
        )
        .unwrap();
    });
    assert_eq!(
        work,
        UnfinishedWork {
            renames: 1,
            ..Default::default()
        }
    );
}

#[test]
fn a_summary_publication_row_is_one_publication() {
    let work = work_after(|_, conn| {
        add_session(conn, "a", r#"{"state":"saved"}"#);
        conn.execute(
            "INSERT INTO result_state (note_id, key, value)
             VALUES ('a', 'summary-publication', '\"/x\"'),
                    ('a', 'some-other-key', '1')",
            [],
        )
        .unwrap();
    });
    assert_eq!(
        work,
        UnfinishedWork {
            publications: 1,
            ..Default::default()
        }
    );
}

#[test]
fn a_publication_json_file_is_one_publication() {
    let work = work_after(|dir, _| {
        let note = dir.join("sessions").join("a");
        fs::create_dir_all(&note).unwrap();
        fs::write(note.join("publication.json"), b"{}").unwrap();
        fs::write(note.join("notes.md"), b"x").unwrap();
    });
    assert_eq!(
        work,
        UnfinishedWork {
            publications: 1,
            ..Default::default()
        }
    );
}

#[test]
fn a_summary_folder_is_one_publication() {
    let work = work_after(|dir, _| {
        let sessions = dir.join("sessions");
        fs::create_dir_all(sessions.join(".longhand-summary-abc")).unwrap();
        fs::create_dir_all(sessions.join("b")).unwrap();
    });
    assert_eq!(
        work,
        UnfinishedWork {
            publications: 1,
            ..Default::default()
        }
    );
}

#[test]
fn a_schema_with_missing_tables_and_columns_is_inspected_without_an_error() {
    // Version 3 has no move_journal, result_state or transcribe_again_model.
    let dir = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(db(dir.path())).unwrap();
    schema::migrations().to_version(&mut conn, 3).unwrap();
    add_session(
        &conn,
        "a",
        r#"{"state":"queued","stopped_at":"2026-09-30T10:00:00-05:00"}"#,
    );
    conn.pragma_update(None, "user_version", OWN_COUNT + 5)
        .unwrap();
    drop(conn);
    let (reason, work) = not_own(inspect(dir.path()));
    assert!(matches!(reason, NotOwnReason::NewerVersion { .. }));
    assert_eq!(
        work,
        UnfinishedWork {
            notes: 1,
            ..Default::default()
        }
    );
}

#[test]
fn a_database_with_no_sessions_table_is_inspected_without_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(db(dir.path())).unwrap();
    conn.execute_batch("CREATE TABLE other (x INTEGER); PRAGMA user_version = 99;")
        .unwrap();
    drop(conn);
    let (_, work) = not_own(inspect(dir.path()));
    assert!(work.is_empty());
}

// ------------------------------------------------------------ no change

/// A newer database in WAL mode whose rows are only in `-wal`. The writer
/// stays open, so the WAL is not checkpointed.
fn wal_only_db(dir: &Path) -> Connection {
    drop(newer_db(dir));
    let conn = Connection::open(db(dir)).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    add_session(
        &conn,
        "a",
        r#"{"state":"queued","stopped_at":"2026-09-30T10:00:00-05:00"}"#,
    );
    conn
}

#[test]
fn rows_only_in_the_wal_are_read_while_a_writer_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let writer = wal_only_db(dir.path());
    assert!(side(dir.path(), "-shm").exists());
    let (_, work) = not_own(inspect(dir.path()));
    assert_eq!(work.notes, 1);
    // An existing -shm file is not deleted.
    assert!(side(dir.path(), "-shm").exists());
    drop(writer);
}

#[test]
fn rows_only_in_a_copied_wal_are_read_and_no_file_changes() {
    let source = tempfile::tempdir().unwrap();
    let writer = wal_only_db(source.path());
    let dir = tempfile::tempdir().unwrap();
    fs::copy(db(source.path()), db(dir.path())).unwrap();
    fs::copy(side(source.path(), "-wal"), side(dir.path(), "-wal")).unwrap();
    drop(writer);

    let db_before = fs::read(db(dir.path())).unwrap();
    let wal_before = fs::read(side(dir.path(), "-wal")).unwrap();
    assert!(!wal_before.is_empty());

    let (_, work) = not_own(inspect(dir.path()));
    assert_eq!(work.notes, 1);

    assert_eq!(fs::read(db(dir.path())).unwrap(), db_before);
    assert_eq!(fs::read(side(dir.path(), "-wal")).unwrap(), wal_before);
    assert_eq!(
        files(dir.path()),
        vec!["longhand.db".to_string(), "longhand.db-wal".to_string()]
    );
}

#[test]
fn an_own_wal_database_is_left_with_the_same_files() {
    let dir = tempfile::tempdir().unwrap();
    own_db(dir.path());
    let before = fs::read(db(dir.path())).unwrap();
    assert_eq!(inspect(dir.path()), Inspection::Own);
    assert_eq!(fs::read(db(dir.path())).unwrap(), before);
    assert_eq!(files(dir.path()), vec!["longhand.db".to_string()]);
}

#[test]
fn an_existing_shm_file_is_not_deleted() {
    let source = tempfile::tempdir().unwrap();
    let writer = wal_only_db(source.path());
    let dir = tempfile::tempdir().unwrap();
    for suffix in ["", "-wal", "-shm"] {
        fs::copy(side(source.path(), suffix), side(dir.path(), suffix)).unwrap();
    }
    drop(writer);
    let (_, work) = not_own(inspect(dir.path()));
    assert_eq!(work.notes, 1);
    assert!(side(dir.path(), "-shm").exists());
}
