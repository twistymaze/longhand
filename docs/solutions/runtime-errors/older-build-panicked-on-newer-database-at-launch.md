---
title: An older build panicked at launch on a database that a newer build migrated
date: 2026-09-30
category: runtime-errors
module: Store open and launch check (crates/session-core/src/schema.rs, inspect.rs, backup.rs; src-tauri/src/startup.rs, lib.rs)
problem_type: runtime_error
component: service_layer
related_components:
  - tauri_backend
  - database
severity: high
symptoms:
  - "An older build closes at launch with a panic: Failed to setup app"
  - "Store::open returns rusqlite_migration DatabaseTooFarAhead after a newer build applied a migration that the older build does not know"
  - "The log has no error line for the launch, and a delete of longhand.db makes the app start again"
root_cause: missing_validation
resolution_type: code_fix
framework_version: "tauri 2.11.6, rusqlite_migration 2.5.0, rust 1.94.0"
tags:
  - sqlite
  - rusqlite-migration
  - database-too-far-ahead
  - older-build
  - launch-check
  - tauri-setup-panic
  - database-backup
  - applied-migrations
---

# An older build panicked at launch on a database that a newer build migrated

## Problem

A plan build from a worktree applied migration 0015 to `longhand.db` in the app data folder. After that, main's build closed at launch with no message and no log line. The only workaround was to delete the database by hand. The problem occurred each time the user tested a plan build and then went back to main's build.

The chain of the failure. A path of the form `<crate>-<version>/src/...` is in the Cargo registry (`~/.cargo/registry/src/`), not in this repo. `sessions/` and `database-backups/` are folders in the app data folder.

1. `Store::open` calls `Store::init`, which runs `migrations().to_latest(&mut conn)` (`crates/session-core/src/jobs.rs:309-322`).
2. `rusqlite_migration` 2.5.0 refuses a database whose `user_version` is above its last migration. It returns `MigrationDefinitionError::DatabaseTooFarAhead` (`rusqlite_migration-2.5.0/src/lib.rs:720`). It refuses also when the new migration only adds a column.
3. `CaptureManager::new` opens the store (`src-tauri/src/managers/capture.rs:961`) and returns the error to the Tauri `setup` closure.
4. Tauri 2.11.6 runs `setup` inside `app.run`. When `setup` returns `Err`, Tauri panics with "Failed to setup app" (`tauri-2.11.6/src/app.rs:1425` and `:1477`). The app closes before the logger writes the error.

A second form of the same problem: two plan branches can each add a different migration 0015 (`docs/solutions/workflow-issues/parallel-branches-bump-same-schema-version.md`). A database from one branch then has main's version number but a different schema. `to_latest` accepts that database, and a later query fails.

The fix is in PR #30.

## Symptoms

- Main's build closes immediately at launch, after a plan build ran on the same computer.
- No alert shows, and `~/Library/Logs/net.ewal.longhand/Longhand.log` has no error line for the launch.
- macOS can show a crash report, because the cause is a panic.
- A delete of `longhand.db` makes the app start again.

## What Didn't Work

- **A suspicion that the local settings file caused the crash.** The user first suspected the settings JSON that the newer build wrote. The settings are not the cause. `settings_from_value` parses the stored value into `LocalSettings`, and on a parse error it salvages the valid fields (`src-tauri/src/managers/settings.rs:911-926`). Unknown keys from a newer build do not stop the load.
- **A compatibility rule for each migration** (rejected in planning). Each new migration would need a decision about which older builds can still open the database. The decision is easy to forget, and a wrong decision corrupts data. A rebuild from the storage folder covers the same case, and no migration needs a decision.
- **`tauri-plugin-dialog` for the alerts** (rejected in planning, KTD6). Its `blocking_show` "cannot be executed on the main thread as it will freeze your application" (`tauri-plugin-dialog-2.7.3/src/lib.rs:160`). `setup` runs on the main thread. After a `setup` error, no app exists to show a plugin dialog.
- **Keep only the latest backup** (rejected in planning). A second rebuild would then delete a backup that holds the only copy of unfinished work.

## Solution

The key fact: no data lives only in the database. Each saved session has its home folder in the storage folder. The library scan adopts every home folder that has no row (`crates/library/src/reconcile.rs:611`), at launch and then every 60 s (`src-tauri/src/managers/library.rs:77`). So an older build does not try to read a newer database. It moves the database aside and makes a new one. The library scan then lists every saved session again.

### 1. Record the identity of each applied migration

The SQL of each migration is in one list, `MIGRATION_SQL` (`crates/session-core/src/schema.rs:10-25`). Both `migration_steps` and `migration_hashes` read it. `Store::init` records a hash for each applied version after `to_latest` (`crates/session-core/src/jobs.rs:321-322`):

```rust
crate::schema::migrations().to_latest(&mut conn)?;
crate::schema::record_applied_migrations(&conn, &crate::schema::migration_hashes())?;
```

The hash is SHA-256 of the SQL with comments and whitespace removed (`schema.rs:54-100`), so a comment edit is not a divergence. `record_applied_migrations` uses `INSERT OR IGNORE`, so a present row does not change (`schema.rs:117-136`). The `applied_migrations` table needs no migration file, so the database version does not change.

### 2. Inspect the database read-only at launch

`inspect::inspect` returns `Own`, `NotOwn { reason, work }` or `Unreadable { error }` (`crates/session-core/src/inspect.rs:39-52`). A missing or zero-byte database is `Own`, and the inspection opens no file (`inspect.rs:99-108`).

The inspection opens the database with `SQLITE_OPEN_READ_ONLY` (`inspect.rs:162-165`). It does not use `immutable=1`, because `immutable` ignores the WAL, and the inspection then misses rows that are only in `-wal`. It never calls `Store::open`, which writes pragmas and migrates.

`not_own_reason` finds two reasons (`inspect.rs:175-203`):

- `NewerVersion`: `user_version` is above this build's migration count.
- `DifferentMigration`: a recorded hash differs from this build's hash for the same version. This finds a branch build with the same version number but different SQL. A missing row is trusted, so a stop between a migration and its row is safe.

For `NotOwn`, the inspection counts the unfinished work (`inspect.rs:207-244`, `:274-294`): sessions in a state other than `saved` or `discarded` (an unknown state counts), rows in `jobs`, sessions with a `transcribe_again_model`, rows in `move_journal` or `move_notes`, rows in `rename_intents`, `summary-publication` rows in `result_state`, `sessions/*/publication.json` files and `sessions/.longhand-summary-*` folders. A table or column that does not exist counts as zero.

SQLite can make `-shm` or `-wal` for a read-only open. The inspection records which side files existed before the open, and after the close it deletes each side file that it made (`inspect.rs:110-124`). So a Quit in the alert changes no file.

### 3. Run the check only as the only instance

Because the inspection deletes side files, a second live instance could lose its `-shm` file. `check_database` therefore runs only when the instance lock gives `Liveness::OnlyInstance` (`src-tauri/src/startup.rs:304-311`). Without the lock the check is skipped, and a database error then stops the start with the startup failure alert.

`setup_app` runs the check after `take_instance_lock` and `install_at_launch`, before any reader opens `longhand.db` (`src-tauri/src/lib.rs:207-217`). An update that installs at launch restarts into a newer build before any alert shows.

`decide` maps the inspection to an action (`startup.rs:85-98`):

- `Own`: start normally.
- `NotOwn` with no unfinished work: rebuild with no alert.
- `NotOwn` with unfinished work, or `Unreadable`: show an alert with Quit and Rebuild.

The alert for unfinished work names each kind of unfinished work and says that a rebuild removes it and that the backup is the way to recover it (`startup.rs:148-180`). For `DifferentMigration`, the text says "another version", because a hash does not tell which version is newer. Quit is the first button, so it is the default (`startup.rs:208-220`). Each result other than the Rebuild button is Quit (`startup.rs:199-204`). Quit exits with code 0 and changes no file (`startup.rs:341-345`).

### 4. Move the database aside with renames only

`backup::move_aside` makes `database-backups/<UTC time>/` and renames four entries into it, in this order: `sessions/`, `longhand.db-shm`, `longhand.db-wal`, `longhand.db` (`crates/session-core/src/backup.rs:57` for the order, `:209-223` for the function). An entry that does not exist is skipped. Every step is a rename on one volume, so the move needs no free space.

The step order makes each crash state recognizable from disk (`backup.rs:244-277`):

```rust
write_synced(&flag, INCOMPLETE_TEXT)?;
if kind == BackupKind::ChosenInAlert {
    write_synced(&folder.join(REBUILD_MARKER), MARKER_TEXT)?;
}
sync_dir(folder)?;
sync_dir(&app_data.join(BACKUPS_DIR))?;
sync_dir(app_data)?;
// ... the renames ...
sync_dir(folder)?;
sync_dir(app_data)?;
fs::remove_file(&flag).map_err(io_error("remove", &flag))
```

The `move-incomplete` flag goes first. `write_synced` calls `sync_all` on the flag and the marker (`backup.rs:280-285`). The syncs of the directories before the first rename and after the last rename were added after code review: without them, a power loss could keep the renames and lose the flag, and the undo at the next launch then skips the folder.

A rename that fails retries for 2 s (Windows sharing violations), then the renames that succeeded move back (`backup.rs:107-126`, `:225-242`). At the next launch, `undo_incomplete` moves the entries of each folder that still has the flag back into the app data folder (`backup.rs:140-158`). `move_orphan_wal` moves a `-wal` or `-shm` with no database aside, so SQLite never applies a stale WAL to a new database (`backup.rs:191-205`).

Retention keeps every backup with the `rebuild-chosen` marker and the newest 3 automatic backups (`backup.rs:47`, `:363-374`). A retention failure only writes a warning and does not stop the start (`startup.rs:284-291`). The rebuild writes one log line with the old version, the new version and the backup path (`startup.rs:292-297`).

### 5. No startup error leaves `setup`

The `setup` body is in `setup_app`, which returns a `Result` (`src-tauri/src/lib.rs:198-201`). The closure never returns `Err` (`lib.rs:394-401`):

```rust
.setup(move |app| {
    if let Err(e) = setup_app(app, &specta_builder) {
        startup::fail(e.as_ref(), startup::log_file(app.handle()).as_deref());
    }
    Ok(())
})
```

`startup::fail` logs the error, flushes the logger, shows an `rfd` alert with a description, a next step, the log file path and the error text, and exits with code 1 (`startup.rs:241-256`). The `.build(context)` error gets the same treatment (`lib.rs:404-406`). No app handle exists then, so `log_file_for` builds the log path `~/Library/Logs/<identifier>/<product name>.log` on macOS (`startup.rs:225-231`).

## Why This Works

- The storage folder is the authority for saved sessions (CONCEPTS.md, Library). The database is a cache of it plus the state of unfinished work. When no unfinished work exists, a new database loses nothing that the library scan cannot adopt again.
- The check does not depend on what a newer migration changed. Any migration that a newer build adds raises `user_version` above the older build's count, and any different SQL at the same version gives a different hash. So each new migration is covered with no extra code.
- The inspection never writes the database and removes only the side files that it made. So the user can choose Quit and open the newer build, and that build finds its database unchanged.
- Renames only, with a flag that is on disk before the first rename and removed only after the last rename is on disk, keep one backup in one folder. A crash at any step either leaves a complete backup or leaves a flag that the next launch undoes.
- A `setup` closure that never returns `Err` removes the panic path in Tauri. Each startup failure now shows a reason and a next step, and the log has the error.

## Prevention

- Never return `Err` from the Tauri `setup` closure. Tauri 2.11 panics on it, and the panic hides the cause. Add new startup steps to `setup_app`, and give a recoverable state its own handling before the error can reach `setup`.
- Do not add a compatibility decision to a new migration. The ownership check covers each new migration automatically, as long as the migration SQL goes into `MIGRATION_SQL` in `schema.rs`.
- Never edit the SQL of a migration that a build already applied. A changed statement changes the hash, and every database that has the old hash is then `DifferentMigration` and rebuilt. Comment and whitespace edits do not change the hash. See `docs/solutions/workflow-issues/edited-applied-migration-breaks-older-databases.md`.
- When new state can hold the only copy of user work (a new table, a new staging file or folder), add it to the unfinished work count in `inspect.rs`. Otherwise a rebuild removes it with no alert.
- To test the path manually in a signed bundle: quit the app, copy the app data folder aside, then set `PRAGMA user_version = 99` on `longhand.db`. With all sessions saved, expect no alert, every saved note in the list, and a new folder in `database-backups/`. With one session queued, expect the alert that names one note. The automated tests use the same method (`crates/session-core/tests/inspect.rs:375`).
- A database from a branch that was cut before PR #30 has no hash rows. It is trusted, so a different migration at the same version is not found. This gap closes when both builds carry the fix.

## Related

- [A rename written into an applied SQL migration broke every older database](../workflow-issues/edited-applied-migration-breaks-older-databases.md): the opposite direction (a newer build on an older database). The hash in `applied_migrations` now finds an edited applied migration as `DifferentMigration`.
- [Parallel worktree PRs that each raise a schema version must renumber at merge](../workflow-issues/parallel-branches-bump-same-schema-version.md): installed builds of unmerged branches are the usual source of a newer or different database.
- [A crash between a save step and its record stranded or duplicated a completed session](../logic-errors/save-state-machine-crash-window-recovery.md): the same rename-only, flag-first crash recovery that `backup.rs` uses.
- [Rules for settings that several computers share through a synced folder](../integration-issues/shared-settings-in-a-synced-folder-rules.md): the shared settings file at a newer schema is still read. The database is moved aside instead, because it is a local cache of the storage folder and the library scan can make it again.
