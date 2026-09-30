//! Read-only inspection of the notes database at launch (KTD1, KTD3).
//!
//! [`inspect`] tells whether `longhand.db` in the app data folder is this
//! build's own. It never calls `Store::open`, which writes pragmas and
//! migrates. It opens the database with `SQLITE_OPEN_READ_ONLY` (not
//! `immutable`, which ignores the WAL), and it closes the connection before
//! it returns. SQLite can make `longhand.db-shm` or `longhand.db-wal` for a
//! read-only open. The inspection deletes each of these files that it made.

use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::schema;

/// The name of the database file in the app data folder.
pub const DATABASE_FILE: &str = "longhand.db";

/// The name of the working folder in the app data folder.
pub const WORKING_FOLDER: &str = "sessions";

/// The file in a note's working folder that records an unfinished
/// publication. The same name as `session_save::publication::RECORD`.
const PUBLICATION_RECORD: &str = "publication.json";

/// The name prefix of a summary folder that a publication made in the
/// working folder.
const SUMMARY_FOLDER_PREFIX: &str = ".longhand-summary-";

/// The `result_state` key of an unfinished summary publication.
const SUMMARY_PUBLICATION_KEY: &str = "summary-publication";

/// How long the inspection waits for a lock that another connection holds.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// The result of [`inspect`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inspection {
    /// The database is this build's own, or there is no database. The build
    /// can open it with `Store::open`.
    Own,
    /// The database is not this build's own (R1). `work` is the unfinished
    /// work that it holds (R3).
    NotOwn {
        reason: NotOwnReason,
        work: UnfinishedWork,
    },
    /// The inspection could not read the database (R7). `error` is the error
    /// text.
    Unreadable { error: String },
}

/// Why the database is not this build's own (R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotOwnReason {
    /// The database version is above this build's migration count.
    NewerVersion { found: i64, own: i64 },
    /// The recorded hash of the migration at `version` is not this build's
    /// hash. `version` is the lowest version with a different hash.
    DifferentMigration { version: i64 },
}

/// The unfinished work that a database and its working folder hold (R3,
/// KTD3). The alert names each kind that is not zero (R6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnfinishedWork {
    /// Notes in a state other than Saved or Discarded. A state that this
    /// build does not know counts.
    pub notes: u64,
    /// Rows in `jobs`, and notes with a Transcribe Again run
    /// (`transcribe_again_model` is not null). Regenerate Summary is a
    /// `jobs` row.
    pub jobs: u64,
    /// A Move is open: `move_journal` or `move_notes` has a row.
    pub open_move: bool,
    /// Folder renames in progress: rows in `rename_intents`.
    pub renames: u64,
    /// Unfinished publications: `summary-publication` rows in
    /// `result_state`, `sessions/*/publication.json` files and
    /// `sessions/.longhand-summary-*` folders.
    pub publications: u64,
}

impl UnfinishedWork {
    /// True when the database and its working folder hold no unfinished
    /// work.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Inspect `longhand.db` in `app_data` without a change to any file
/// (KTD1). A missing or zero-byte database is [`Inspection::Own`], and the
/// inspection opens and makes no file. The unfinished work is counted only
/// for [`Inspection::NotOwn`].
pub fn inspect(app_data: &Path) -> Inspection {
    let path = app_data.join(DATABASE_FILE);
    match std::fs::metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Inspection::Own,
        Err(e) => {
            return Inspection::Unreadable {
                error: e.to_string(),
            };
        }
        Ok(meta) if meta.len() == 0 => return Inspection::Own,
        Ok(_) => {}
    }

    let side_files = [side_file(&path, "-shm"), side_file(&path, "-wal")];
    let existed: Vec<bool> = side_files.iter().map(|p| p.exists()).collect();

    let result = inspect_database(&path);

    // The connection is closed. Delete each side file that the open made.
    // A file that existed before is not changed.
    for (file, existed) in side_files.iter().zip(existed) {
        if !existed
            && file.exists()
            && let Err(e) = std::fs::remove_file(file)
        {
            log::warn!("inspection could not remove {}: {e}", file.display());
        }
    }

    match result {
        Ok(Ownership::Own) => Inspection::Own,
        Ok(Ownership::NotOwn(reason, database_work)) => {
            match count_folder_work(&app_data.join(WORKING_FOLDER)) {
                Ok(folder_publications) => Inspection::NotOwn {
                    reason,
                    work: UnfinishedWork {
                        publications: database_work.publications + folder_publications,
                        ..database_work
                    },
                },
                Err(e) => Inspection::Unreadable {
                    error: e.to_string(),
                },
            }
        }
        Err(e) => Inspection::Unreadable {
            error: e.to_string(),
        },
    }
}

enum Ownership {
    Own,
    NotOwn(NotOwnReason, UnfinishedWork),
}

fn side_file(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Open the database read-only, find its ownership and, for NotOwn, count
/// its unfinished work. The connection closes before this returns.
fn inspect_database(path: &Path) -> rusqlite::Result<Ownership> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let ownership = match not_own_reason(&conn)? {
        None => Ownership::Own,
        Some(reason) => Ownership::NotOwn(reason, count_database_work(&conn)?),
    };
    conn.close().map_err(|(_, e)| e)?;
    Ok(ownership)
}

fn not_own_reason(conn: &Connection) -> rusqlite::Result<Option<NotOwnReason>> {
    let found: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let own = schema::MIGRATION_SQL.len() as i64;
    if found > own {
        return Ok(Some(NotOwnReason::NewerVersion { found, own }));
    }
    if !table_exists(conn, "applied_migrations")? {
        return Ok(None);
    }
    // A missing row is trusted. Only a present row with a different hash is
    // a divergence (KTD2).
    let hashes = schema::migration_hashes();
    let mut stmt =
        conn.prepare("SELECT version, sha256 FROM applied_migrations ORDER BY version")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (version, sha256) = row?;
        if version < 1 || version > found {
            continue;
        }
        let Some(own_hash) = hashes.get((version - 1) as usize) else {
            continue;
        };
        if *own_hash != sha256 {
            return Ok(Some(NotOwnReason::DifferentMigration { version }));
        }
    }
    Ok(None)
}

/// Count the unfinished work in the database (KTD3, items 1 to 3). A table
/// or column that does not exist counts as zero.
fn count_database_work(conn: &Connection) -> rusqlite::Result<UnfinishedWork> {
    let mut work = UnfinishedWork::default();
    if table_exists(conn, "sessions")? && column_exists(conn, "sessions", "state")? {
        work.notes = count(
            conn,
            "SELECT count(*) FROM sessions
             WHERE json_extract(state, '$.state') IS NULL
                OR json_extract(state, '$.state') NOT IN ('saved', 'discarded')",
        )?;
    }
    if table_exists(conn, "jobs")? {
        work.jobs += count(conn, "SELECT count(*) FROM jobs")?;
    }
    if table_exists(conn, "sessions")? && column_exists(conn, "sessions", "transcribe_again_model")?
    {
        work.jobs += count(
            conn,
            "SELECT count(*) FROM sessions WHERE transcribe_again_model IS NOT NULL",
        )?;
    }
    for table in ["move_journal", "move_notes"] {
        if table_exists(conn, table)? && count(conn, &format!("SELECT count(*) FROM {table}"))? > 0
        {
            work.open_move = true;
        }
    }
    if table_exists(conn, "rename_intents")? {
        work.renames = count(conn, "SELECT count(*) FROM rename_intents")?;
    }
    if table_exists(conn, "result_state")? && column_exists(conn, "result_state", "key")? {
        work.publications = conn.query_row(
            "SELECT count(*) FROM result_state WHERE key = ?1",
            [SUMMARY_PUBLICATION_KEY],
            |r| r.get::<_, i64>(0),
        )? as u64;
    }
    Ok(work)
}

fn count(conn: &Connection, sql: &str) -> rusqlite::Result<u64> {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0))
        .map(|n| n as u64)
}

fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2",
        [table, column],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
}

/// Count the unfinished publications in the working folder (KTD3, item 4):
/// `sessions/*/publication.json` files and `sessions/.longhand-summary-*`
/// folders. A missing working folder counts as zero.
fn count_folder_work(working: &Path) -> io::Result<u64> {
    let entries = match std::fs::read_dir(working) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        other => other?,
    };
    let mut publications = 0;
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let summary_folder = entry
            .file_name()
            .to_string_lossy()
            .starts_with(SUMMARY_FOLDER_PREFIX);
        if summary_folder || entry.path().join(PUBLICATION_RECORD).is_file() {
            publications += 1;
        }
    }
    Ok(publications)
}
