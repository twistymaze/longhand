---
title: Removed transcript text stayed in the notes database, its WAL and its backups
date: 2026-10-31
category: security-issues
module: Notes database (crates/session-core/src/jobs.rs, backup.rs, migrations 0024 and 0025)
problem_type: security_issue
component: database
severity: high
symptoms:
  - Planning of transcript removal found that the store kept the recognition rows and the summary cache of each saved note, so a removed transcript stayed readable in longhand.db
  - A SQLite DELETE leaves the old text in free pages of the database file and in the WAL
  - Kept rebuild backups held the same rows, and a backup that the cleanup cannot open still holds them
root_cause: data_integrity
resolution_type: code_fix
related_components:
  - pipeline
  - src-tauri
tags:
  - transcript-removal
  - privacy
  - sqlite
  - secure-delete
  - vacuum
  - wal
  - backups
  - store-is-an-index
---

# Removed transcript text stayed in the notes database, its WAL and its backups

## Problem

Transcript removal (PR 87) deletes `transcript.md` and `recognition.jsonl` from the note folder. Before that work, the store also kept a copy of the same text after a save: the recognition checkpoints and rows (Transcribe Again reused them) and the summary cache (Regenerate Summary reused it). A deletion of the files only would leave the text in `longhand.db`, so the removal that the user asked for would not be complete.

## Root cause

The store was used as a second copy of note content, not as an index. Three SQLite facts also keep deleted text on disk:

- A `DELETE` marks pages free. The old bytes stay in the file until SQLite writes over them.
- In WAL mode, the deleted rows and the old pages stay in `longhand.db-wal` until a checkpoint.
- A rebuild backup is a full copy of the database files, with the same rows.

## Solution

- **The store is an index.** After a save, the store keeps only index data: state, snapshot, `found_speakers` and fingerprints. `Store::apply_clearing_content` clears the recognition data in the same transaction as the save. The summary cache goes after the title job, because only a waiting title request or a regeneration uses it. Transcribe Again transcribes again, and Resume reads the timed text from `recognition.jsonl` (see CONCEPTS.md, Store).
- **Delete for real.** The store opens with `PRAGMA secure_delete = ON`, so a delete writes over the freed bytes. After a clear, `Store::checkpoint_wal` runs `PRAGMA wal_checkpoint(TRUNCATE)`, so the WAL holds no old pages.
- **One VACUUM for earlier deletes.** Rows that earlier builds deleted without `secure_delete` stay in free pages. Migration 0025 sets `computer.vacuum_owed = 1`, and the store runs `VACUUM` at open, outside the migration transaction, then clears the flag. A failed VACUUM (for example a full disk) keeps the flag, and the next open tries again.
- **Backups.** At each launch, a background thread opens each kept backup with its own connection, never `Store::open` (which migrates), deletes the same rows for old schemas too (it reads the note column `note_id` or `session_id` of each table), and runs `VACUUM` and a WAL checkpoint when rows went. A backup that it cannot open or change is deleted, but only after the current store passes `PRAGMA quick_check`. The check runs only when a backup cannot be cleaned, because it locks the store for a full read.

## Prevention

Before a feature keeps note content in the store, ask whether the file in the note folder can serve the same read. The store must not hold text that a user can ask Longhand to delete. A new table or checkpoint that holds such text needs a clear at save, a line in migration-style cleanup, and the same rule in `backup::clean_backups`. The tests in `crates/session-core/tests/jobs.rs` (`after_the_clears_and_a_wal_checkpoint_no_file_holds_the_text`) and `crates/session-core/tests/backup.rs` search the database files for the text bytes, not only the rows.
