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

fn add_note(conn: &Connection, note_id: Option<&str>, text: &str, anchor: &str) {
    conn.execute(
        "INSERT INTO notes (session_id, text, anchor) VALUES (?1, ?2, ?3)",
        params![note_id, text, anchor],
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
            "SELECT channels FROM segments WHERE note_id = 's1'",
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
            // A Join row: migration 0025 deletes the Transcribe rows of a
            // saved note.
            conn.execute(
                "INSERT INTO checkpoints (session_id, stage, key, payload)
                 VALUES (?1, 'join', 'w1', '{}')",
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
                .checkpoints(id, session_core::Stage::Join)
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
            "UPDATE sessions SET mic_id = 'usb-uid', mic_label = 'USB Audio'
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
        }
    );
}

// ------------------------------------------------ migration 0022: note_id

/// The tables that had a `session_id` column at version 21.
const NOTE_ID_TABLES: [&str; 10] = [
    "segments",
    "jobs",
    "checkpoints",
    "rename_intents",
    "owed_renames",
    "recognition_output",
    "recognition_epochs",
    "result_state",
    "move_notes",
    "type_moves",
];

/// The hashes that builds with 21 migrations recorded. An applied migration
/// must not change: `inspect` compares these rows with the build hashes.
const VERSION_21_HASHES: [&str; 21] = [
    "bcf965daa09d177a7d11ab5102cf3b191b0182f9ed1ff3a7695ff4bee102b757",
    "5765eb8b34146e3f7048a003cabab84b087532ab94ee810420f2443f928566c0",
    "5b1b1b75f35a9d23bc470c2c27001e084579624b2fe807078a292dab717b90bd",
    "b41e7dccc79bf380dc6a59e2d9f500ae60791258272ffb5aa8a99bc2792d65ef",
    "653542d83cee9fe2276a5a634c72993d7a3bc3ae71aabe4e18b3a5c71e5a7b0f",
    "e7c16f194350697bf915d42619f54586d6f5639e4cbc7a914915b26a38dfe087",
    "5c12d00b83f7f3adf118844c16773c646c9447054b7c15d9f833a448a03e8e3b",
    "2fc87df29e508ef2ceb0472542fd3247f79a09aeaa41eb95f7942246afe9bf8f",
    "94a10f12e6280ec10fd6c7e58a3d3f170ede9fa817c42daa1000a749325a4275",
    "5b03f2ca9145368fd5e104e08540e6c7a55e5be1e369ecdbf623a0cc6d138fe1",
    "26509e63a8cfd020e3a0b790b84034d035421b2bfc97f52b158eb890b6803b41",
    "58ab795e83a0ba808ef4425425e545cd82f2500a9656470601e9a7295f7089b0",
    "0b9165d809580bcd9251b5b42974ca7eeb7215f4e32375556c36cfbebd826d82",
    "64cf24b97fa28f96e55a6c6c72ae1c8f5fa31411a541f1c5cf14c7ab0f7fb589",
    "6f5a3c4533602a45171e3309d5279d54fc3f2af584c1ae8bd2778ab049c5ff15",
    "8b1703d315ae28105ddd0f5a1d30a08756f2b094081619ed2ef3a92dcc05333f",
    "9f8f41e6ff367db227adb42d130d4203524b42797e89e8ddc3c14209217cdd64",
    "324bda22e9d6d731116c7561242b4c701614e2462e08dd943a8895068623a7eb",
    "08fe9888edc0af2cfadc3e5e758cd20f62e40546d54b9fe3cb4bd3b777d68dd9",
    "72e7924113bd19e371e4bada50463a38f890e5d9d8800f052ca89bc5580cca56",
    "28ec11769db91f0e1d8804f576356c2725d19433c674a2edbbde6a5678596bae",
];

/// A database as the app audio work left it: migrations 0001 to 0021
/// applied.
fn version_twenty_one(path: &std::path::Path) -> Connection {
    let mut conn = Connection::open(path).unwrap();
    session_core::schema::migrations()
        .to_version(&mut conn, 21)
        .unwrap();
    conn
}

/// A recognition block that `validate` accepts.
fn recognition_block() -> session_core::recognition::OutputBlock {
    use session_core::recognition::{Coverage, OutputBlock, SampleRange, SpeakerScope};
    let range = SampleRange {
        start: 0,
        end: 16000,
    };
    OutputBlock {
        key: "window-0".into(),
        scope: SpeakerScope {
            generation: "g".into(),
            epoch: 1,
        },
        revision: 1,
        coverage: Coverage {
            received: Some(range),
            computed: Some(range),
            provisional: None,
            committed: Some(range),
        },
        rows: vec![],
        speakers: vec![],
    }
}

/// Add the note `id` and one row for it in each table of [`NOTE_ID_TABLES`],
/// with the version-21 column name. The note is not saved, so migration
/// 0025 keeps its rows.
fn add_a_row_in_each_table(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO sessions (id, created_at, state) VALUES (?1, ?2, ?3)",
        params![id, "2026-09-22T10:00:00-05:00", r#"{"state":"capturing"}"#],
    )
    .unwrap();
    let statements = [
        "INSERT INTO segments
         (session_id, idx, offset_ms, duration_ms, wall_start, wall_end, audio_file)
         VALUES (?1, 0, 0, 60000, '2026-09-22T10:00:00-05:00',
                 '2026-09-22T10:01:00-05:00', 'audio/segment-01.opus')",
        "INSERT INTO jobs (session_id, stage) VALUES (?1, 'transcribe')",
        "INSERT INTO checkpoints (session_id, stage, key, payload)
         VALUES (?1, 'transcribe', 'w1', '{}')",
        "INSERT INTO rename_intents (session_id, from_path, to_path) VALUES (?1, '/a', '/b')",
        "INSERT INTO owed_renames (session_id) VALUES (?1)",
        "INSERT INTO recognition_epochs (session_id, generation, next_epoch) VALUES (?1, 'g', 1)",
        "INSERT INTO result_state (session_id, key, value) VALUES (?1, 'k', '1')",
        "INSERT INTO move_notes (session_id, step) VALUES (?1, '{\"step\":\"pending\"}')",
        "INSERT INTO type_moves (session_id, to_type, changed_at)
         VALUES (?1, 'Interview', '2026-09-22T10:00:00-05:00')",
    ];
    for sql in statements {
        conn.execute(sql, [id]).unwrap();
    }
    conn.execute(
        "INSERT INTO recognition_output (session_id, generation, key, revision, payload)
         VALUES (?1, 'g', 'window-0', 1, ?2)",
        params![id, serde_json::to_string(&recognition_block()).unwrap()],
    )
    .unwrap();
}

/// The number of rows for `id` in all tables of [`NOTE_ID_TABLES`], after
/// migration 0022.
fn rows_of(conn: &Connection, id: &str) -> i64 {
    NOTE_ID_TABLES
        .iter()
        .map(|table| {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE note_id = ?1"),
                [id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
        })
        .sum()
}

#[test]
fn migration_twenty_two_keeps_the_rows_of_each_table_with_their_note_id() {
    use session_core::Stage;
    use session_core::jobs::{MoveStep, RenameIntent};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    add_a_row_in_each_table(&version_twenty_one(&path), "s1");

    let mut store = Store::open(&path).unwrap();
    let segments = store.segments("s1").unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].audio_file, "audio/segment-01.opus");
    assert_eq!(store.current_stage("s1").unwrap(), Some(Stage::Transcribe));
    assert_eq!(
        store
            .checkpoint_value("s1", Stage::Transcribe, "w1")
            .unwrap(),
        Some("{}".to_string())
    );
    assert_eq!(
        store.rename_intent("s1").unwrap(),
        Some(RenameIntent {
            from: "/a".into(),
            to: "/b".into(),
            temp: None,
        })
    );
    assert_eq!(store.owed_renames().unwrap(), ["s1"]);
    assert_eq!(
        store.recognition_block("s1", "g", "window-0").unwrap(),
        Some(recognition_block())
    );
    assert_eq!(
        store.result_state("s1", "k").unwrap(),
        Some(serde_json::json!(1))
    );
    assert_eq!(store.move_step("s1").unwrap(), Some(MoveStep::Pending));
    assert_eq!(
        store.type_move("s1").unwrap().map(|m| m.to),
        Some(Some("Interview".to_string()))
    );
    // The stored next epoch is 1, so a restart gives epoch 2.
    let replay = store.restart_recognition_epoch("s1", "g", 0, 0).unwrap();
    assert_eq!(replay.scope.epoch, 2);
    drop(store);
    assert_eq!(
        user_version(&path),
        session_core::schema::MIGRATION_SQL.len() as i64
    );
}

#[test]
fn after_migration_twenty_two_no_table_has_the_old_id_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT m.name, p.name FROM sqlite_master AS m, pragma_table_info(m.name) AS p
             WHERE m.type = 'table' AND p.name IN ('session_id', 'note_id')
             ORDER BY m.name",
        )
        .unwrap();
    let columns: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut expected: Vec<(String, String)> = NOTE_ID_TABLES
        .iter()
        .map(|table| (table.to_string(), "note_id".to_string()))
        .collect();
    expected.sort();
    assert_eq!(columns, expected);
}

#[test]
fn after_migration_twenty_two_a_deleted_note_still_deletes_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_twenty_one(&path);
        add_a_row_in_each_table(&conn, "s1");
        add_a_row_in_each_table(&conn, "s2");
    }
    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    assert_eq!(rows_of(&conn, "s1"), NOTE_ID_TABLES.len() as i64);
    conn.execute("DELETE FROM sessions WHERE id = 's1'", [])
        .unwrap();
    assert_eq!(rows_of(&conn, "s1"), 0);
    assert_eq!(rows_of(&conn, "s2"), NOTE_ID_TABLES.len() as i64);
}

/// A database that a build with 21 migrations made is Longhand's own before
/// and after migration 0022, because migrations 1 to 21 keep their hashes.
#[test]
fn migrations_one_to_twenty_one_keep_their_hashes() {
    use session_core::inspect::{Inspection, inspect};
    use session_core::schema;
    assert_eq!(schema::migration_hashes()[..21], VERSION_21_HASHES);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_twenty_one(&path);
        let recorded: Vec<String> = VERSION_21_HASHES.iter().map(|h| h.to_string()).collect();
        schema::record_applied_migrations(&conn, &recorded).unwrap();
        add_session(&conn, "s1");
    }
    assert_eq!(inspect(dir.path()), Inspection::Own);
    drop(Store::open(&path).unwrap());
    assert_eq!(applied_rows(&path), build_rows());
    assert_eq!(
        user_version(&path),
        session_core::schema::MIGRATION_SQL.len() as i64
    );
    assert_eq!(inspect(dir.path()), Inspection::Own);
}

// ------------------------------------------------ migration 0023: imports

/// The migration count of the last release before the audio import, which
/// has no `Importing` state.
const LAST_RELEASE_VERSION: i64 = 22;

#[test]
fn a_database_with_an_importing_row_is_above_the_last_release_version() {
    // KTD5: an older build cannot parse the state, so it must refuse the
    // database and rebuild it.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    let mut store = Store::open(&path).unwrap();
    store
        .create_import(&session_core::jobs::NewImport {
            id: "i",
            opened_at: chrono::DateTime::parse_from_rfc3339("2026-10-20T14:30:00Z").unwrap(),
            title: &session_core::Title {
                text: "Team sync".into(),
                source: session_core::TitleSource::Proposed,
            },
            note_type: None,
            tags: &[],
            notes: "",
            model: &session_core::RecordingModel {
                asr_model: "m".into(),
                live_transcript: false,
            },
            snapshot: None,
            speaker_count: None,
            source: &session_core::note_record::ImportSource {
                source_name: "Team sync.m4a".into(),
                sha256: None,
            },
        })
        .unwrap();
    drop(store);
    assert!(user_version(&path) > LAST_RELEASE_VERSION);
}

// ------------------------------------------------ migration 0024: transcript removal

fn version_twenty_three(path: &std::path::Path) -> Connection {
    let mut conn = Connection::open(path).unwrap();
    session_core::schema::migrations()
        .to_version(&mut conn, 23)
        .unwrap();
    conn
}

#[test]
fn migration_twenty_four_gives_existing_notes_no_removed_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_twenty_three(&path);
        add_session(&conn, "s1");
        add_session(&conn, "s2");
    }
    let store = Store::open(&path).unwrap();
    assert!(store.session("s1").unwrap().is_some());
    assert!(!store.transcript_removed("s1").unwrap());
    assert!(!store.transcript_removed("s2").unwrap());
}

/// Text that only the deleted rows of the test below held.
const DELETED_TEXT: &str = "words of a transcript that a delete freed";

/// Whether the file at `path` holds `text`. A missing file holds nothing.
fn file_holds(path: &std::path::Path, text: &str) -> bool {
    std::fs::read(path)
        .map(|bytes| bytes.windows(text.len()).any(|w| w == text.as_bytes()))
        .unwrap_or(false)
}

fn free_pages(path: &std::path::Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "freelist_count", |r| r.get(0))
        .unwrap()
}

#[test]
fn the_first_open_after_migration_twenty_four_leaves_no_free_pages() {
    // KTD5: an earlier build deleted rows with no secure_delete, so the
    // free pages still hold their text.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_twenty_three(&path);
        add_session(&conn, "s1");
        let payload = DELETED_TEXT.repeat(40);
        for key in 0..200 {
            conn.execute(
                "INSERT INTO checkpoints (note_id, stage, key, payload)
                 VALUES ('s1', 'transcribe', ?1, ?2)",
                params![format!("w{key}"), payload],
            )
            .unwrap();
        }
        conn.execute("DELETE FROM checkpoints", []).unwrap();
    }
    assert!(free_pages(&path) > 0);
    assert!(file_holds(&path, DELETED_TEXT));

    let store = Store::open(&path).unwrap();
    assert!(store.session("s1").unwrap().is_some());
    assert_eq!(free_pages(&path), 0);
    assert!(!file_holds(&path, DELETED_TEXT));
    assert!(!file_holds(
        &dir.path().join("longhand.db-wal"),
        DELETED_TEXT
    ));
}

// ------------------------------------------------ migration 0025: saved note content

fn version_twenty_four(path: &std::path::Path) -> Connection {
    let mut conn = Connection::open(path).unwrap();
    session_core::schema::migrations()
        .to_version(&mut conn, 24)
        .unwrap();
    conn
}

/// Text that only the recognition rows and the summary cache hold.
const TRANSCRIPT_TEXT: &str = "the budget for the spring term is final";

/// The stages and keys of the checkpoints that [`add_content`] writes.
const CONTENT_CHECKPOINTS: [(&str, &str); 10] = [
    ("assemble", "done"),
    ("transcribe", "w1"),
    ("diarize", "d1"),
    ("previous_transcribe", "w1"),
    ("previous_assemble", "done"),
    ("previous_summarize", "chunk-0"),
    ("previous_save", "done"),
    ("summarize", "chunk-0"),
    ("summarize", "failed"),
    ("join", "done"),
];

/// Add the note `id` in `state` with a row in each table of note content,
/// as an earlier build left it: each payload holds [`TRANSCRIPT_TEXT`].
fn add_content(conn: &Connection, id: &str, state: &str) {
    conn.execute(
        "INSERT INTO sessions (id, created_at, state) VALUES (?1, ?2, ?3)",
        params![id, "2026-09-22T10:00:00-05:00", state],
    )
    .unwrap();
    let payload = serde_json::to_string(&TRANSCRIPT_TEXT.repeat(30)).unwrap();
    for (stage, key) in CONTENT_CHECKPOINTS {
        // The rows that the clear keeps hold no transcript.
        let kept = stage == "join" || key == "failed";
        conn.execute(
            "INSERT INTO checkpoints (note_id, stage, key, payload) VALUES (?1, ?2, ?3, ?4)",
            params![id, stage, key, if kept { "{}" } else { payload.as_str() }],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO recognition_output (note_id, generation, key, revision, payload)
         VALUES (?1, 'g', 'window-0', 1, ?2)",
        params![id, payload],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO recognition_epochs (note_id, generation, next_epoch) VALUES (?1, 'g', 1)",
        [id],
    )
    .unwrap();
}

/// The stage and key of each checkpoint of `id`, in order.
fn checkpoint_rows(conn: &Connection, id: &str) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare("SELECT stage, key FROM checkpoints WHERE note_id = ?1 ORDER BY stage, key")
        .unwrap();
    stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn recognition_rows(conn: &Connection, id: &str) -> i64 {
    ["recognition_output", "recognition_epochs"]
        .iter()
        .map(|table| {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE note_id = ?1"),
                [id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
        })
        .sum()
}

fn pairs(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = rows
        .iter()
        .map(|(s, k)| (s.to_string(), k.to_string()))
        .collect();
    out.sort();
    out
}

const SAVED: &str = r#"{"state":"saved"}"#;
const CAPTURING: &str = r#"{"state":"capturing"}"#;

#[test]
fn migration_twenty_five_clears_saved_notes_and_keeps_unfinished_ones() {
    // AE13: a database from the previous schema.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_twenty_four(&path);
        add_content(&conn, "saved", SAVED);
        add_content(&conn, "open", CAPTURING);
    }

    drop(Store::open(&path).unwrap());

    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        checkpoint_rows(&conn, "saved"),
        pairs(&[("join", "done"), ("summarize", "failed")])
    );
    assert_eq!(recognition_rows(&conn, "saved"), 0);
    assert_eq!(checkpoint_rows(&conn, "open"), pairs(&CONTENT_CHECKPOINTS));
    assert_eq!(recognition_rows(&conn, "open"), 2);
}

#[test]
fn migration_twenty_five_keeps_the_summary_cache_of_a_waiting_title_request() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_twenty_four(&path);
        add_content(&conn, "titled", SAVED);
        conn.execute(
            "INSERT INTO checkpoints (note_id, stage, key, payload)
             VALUES ('titled', 'summarize', 'title-request', '{}')",
            [],
        )
        .unwrap();
    }

    drop(Store::open(&path).unwrap());

    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        checkpoint_rows(&conn, "titled"),
        pairs(&[
            ("join", "done"),
            ("summarize", "chunk-0"),
            ("summarize", "failed"),
            ("summarize", "title-request"),
        ])
    );
    assert_eq!(recognition_rows(&conn, "titled"), 0);
}

#[test]
fn after_migration_twenty_five_no_file_holds_the_transcript_of_a_saved_note() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("longhand.db");
    {
        let conn = version_twenty_four(&path);
        add_content(&conn, "saved", SAVED);
        // A build with migration 0024 already ran its VACUUM.
        conn.execute("UPDATE computer SET vacuum_owed = 0", [])
            .unwrap();
        // Free pages from an earlier delete with no secure_delete.
        conn.execute(
            "INSERT INTO checkpoints (note_id, stage, key, payload)
             VALUES ('saved', 'verify', 'v1', ?1)",
            [TRANSCRIPT_TEXT.repeat(200)],
        )
        .unwrap();
        conn.execute("DELETE FROM checkpoints WHERE stage = 'verify'", [])
            .unwrap();
    }
    assert!(file_holds(&path, TRANSCRIPT_TEXT));

    let store = Store::open(&path).unwrap();
    assert!(store.session("saved").unwrap().is_some());
    assert_eq!(free_pages(&path), 0);
    assert!(!file_holds(&path, TRANSCRIPT_TEXT));
    assert!(!file_holds(
        &dir.path().join("longhand.db-wal"),
        TRANSCRIPT_TEXT
    ));
}
