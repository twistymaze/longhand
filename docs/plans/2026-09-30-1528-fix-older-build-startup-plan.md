---
title: Older Build Startup - Plan
type: fix
date: 2026-09-30
topic: older-build-startup
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Older Build Startup - Plan

## Goal Capsule

- **Objective:** An older Longhand build starts after a newer build has used the same data. It never crashes at launch. When it cannot start, it tells the user why and what to do.
- **Product authority:** This plan. The storage folder stays the authority for saved notes (`CONCEPTS.md`, Library).
- **Open blockers:** None.

---

## Product Contract

### Summary

When the notes database is from a newer build, the older build rebuilds the database from the storage folder and starts. If the database holds unfinished work, the user first chooses between Quit and Rebuild. Every other startup failure shows an alert with the reason, then quits.

### Problem Frame

A plan build from a worktree adds a migration to `crates/session-core/migrations/`. When that build runs, the database version in `longhand.db` goes up. After that, main's build cannot open the database. `rusqlite_migration` refuses a database whose version is above its last migration (`DatabaseTooFarAhead`), and it refuses even when the new migration only adds a column. The error goes from `CaptureManager::new` (`src-tauri/src/managers/capture.rs:961`) to the Tauri `setup` closure. `.build(...).expect(...)` (`src-tauri/src/lib.rs:374`) then panics. The app closes with no message and no log line.

The local settings are not the cause. `LocalSettings` ignores unknown keys, and a parse failure salvages the valid fields (`src-tauri/src/managers/settings.rs:910`).

The problem occurs each time the user tests a plan build and then goes back to main's build. The workaround is to delete the database by hand.

### Key Decisions

- **Rebuild from the storage folder instead of a compatibility rule for each migration.** Saved notes can be rebuilt: the library scan adopts every home folder that has no row, including this computer's own notes (`crates/library/src/reconcile.rs:611`). Governs R1, R3. (session-settled: user-approved — chosen over a per-migration compatibility floor: a rebuild covers the same case, and no migration needs a compatibility decision)
- **The user decides when a rebuild would lose unfinished work.** Governs R4, R5. (session-settled: user-directed — chosen over "alert and quit" and "rebuild without asking": the user wants the choice, with a clear warning of the loss)
- **No startup failure crashes the app.** Governs R7.

### Requirements

**Newer database**

- R1. When the database version is above the build's last migration, and the database holds no unfinished work, the build rebuilds the database and starts with no alert.
- R2. Unfinished work is any of these: a note that is not Saved (recording, stopped, queued, processing or saving), notes typed with no session, an open Move, or a publication that has not finished.
- R3. A rebuild keeps the newer database as a backup next to the new one, then makes an empty database at the build's own version. The library scan at launch then adopts every saved note in the storage folder.
- R4. When the database holds unfinished work, the build shows an alert before it starts. The alert says that a newer version of Longhand has unfinished notes, and that a rebuild removes them from the list. It gives two choices: Quit and Rebuild.
- R5. Quit closes the app and changes no file. Rebuild does what R3 describes.
- R6. When the build cannot read the newer database to check for unfinished work, it counts the database as holding unfinished work (R4).

**Other startup failures**

- R7. Any error that stops the app from starting shows an alert with a short plain description and the error text, then quits. The error is also written to the log. The app does not panic.

**Records**

- R8. Each rebuild writes a log line with the old and new database versions and the path of the backup.

### Acceptance Examples

- AE1. **Covers R1, R3.** Given main's build after a plan build ran migration 0015, and all notes are Saved. When main's build starts, then it starts normally, every saved note is in the list, and the old database is kept as a backup.
- AE2. **Covers R4, R5.** Given a plan build quit while a note was queued for processing. When main's build starts, then the alert shows. Quit closes the app and leaves `longhand.db` unchanged. Rebuild starts the app, and the queued note is not in the list.
- AE3. **Covers R3.** Given AE1 is complete. When the plan build starts again, then it migrates the rebuilt database up as usual and lists every saved note.
- AE4. **Covers R7.** Given the app data folder cannot be written. When the app starts, then an alert shows the reason and the app quits. No crash report is made.

### Scope Boundaries

- A database from a newer build is never opened or changed in place by the older build. The rebuild does not copy rows from it.
- The newer build does not read the backup. Recovery of unfinished work from the backup is manual (open the newer build with the backup file).
- Out of scope: the older build drops new settings keys when it writes the settings file. Loading is not affected, and the newer build gives those keys their defaults again. Record this as a separate idea if it matters.
- Out of scope: separate data folders for test builds.

### Outstanding Questions

**Deferred to Planning**

- Which database rows only this computer holds, beyond R2's list, and what a rebuild loses for a saved note (for example the owed folder renames, the recognition snapshot, the prompt snapshot). Confirm that each loss has the same effect as a note that another computer recorded.
- The backup file name, and whether to keep only the most recent backup. The SQLite `-wal` and `-shm` files must move with the database.
- Where the version check runs so that no other startup step opens the database first.
- How the alert shows before the main window exists (`tauri-plugin-dialog` is already a dependency).

### Sources / Research

- `crates/session-core/src/schema.rs` and `crates/session-core/src/jobs.rs:321`: migrations and `to_latest`.
- `docs/solutions/workflow-issues/edited-applied-migration-breaks-older-databases.md`: migration rules for this store.
- `crates/session-core/migrations/0001_init.sql`: the store is the working state for sessions not yet saved, and every saved row can be rebuilt from the storage folder.
