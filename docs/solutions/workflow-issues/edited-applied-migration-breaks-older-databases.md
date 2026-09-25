---
title: A rename written into an applied SQL migration broke every older database
date: 2026-09-25
category: workflow-issues
module: Session store migrations (crates/session-core/src/schema.rs, crates/session-core/migrations/)
problem_type: workflow_issue
component: development_workflow
related_components:
  - database
severity: high
applies_when:
  - "A change renames or removes a column, a table, or a stored value in crates/session-core"
  - "A change edits any file in crates/session-core/migrations/ that a release or test build already applied"
  - "A terminology rename (for example land to save) touches SQL"
symptoms:
  - "New Note fails with: No such column: home_path in SELECT id, created_at, state, ... FROM sessions"
  - "Only a machine with a database made by an older build fails; a new database works"
  - "All tests pass, because each test makes a new database"
root_cause: missing_workflow_step
resolution_type: workflow_improvement
tags:
  - sqlite
  - migrations
  - rusqlite-migration
  - user-version
  - schema
  - rename
---

# A rename written into an applied SQL migration broke every older database

## Context

Commit 399f360 (2026-09-24) renamed the "land" terms to "save". It changed the column `landed_path` to `home_path`, and the stored state `landed` to `saved`. It made the column change by editing `crates/session-core/migrations/0001_init.sql` in place. No new migration was added.

`crates/session-core/src/schema.rs` uses `rusqlite_migration`, which records the applied version in SQLite's `user_version` pragma. It runs only the migrations above that version. A database made before 399f360 is already at version 1 or higher, so the edited 0001 never runs on it. The code then selects `home_path` from a table that still has `landed_path`.

On 2026-09-25, the notarized build of PR #9 ran on a laptop that last had a build from before the rename. New Note failed at once with `No such column: home_path`. The tests did not find this, because each test makes a new database from the current migrations.

## Guidance

- **Never edit a migration after a build applied it.** Treat every file in `crates/session-core/migrations/` as fixed once any build, including a test build on a real machine, has run it. Put each schema change in a new numbered file (`0006_...sql`), added to the list in `schema.rs`.
- **A rename needs a migration for the data too.** A column rename is `ALTER TABLE sessions RENAME COLUMN old TO new;`. A renamed stored value also needs an `UPDATE`, for example `UPDATE sessions SET state = 'saved' WHERE state = 'landed';`. Check for stored names in other places too, for example stage names in checkpoint rows.
- **Comments and text-only edits are safe.** A change to a comment in an old migration does not change the schema. Keep a column, table or value change out of that edit.
- **Test from an old schema.** A migration test should open a database made by the previous migrations, with rows in it, then apply the new migration. A test that starts from an empty database cannot find this class of bug.

## Why This Matters

The failure shows only on real machines with old data, never in CI. It also blocks the main task of the app, starting a note. The user has two fixes: delete the database, which removes the old notes from the list (their folders stay on disk), or fix the database by hand. Neither fix is acceptable when the app has users.

## When to Apply

- Before you commit any change under `crates/session-core/migrations/`, run `git log -- <file>`. If the file exists on main, add a new migration instead.
- When a terminology change in `CONCEPTS.md` renames a term that SQL stores (a column, a state, an event, a stage).
- When two branches add migrations at the same time, also see `docs/solutions/workflow-issues/parallel-branches-bump-same-schema-version.md`.

## Examples

Wrong (399f360 edited the applied migration):

```sql
-- crates/session-core/migrations/0001_init.sql
-    landed_path     TEXT
+    home_path     TEXT
```

Right (a new migration that each old database runs once):

```sql
-- crates/session-core/migrations/0006_rename_landed.sql
ALTER TABLE sessions RENAME COLUMN landed_path TO home_path;
UPDATE sessions SET state = 'saved' WHERE state = 'landed';
```

For the laptop, the same two statements, run by hand with `sqlite3`, fix the database. No migration was added, because no other database from before the rename is in use.
