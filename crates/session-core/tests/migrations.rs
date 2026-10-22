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
    // A note from an older build resumes on "System default" with system
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

/// A database as the system audio work left it: migrations 0001 to 0006
/// applied.
fn version_six(path: &std::path::Path) -> Connection {
    let mut conn = Connection::open(path).unwrap();
    session_core::schema::migrations()
        .to_version(&mut conn, 6)
        .unwrap();
    conn
}

#[test]
fn migration_seven_keeps_saved_notes_and_gives_them_no_transcript_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_six(&path);
        for id in ["s1", "s2"] {
            add_session(&conn, id);
            conn.execute(
                "INSERT INTO segments
                 (session_id, idx, offset_ms, duration_ms, wall_start, wall_end, audio_file,
                  channels)
                 VALUES (?1, 0, 0, 60000, '2026-09-22T10:00:00-05:00',
                         '2026-09-22T10:01:00-05:00', 'audio/segment-01.opus', 2)",
                [id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO checkpoints (session_id, stage, key, payload)
                 VALUES (?1, 'transcribe', 'w1', '{}')",
                [id],
            )
            .unwrap();
        }
        conn.execute(
            "UPDATE sessions SET asr_model = 'parakeet', title = 'Budget', title_source = 'user',
               notes = '- agenda', found_speakers = 2, home_path = '/notes/Budget',
               stereo_from = 0 WHERE id = 's1'",
            [],
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.state, session_core::SessionState::Saved);
    assert_eq!(record.asr_model.as_deref(), Some("parakeet"));
    assert_eq!(record.title.unwrap().text, "Budget");
    assert_eq!(record.found_speakers, Some(2));
    assert_eq!(record.home_path.as_deref(), Some("/notes/Budget"));
    assert_eq!(store.notes("s1").unwrap(), "- agenda");
    assert_eq!(store.stereo_from("s1").unwrap(), Some(0));
    assert_eq!(store.segments("s1").unwrap()[0].channels, 2);
    assert_eq!(store.sessions().unwrap().len(), 2);
    for id in ["s1", "s2"] {
        assert_eq!(
            store
                .checkpoints(id, session_core::Stage::Transcribe)
                .unwrap()
                .len(),
            1
        );
        // A note from before the migration has no recorded fingerprint.
        assert_eq!(store.transcript_fingerprint(id).unwrap(), None);
    }
}

/// A database as the summary prompt work left it: migrations 0001 to 0013
/// applied.
fn version_thirteen(path: &std::path::Path) -> Connection {
    let mut conn = Connection::open(path).unwrap();
    session_core::schema::migrations()
        .to_version(&mut conn, 13)
        .unwrap();
    conn
}

#[test]
fn migration_fourteen_keeps_existing_segments_with_their_audio() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_thirteen(&path);
        add_session(&conn, "s1");
        for (idx, audio) in [(0, "audio/segment-01.opus"), (1, "audio/segment-02.opus")] {
            conn.execute(
                "INSERT INTO segments
                 (session_id, idx, offset_ms, duration_ms, wall_start, wall_end, audio_file,
                  channels)
                 VALUES ('s1', ?1, ?2, 60000, '2026-09-22T10:00:00-05:00',
                         '2026-09-22T10:01:00-05:00', ?3, 2)",
                params![idx, idx * 120_000, audio],
            )
            .unwrap();
        }
    }
    let store = Store::open(&path).unwrap();
    let segments = store.segments("s1").unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].audio_file, "audio/segment-02.opus");
    assert_eq!(segments[1].channels, 2);
    // No segment from before the migration has its audio removed.
    assert!(store.removed_audio("s1").unwrap().is_empty());
    drop(store);
    let conn = Connection::open(&path).unwrap();
    let flags: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM segments WHERE audio_removed = 0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(flags, 2);
}

// ------------------------------------------------ applied migration hashes

/// The rows of `applied_migrations`, in version order.
fn applied_rows(path: &std::path::Path) -> Vec<(i64, String)> {
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn
        .prepare("SELECT version, sha256 FROM applied_migrations ORDER BY version")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The rows that a database with all of this build's migrations must have.
fn build_rows() -> Vec<(i64, String)> {
    session_core::schema::migration_hashes()
        .into_iter()
        .enumerate()
        .map(|(i, hash)| (i as i64 + 1, hash))
        .collect()
}

fn user_version(path: &std::path::Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

#[test]
fn a_new_database_records_each_migration_with_the_build_hash() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    drop(Store::open(&path).unwrap());
    let rows = applied_rows(&path);
    assert_eq!(rows.len(), session_core::schema::MIGRATION_SQL.len());
    assert_eq!(rows, build_rows());
}

#[test]
fn a_database_with_no_table_gets_its_rows_and_keeps_its_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        session_core::schema::migrations()
            .to_version(&mut conn, 14)
            .unwrap();
        add_session(&conn, "s1");
    }
    let store = Store::open(&path).unwrap();
    assert!(store.session("s1").unwrap().is_some());
    drop(store);
    assert_eq!(applied_rows(&path), build_rows());
    assert_eq!(
        user_version(&path),
        session_core::schema::MIGRATION_SQL.len() as i64
    );
}

#[test]
fn two_opens_give_the_same_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    drop(Store::open(&path).unwrap());
    let first = applied_rows(&path);
    drop(Store::open(&path).unwrap());
    assert_eq!(applied_rows(&path), first);
}

#[test]
fn a_comment_or_whitespace_change_gives_the_same_hash() {
    use session_core::schema::migration_hash;
    let sql = "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT);\n\
               ALTER TABLE t ADD COLUMN x TEXT;";
    let commented = "-- A new table.\n\
                     CREATE TABLE t (\n    id INTEGER PRIMARY KEY, -- the key\n    name TEXT\n);\n\n\
                     /* A block\n   comment. */\n\
                     ALTER TABLE t ADD COLUMN x TEXT;\n";
    let spaced =
        "  CREATE\tTABLE t(id INTEGER PRIMARY KEY,name TEXT);ALTER TABLE t ADD COLUMN x TEXT;";
    assert_eq!(migration_hash(sql), migration_hash(commented));
    assert_eq!(migration_hash(sql), migration_hash(spaced));
}

#[test]
fn a_changed_statement_gives_a_different_hash() {
    use session_core::schema::migration_hash;
    assert_ne!(
        migration_hash("ALTER TABLE t ADD COLUMN x TEXT;"),
        migration_hash("ALTER TABLE t ADD COLUMN y TEXT;")
    );
}

/// Covers AE4. A later build adds a migration: the database migrates to the
/// new version, keeps its rows, and records a row for the new version.
#[test]
fn an_added_migration_records_its_hash() {
    use session_core::schema;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    drop(Store::open(&path).unwrap());
    add_session(&Connection::open(&path).unwrap(), "s1");

    let extra = "ALTER TABLE sessions ADD COLUMN later_column TEXT;";
    let mut steps = schema::migration_steps();
    steps.push(M::up(extra));
    let mut hashes = schema::migration_hashes();
    hashes.push(schema::migration_hash(extra));
    let mut conn = Connection::open(&path).unwrap();
    Migrations::new(steps).to_latest(&mut conn).unwrap();
    schema::record_applied_migrations(&conn, &hashes).unwrap();
    drop(conn);

    let n = schema::MIGRATION_SQL.len();
    let rows = applied_rows(&path);
    assert_eq!(rows.len(), n + 1);
    assert_eq!(rows[..n].to_vec(), build_rows());
    assert_eq!(rows[n], (n as i64 + 1, schema::migration_hash(extra)));
    assert_eq!(user_version(&path), n as i64 + 1);
    let count: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM sessions WHERE id = 's1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

/// A database as the audio removal work left it: migrations 0001 to 0014
/// applied.
fn version_fourteen(path: &std::path::Path) -> Connection {
    let mut conn = Connection::open(path).unwrap();
    session_core::schema::migrations()
        .to_version(&mut conn, 14)
        .unwrap();
    conn
}

#[test]
fn migration_fifteen_gives_existing_prompt_columns_an_empty_title_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_fourteen(&path);
        add_session(&conn, "s1");
        conn.execute(
            "UPDATE sessions SET prompt_snapshot_id = 'b1', prompt_snapshot_name = 'Standup',
               prompt_snapshot_text = 'Write a standup.' WHERE id = 's1'",
            [],
        )
        .unwrap();
        add_session(&conn, "s2");
    }
    let _store = Store::open(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    let title_text: String = conn
        .query_row(
            "SELECT prompt_snapshot_title_text FROM sessions WHERE id = 's1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(title_text, "");
    let name: String = conn
        .query_row(
            "SELECT prompt_snapshot_name FROM sessions WHERE id = 's1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(name, "Standup");
    // A note with no snapshot still has none.
    let id: Option<String> = conn
        .query_row(
            "SELECT prompt_snapshot_id FROM sessions WHERE id = 's2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(id, None);
}

#[test]
fn migration_sixteen_keeps_existing_rows_and_gives_them_no_open_time_owner_or_fingerprints() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_fourteen(&path);
        add_session(&conn, "s1");
        conn.execute(
            "UPDATE sessions SET home_path = '/notes/a', notes = '- agenda\n' WHERE id = 's1'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO segments
             (session_id, idx, offset_ms, duration_ms, wall_start, wall_end, audio_file, channels)
             VALUES ('s1', 0, 0, 60000, '2026-09-22T10:05:00-05:00',
                     '2026-09-22T10:06:00-05:00', 'audio/segment-01.opus', 1)",
            [],
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.state, session_core::SessionState::Saved);
    assert_eq!(record.home_path.as_deref(), Some("/notes/a"));
    assert_eq!(store.notes("s1").unwrap(), "- agenda\n");
    let segments = store.segments("s1").unwrap();
    assert_eq!(segments.len(), 1);
    // A row from before the migration has no open time, so its start is
    // its first segment, as before.
    assert_eq!(record.started_at, None);
    assert_eq!(record.start(&segments), segments[0].wall_start);
    // This computer cannot tell whether it started an older note, so the
    // flag is false: removal (R18) then keeps the note.
    assert!(!store.started_here("s1").unwrap());
    assert_eq!(
        store.written_fingerprints("s1").unwrap(),
        session_core::jobs::WrittenFingerprints::default()
    );
}

#[test]
fn migration_twenty_gives_existing_segments_no_pause_flag() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        session_core::schema::migrations()
            .to_version(&mut conn, 19)
            .unwrap();
        add_session(&conn, "s1");
        for idx in 0..2 {
            conn.execute(
                "INSERT INTO segments
                 (session_id, idx, offset_ms, duration_ms, wall_start, wall_end, audio_file,
                  channels)
                 VALUES ('s1', ?1, ?2, 60000, '2026-09-22T10:00:00-05:00',
                         '2026-09-22T10:01:00-05:00', ?3, 1)",
                params![
                    idx,
                    idx * 120_000,
                    format!("audio/segment-0{}.opus", idx + 1)
                ],
            )
            .unwrap();
        }
    }
    let store = Store::open(&path).unwrap();
    let flags: Vec<bool> = store
        .segments("s1")
        .unwrap()
        .iter()
        .map(|s| s.ends_in_pause)
        .collect();
    assert_eq!(flags, [false, false]);
}

#[test]
fn a_new_database_has_the_pause_flag_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    let (kind, default): (String, Option<String>) = conn
        .query_row(
            "SELECT type, dflt_value FROM pragma_table_info('segments')
             WHERE name = 'ends_in_pause'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "INTEGER");
    assert_eq!(default.as_deref(), Some("0"));
}

#[test]
fn migration_eighteen_keeps_existing_rows_and_gives_them_no_type_tags_or_pending_type() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        session_core::schema::migrations()
            .to_version(&mut conn, 17)
            .unwrap();
        add_session(&conn, "s1");
        conn.execute(
            "UPDATE sessions SET home_path = '/notes/a', notes = '- agenda\n',
               title = 'Budget', title_source = 'user' WHERE id = 's1'",
            [],
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    let record = store.session("s1").unwrap().unwrap();
    assert_eq!(record.state, session_core::SessionState::Saved);
    assert_eq!(record.home_path.as_deref(), Some("/notes/a"));
    assert_eq!(record.title.map(|t| t.text).as_deref(), Some("Budget"));
    assert_eq!(store.notes("s1").unwrap(), "- agenda\n");
    assert_eq!(record.note_type, None);
    assert!(record.tags.is_empty());
    assert_eq!(record.pending_type, None);
}

#[test]
fn migration_twenty_one_gives_existing_notes_an_empty_app_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        session_core::schema::migrations()
            .to_version(&mut conn, 20)
            .unwrap();
        add_session(&conn, "s1");
        conn.execute(
            "UPDATE sessions SET mic_id = 'usb-uid', mic_label = 'USB Audio', system_audio = 1
             WHERE id = 's1'",
            [],
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert!(store.note_apps("s1").unwrap().is_empty());
    // The migration keeps the capture choice of the note.
    assert_eq!(
        store.capture_choice("s1").unwrap(),
        session_core::CaptureChoice {
            microphone: Some(session_core::Microphone {
                id: "usb-uid".into(),
                label: "USB Audio".into(),
            }),
            system_audio: true,
        }
    );
}
