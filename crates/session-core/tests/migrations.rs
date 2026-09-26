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

/// A database as the rename work left it: migrations 0001 to 0003 applied.
fn version_three(path: &std::path::Path) -> Connection {
    let mut conn = Connection::open(path).unwrap();
    session_core::schema::migrations()
        .to_version(&mut conn, 3)
        .unwrap();
    conn
}

#[test]
fn migration_four_keeps_existing_sessions_and_their_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_three(&path);
        add_session(&conn, "s1");
        conn.execute(
            "UPDATE sessions SET asr_model = 'whisper-large-v3-turbo-q5km', title = 'Budget',
               title_source = 'user', notes = '- agenda' WHERE id = 's1'",
            [],
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.state, session_core::SessionState::Saved);
    assert_eq!(
        record.asr_model.as_deref(),
        Some("whisper-large-v3-turbo-q5km")
    );
    assert_eq!(record.title.unwrap().text, "Budget");
    assert_eq!(store.notes("s1").unwrap(), "- agenda");
    // A session from before the migration has no recorded live mode.
    assert_eq!(record.live_transcript, None);
    drop(store);
    let conn = Connection::open(&path).unwrap();
    let marks: (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT transcribe_again_model, transcribe_again_error FROM sessions WHERE id = 's1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(marks, (None, None));
}

/// A database as the recognition models work left it: migrations 0001 to
/// 0005 applied.
fn version_five(path: &std::path::Path) -> Connection {
    let mut conn = Connection::open(path).unwrap();
    session_core::schema::migrations()
        .to_version(&mut conn, 5)
        .unwrap();
    conn
}

#[test]
fn migration_six_gives_existing_segments_one_channel_and_no_capture_choice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_five(&path);
        add_session(&conn, "s1");
        conn.execute(
            "INSERT INTO segments
             (session_id, idx, offset_ms, duration_ms, wall_start, wall_end, audio_file)
             VALUES ('s1', 0, 0, 60000, '2026-09-22T10:00:00-05:00',
                     '2026-09-22T10:01:00-05:00', 'audio/segment-01.opus')",
            [],
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.segments("s1").unwrap().len(), 1);
    // A note from an older build resumes on "System default" with Computer
    // audio off (KTD2).
    assert_eq!(
        store.capture_choice("s1").unwrap(),
        session_core::CaptureChoice::default()
    );
    drop(store);
    let conn = Connection::open(&path).unwrap();
    let channels: i64 = conn
        .query_row(
            "SELECT channels FROM segments WHERE session_id = 's1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(channels, 1);
    let stereo_from: Option<i64> = conn
        .query_row(
            "SELECT stereo_from FROM sessions WHERE id = 's1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stereo_from, None);
}
