//! Migration 0002 (KTD5): note rows become one notes document for each
//! session, and pending notes go (R26, R27).

use rusqlite::{Connection, params};
use rusqlite_migration::{M, Migrations};
use session_core::Store;

/// A database as the walking skeleton left it: only migration 0001 applied.
fn version_one(path: &std::path::Path) -> Connection {
    let mut conn = Connection::open(path).unwrap();
    Migrations::new(vec![M::up(include_str!("../migrations/0001_init.sql"))])
        .to_latest(&mut conn)
        .unwrap();
    conn
}

fn add_session(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO sessions (id, created_at, state) VALUES (?1, ?2, ?3)",
        params![id, "2026-09-22T10:00:00-05:00", r#"{"state":"saved"}"#],
    )
    .unwrap();
}

fn add_note(conn: &Connection, session_id: Option<&str>, text: &str, anchor: &str) {
    conn.execute(
        "INSERT INTO notes (session_id, text, anchor) VALUES (?1, ?2, ?3)",
        params![session_id, text, anchor],
    )
    .unwrap();
}

fn at(position_ms: u64, gap_anchored: bool) -> String {
    format!(r#"{{"kind":"at","position_ms":{position_ms},"gap_anchored":{gap_anchored}}}"#)
}

const UNANCHORED: &str = r#"{"kind":"unanchored"}"#;

#[test]
fn note_rows_become_one_document_in_their_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_one(&path);
        add_session(&conn, "s1");
        add_session(&conn, "s2");
        add_note(&conn, Some("s1"), "agenda: budget", UNANCHORED);
        add_note(
            &conn,
            Some("s1"),
            "decision:  ship\nFriday",
            &at(760_000, false),
        );
        add_note(&conn, Some("s2"), "other session", &at(1_000, false));
        add_note(
            &conn,
            Some("s1"),
            "back from the break",
            &at(3_723_000, true),
        );
        add_note(&conn, Some("s1"), "ask about hiring", &at(3_900_500, false));
        add_note(
            &conn,
            Some("s1"),
            "[bracketed] thought",
            &at(5_400_000, false),
        );
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.notes("s1").unwrap(),
        "- agenda: budget\n\
         - [00:12:40] decision: ship Friday\n\
         - [01:02:03 ~gap] back from the break\n\
         - [01:05:00] ask about hiring\n\
         - [01:30:00] [bracketed] thought\n"
    );
    assert_eq!(store.notes("s2").unwrap(), "- [00:00:01] other session\n");
}

#[test]
fn a_session_with_no_notes_migrates_to_an_empty_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    add_session(&version_one(&path), "s1");
    assert_eq!(Store::open(&path).unwrap().notes("s1").unwrap(), "");
}

#[test]
fn pending_notes_are_gone_after_the_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_one(&path);
        add_note(&conn, None, "typed with no session open", UNANCHORED);
    }
    let mut store = Store::open(&path).unwrap();
    // A new session does not receive the pending note (R27).
    store
        .create_session(
            "s1",
            chrono::DateTime::parse_from_rfc3339("2026-09-24T10:00:00-05:00").unwrap(),
            None,
        )
        .unwrap();
    assert_eq!(store.notes("s1").unwrap(), "");
    drop(store);
    let conn = Connection::open(&path).unwrap();
    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'notes'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0, "the note-row table is dropped");
}

#[test]
fn existing_sessions_have_no_title_and_no_found_speakers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    add_session(&version_one(&path), "s1");
    let store = Store::open(&path).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.title, None);
    assert_eq!(record.found_speakers, None);
}
