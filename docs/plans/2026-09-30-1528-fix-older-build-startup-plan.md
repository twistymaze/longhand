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

When the notes database is not this build's own, the older build rebuilds the database from the storage folder and starts. If the database holds unfinished work, the user first chooses between Quit and Rebuild. Every other startup failure shows an alert with the reason and a next step, then quits.

### Problem Frame

A plan build from a worktree adds a migration to `crates/session-core/migrations/`. When that build runs, the database version in `longhand.db` goes up. After that, main's build cannot open the database. `rusqlite_migration` refuses a database whose version is above its last migration (`DatabaseTooFarAhead`), and it refuses even when the new migration only adds a column. The error goes from `CaptureManager::new` (`src-tauri/src/managers/capture.rs:961`) to the Tauri `setup` closure. `.build(...).expect(...)` (`src-tauri/src/lib.rs:374`) then panics. The app closes with no message and no log line.

A second shape of the same problem: two plan branches can each add a different migration 0015, and the branch that merges second is renumbered (`docs/solutions/workflow-issues/parallel-branches-bump-same-schema-version.md`). A database from the first branch then has main's version number but a different schema.

The local settings are not the cause. `LocalSettings` ignores unknown keys, and a parse failure salvages the valid fields (`src-tauri/src/managers/settings.rs:910`).

The problem occurs each time the user tests a plan build and then goes back to main's build. The workaround is to delete the database by hand.

### Key Decisions

- **Rebuild from the storage folder instead of a compatibility rule for each migration.** Saved notes can be rebuilt: the library scan adopts every home folder that has no row, including this computer's own notes (`crates/library/src/reconcile.rs:611`). Governs R1, R3, R4. (session-settled: user-approved — chosen over a per-migration compatibility floor: a rebuild covers the same case, and no migration needs a compatibility decision)
- **The user decides when a rebuild would lose unfinished work.** Governs R5, R6, R7. (session-settled: user-directed — chosen over "alert and quit" and "rebuild without asking": the user wants the choice, with a clear warning of the loss)
- **No startup failure crashes the app.** Governs R9.

### Requirements

**Database that is not this build's own**

- R1. The database is not this build's own when its version is above the build's last migration, or when the migrations that it records differ from the build's migrations.
- R2. When the database is not this build's own, holds no unfinished work, and this build holds the instance lock, the build rebuilds the database and starts with no alert.
- R3. Unfinished work is any of these: a note that is not Saved (recording, stopped, queued, processing or saving), an open Move, or a publication that has not finished.
- R4. A rebuild moves the database (with its `-wal` and `-shm` files) and the working folder (`sessions/` in the app data folder) aside as one backup. It then makes an empty database at the build's own version and an empty working folder. The library scan at launch adopts every saved note in the storage folder.
- R5. After a rebuild, each saved note has the same title, notes, transcript, summary and audio as before. State that only this computer held for a saved note is lost, as for a note that another computer recorded.
- R6. When the database holds unfinished work, the build shows an alert before it starts. The alert names the kinds of work that it found (notes, a Move, a publication), says that a rebuild removes this work, and says that the backup is the way to recover it. It gives two choices: Quit and Rebuild.
- R7. When the build cannot read the database to check for unfinished work, it shows the R6 alert, but the alert says that the check failed instead of naming work.
- R8. Quit closes the app and changes no file. Rebuild does what R4 describes.

**Other startup failures**

- R9. Any error that stops the app from starting shows an alert with a short plain description, a next step, and the error text, then quits. The error is also written to the log. The app does not panic. This includes a database that is not this build's own while this build does not hold the instance lock.

**Records**

- R10. Each rebuild writes a log line with the old and new database versions and the path of the backup.

### Acceptance Examples

- AE1. **Covers R1, R2, R4, R5.** Given main's build after a plan build ran migration 0015, and all notes are Saved. When main's build starts, then it starts normally, every saved note is in the list with its title, notes, transcript, summary and audio, and the old database and working folder are kept as a backup.
- AE2. **Covers R1, R2.** Given a database at version 15 from a branch whose migration 0015 differs from main's migration 0015, and all notes are Saved. When main's build starts, then it rebuilds as in AE1 and does not quit on a later database error.
- AE3. **Covers R6, R8.** Given a plan build quit while a note was queued for processing. When main's build starts, then the alert shows and names an unfinished note. Quit closes the app and leaves `longhand.db` unchanged. Rebuild starts the app, and the queued note is not in the list.
- AE4. **Covers R4.** Given AE1 is complete. When the plan build starts again, then it migrates the rebuilt database up as usual and lists every saved note.
- AE5. **Covers R9.** Given the app data folder cannot be written. When the app starts, then an alert shows the reason and a next step, the error is in the log, and the app quits. No crash report is made.

### Scope Boundaries

- A database that is not this build's own is never opened or changed in place by the build. The rebuild does not copy rows from it.
- The newer build does not read the backup. Recovery of unfinished work from the backup is manual: put back both the database and the working folder, then open the newer build.
- Out of scope: the older build drops new settings keys when it writes the settings file. Loading is not affected, and the newer build gives those keys their defaults again. Record this as a separate idea if it matters.
- Out of scope: separate data folders for test builds.

### Outstanding Questions

**Deferred to Planning**

- Which database rows only this computer holds for a saved note (for example the owed folder renames, the recognition snapshot, the prompt snapshot). A state whose loss would change what the user sees of a note goes into R3.
- How the build records its migrations so that it can find a database whose migrations differ (R1).
- The backup name, and whether to keep only the most recent backup.
- Where the check runs so that no other startup step opens the database first.
- How the alert shows. `tauri-plugin-dialog` cannot do this: a blocking dialog must not run on the main thread, where `setup` runs, and after a `setup` error `build` returns no app. The alert needs a dialog that does not depend on the Tauri app.

### Sources / Research

- `crates/session-core/src/schema.rs` and `crates/session-core/src/jobs.rs:321`: migrations and `to_latest`.
- `docs/solutions/workflow-issues/edited-applied-migration-breaks-older-databases.md`: migration rules for this store.
- `crates/session-core/migrations/0001_init.sql`: the store is the working state for sessions not yet saved, and every saved row can be rebuilt from the storage folder.
- `crates/pipeline/src/old_audio.rs`: the launch sweep deletes the old-audio copy of each working folder that has no row.
- `src-tauri/src/lib.rs` (`take_instance_lock`): startup continues when the instance lock is unavailable.

### Residual Risks

- If a plan build also changes the note folder format, the older build's library scan skips those notes with only a log warning, and they disappear from the list with no message.
