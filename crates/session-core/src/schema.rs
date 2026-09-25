//! Migrations, on Handy's `rusqlite_migration` harness
//! (`reference-source/Handy/src-tauri/src/managers/history.rs`), which tracks
//! the applied version in SQLite's `user_version` pragma.

use rusqlite::Transaction;
use rusqlite_migration::{HookResult, M, Migrations};

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("../migrations/0001_init.sql")),
        M::up_with_hook(
            include_str!("../migrations/0002_titles_and_notes.sql"),
            notes_to_documents,
        ),
        M::up(include_str!("../migrations/0003_rename_intent.sql")),
    ])
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
