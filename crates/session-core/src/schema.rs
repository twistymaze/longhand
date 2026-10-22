//! Migrations, on Handy's `rusqlite_migration` harness
//! (`reference-source/Handy/src-tauri/src/managers/history.rs`), which tracks
//! the applied version in SQLite's `user_version` pragma.

use rusqlite::{Connection, Transaction};
use rusqlite_migration::{HookResult, M, Migrations};

/// The SQL of each migration, in version order. Version `n` is item `n - 1`.
/// [`migration_steps`] and [`migration_hashes`] both read this list (KTD2).
pub const MIGRATION_SQL: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_titles_and_notes.sql"),
    include_str!("../migrations/0003_rename_intent.sql"),
    include_str!("../migrations/0004_live_and_transcribe_again.sql"),
    include_str!("../migrations/0005_summary_not_updated.sql"),
    include_str!("../migrations/0006_capture_choice_and_channels.sql"),
    include_str!("../migrations/0007_transcript_fingerprint.sql"),
    include_str!("../migrations/0008_owed_renames.sql"),
    include_str!("../migrations/0009_recognition_snapshot.sql"),
    include_str!("../migrations/0010_recognition_output.sql"),
    include_str!("../migrations/0011_result_generations.sql"),
    include_str!("../migrations/0012_move_journal.sql"),
    include_str!("../migrations/0013_prompt_snapshot.sql"),
    include_str!("../migrations/0014_segment_audio_removed.sql"),
    include_str!("../migrations/0015_prompt_snapshot_title.sql"),
    include_str!("../migrations/0016_notes_only.sql"),
    include_str!("../migrations/0017_runs.sql"),
    include_str!("../migrations/0018_note_types_and_tags.sql"),
    include_str!("../migrations/0019_type_moves_and_bulk_edits.sql"),
    include_str!("../migrations/0020_segment_ends_in_pause.sql"),
    include_str!("../migrations/0021_note_app_set.sql"),
];

/// The version of migration 0002, which has a hook.
const NOTES_TO_DOCUMENTS_VERSION: usize = 2;

/// One step for each item of [`MIGRATION_SQL`]. Migration 0002 also runs
/// [`notes_to_documents`].
pub fn migration_steps() -> Vec<M<'static>> {
    MIGRATION_SQL
        .iter()
        .enumerate()
        .map(|(i, sql)| {
            if i + 1 == NOTES_TO_DOCUMENTS_VERSION {
                M::up_with_hook(sql, notes_to_documents)
            } else {
                M::up(sql)
            }
        })
        .collect()
}

pub fn migrations() -> Migrations<'static> {
    Migrations::new(migration_steps())
}

/// The migration SQL with its comments and all whitespace removed. A comment
/// is `--` to the end of the line, or a `/* */` block, outside a
/// single-quoted string. So a change to a comment or to the layout does not
/// change the hash (KTD2).
pub fn normalize_migration_sql(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            if c == '\'' {
                in_string = false;
            }
            if !c.is_whitespace() {
                out.push(c);
            }
            continue;
        }
        match c {
            '\'' => {
                in_string = true;
                out.push(c);
            }
            '-' if chars.peek() == Some(&'-') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
            }
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

/// The SHA-256, as lowercase hex, of the normalized migration SQL.
pub fn migration_hash(sql: &str) -> String {
    crate::folder::fingerprint(normalize_migration_sql(sql).as_bytes())
}

/// This build's hash of each migration, in version order.
pub fn migration_hashes() -> Vec<String> {
    MIGRATION_SQL
        .iter()
        .map(|sql| migration_hash(sql))
        .collect()
}

/// Create `applied_migrations` if it is missing. Then insert a row for each
/// version from 1 to `user_version` that has no row, with its hash from
/// `hashes` (item `n - 1` for version `n`). An existing row does not change.
/// A version with no item in `hashes` gets no row.
///
/// Run this after the migrations. A missing row is trusted, so a stop between
/// a migration and its row is safe (KTD2).
pub fn record_applied_migrations(conn: &Connection, hashes: &[String]) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let applied = usize::try_from(version).unwrap_or(0).min(hashes.len());
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS applied_migrations (
             version INTEGER PRIMARY KEY,
             sha256 TEXT NOT NULL
         );",
    )?;
    {
        let mut insert = tx.prepare(
            "INSERT OR IGNORE INTO applied_migrations (version, sha256) VALUES (?1, ?2)",
        )?;
        for (i, hash) in hashes[..applied].iter().enumerate() {
            insert.execute(rusqlite::params![i as i64 + 1, hash])?;
        }
    }
    tx.commit()
}

/// Migration 0002 (KTD5): write each session's note rows, in their order,
/// into its notes document, one line for each note in the form that
/// `notes.md` had before (R26). Then drop the notes table.
///
/// SQL cannot do this step: the old form puts each note on one line with its
/// whitespace collapsed, and gives the position as `hh:mm:ss`. The step reads
/// the anchor as plain JSON so that it does not depend on types that later
/// work can remove.
fn notes_to_documents(tx: &Transaction) -> HookResult {
    let mut documents: Vec<(String, String)> = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT session_id, text, anchor FROM notes
             WHERE session_id IS NOT NULL ORDER BY session_id, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (session_id, text, anchor) = row?;
            let line = note_line(&text, &anchor);
            match documents.last_mut() {
                Some((id, document)) if *id == session_id => document.push_str(&line),
                _ => documents.push((session_id, line)),
            }
        }
    }
    for (session_id, document) in documents {
        tx.execute(
            "UPDATE sessions SET notes = ?2 WHERE id = ?1",
            rusqlite::params![session_id, document],
        )?;
    }
    tx.execute_batch("DROP TABLE notes;")?;
    Ok(())
}

/// One note as a line: `- [hh:mm:ss] text`, `- [hh:mm:ss ~gap] text`, or
/// `- text` when the note has no position.
fn note_line(text: &str, anchor: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let anchor: serde_json::Value = serde_json::from_str(anchor).unwrap_or_default();
    let position = (anchor["kind"] == "at")
        .then(|| anchor["position_ms"].as_u64())
        .flatten();
    match position {
        None => format!("- {text}\n"),
        Some(ms) => {
            let gap = if anchor["gap_anchored"] == true {
                " ~gap"
            } else {
                ""
            };
            format!("- [{}{gap}] {text}\n", crate::folder::format_position(ms))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn migrations_are_valid() {
        super::migrations().validate().unwrap();
    }

    #[test]
    fn a_note_line_has_the_form_that_notes_md_had() {
        let at = |ms: u64, gap: bool| {
            format!(r#"{{"kind":"at","position_ms":{ms},"gap_anchored":{gap}}}"#)
        };
        assert_eq!(
            super::note_line("a  b\nc", r#"{"kind":"unanchored"}"#),
            "- a b c\n"
        );
        assert_eq!(
            super::note_line("x", &at(760_000, false)),
            "- [00:12:40] x\n"
        );
        assert_eq!(
            super::note_line("x", &at(3_723_000, true)),
            "- [01:02:03 ~gap] x\n"
        );
    }
}
