---
title: A fresh marker from this computer's earlier run blocked this computer's own note
date: 2026-10-01
category: logic-errors
module: In-use marker checks (crates/session-save/src/in_use_marker.rs, crates/library/src/scan.rs, src-tauri/src/commands/session.rs, crates/pipeline/src/empty_note.rs, crates/session-core runs table)
problem_type: logic_error
component: service_layer
symptoms:
  - "After the user quit or the app crashed while a note recorded, waited or processed, the next launch finished the work, but for up to 30 minutes the note page said the note was in use on another computer"
  - "Record, Resume, Delete, Transcribe Again, Regenerate Summary and Remove Audio were refused on the same computer that did the work"
  - "The empty-note pass at launch kept an empty note for 30 minutes after a crash"
root_cause: logic_error
resolution_type: code_fix
severity: high
tags: [in-use-marker, run-id, two-computers, sync, one-library, launch-reconcile, instance-lock]
related_components: [library, session_core, background_job]
---

# A fresh marker from this computer's earlier run blocked this computer's own note

## Problem

The in-use checks of PR #33 excepted only the in-use marker of the current run. After a quit or a crash, the next launch on the same computer read its own earlier marker as work on another computer, and refused the user's actions on that note for up to 30 minutes.

## Symptoms

- The user quits the app, or the app crashes, while a note records, waits or is processed. The next launch finishes the work.
- For up to 30 minutes after that, the note page says that the note is in use on another computer. The text is "This note is recording or being processed on another computer. Try again when it is done." (`src-tauri/src/commands/session.rs:132-133`).
- On the same computer that did the work, Record, Resume, Delete, Transcribe Again, Regenerate Summary and Remove Audio are refused (R21).
- The empty-note pass at launch keeps an empty note (`Kept::InUse`) for 30 minutes after a crash.
- Before PR #33, a stale marker of this computer only paused audio removal, so the defect was not visible.

The cause has three parts:

- Each launch makes a new run ID from the process ID and the time in nanoseconds (`crates/pipeline/src/runner.rs:307-311`).
- `InUseKeeper::stop` keeps the markers of open work when the app quits (`crates/pipeline/src/in_use.rs:97-103`). A marker stays fresh for `FRESH_FOR`, 30 minutes (`crates/session-save/src/in_use_marker.rs:37`).
- The checks compared each marker with the current run ID only. The marker of the earlier run on this computer had a different run ID, so it counted as a marker of another computer.

## What Didn't Work

- **Except only the current run.** This was the original defect. A run ID identifies one launch, not one computer. A quit or a crash keeps the earlier run's marker, and the next run does not recognize it.
- **A stable computer ID in the marker (rejected).** Each install would write a fixed ID into its markers, and each check would except markers with that ID. This fails when the app data is copied to a new Mac, for example by Migration Assistant or by a restore from a backup. Two computers then have the same ID. Each computer ignores the other computer's real markers, so two computers can record into or delete the same note. A list of run IDs stays correct after a copy: each computer makes new run IDs after the copy, and a copied run ID can only match a marker that the old computer wrote before the copy.

## Solution

The store keeps the run IDs of this computer. The in-use checks except all of these runs, not only the current run.

1. A new migration adds the table `runs` (`crates/session-core/migrations/0017_runs.sql:6-9`):

   ```sql
   CREATE TABLE runs (
       seq INTEGER PRIMARY KEY AUTOINCREMENT,
       id TEXT NOT NULL UNIQUE
   );
   ```

2. `Store::add_run` adds the current run, keeps the newest `RUNS_KEPT` runs, and returns the earlier runs, newest first (`crates/session-core/src/jobs.rs:1725-1741`). `RUNS_KEPT` is 16 (`crates/session-core/src/jobs.rs:316`).

   ```rust
   tx.execute("INSERT OR IGNORE INTO runs (id) VALUES (?1)", [run_id])?;
   tx.execute(
       "DELETE FROM runs WHERE seq NOT IN
        (SELECT seq FROM runs ORDER BY seq DESC LIMIT ?1)",
       [RUNS_KEPT as i64],
   )?;
   ```

3. At launch, `own_runs` in `src-tauri/src/managers/processing.rs:639-656` makes the list of this computer's runs, with the current run first. It adds the earlier runs only when this process holds the instance lock. If the store cannot add the run, only the current run counts:

   ```rust
   .add_run(run_id)
   .unwrap_or_else(|e| {
       log::error!("could not add this run to the runs of this computer: {e}");
       Vec::new()
   });
   let mut runs = vec![run_id.to_string()];
   if matches!(liveness, session_core::jobs::Liveness::OnlyInstance) {
       runs.extend(earlier);
   }
   ```

   The launch calls it at `src-tauri/src/managers/processing.rs:821`, and `ProcessingManager::own_runs()` gives the list to the commands (`src-tauri/src/managers/processing.rs:961-963`).

4. `in_use_except` takes a list of run IDs, and skips the markers of all of them (`crates/session-save/src/in_use_marker.rs:139-157`).

5. Each check uses the list:
   - The library scan: `ScanOptions::own_runs` (`crates/library/src/scan.rs:48`), read in `remote_state` (`crates/library/src/scan.rs:572`). `Reconciler::with_own_runs` sets it (`crates/library/src/reconcile.rs:196-198`), and the processing manager calls it (`src-tauri/src/managers/processing.rs:1014`).
   - Record and the other R21 actions: `check_record` and `folder_in_use` (`src-tauri/src/commands/session.rs:233-295`).
   - Delete: `in_use_elsewhere` (`src-tauri/src/commands/session.rs:1056-1101`).
   - The empty-note pass gets `earlier_runs: &self.own_runs[1..]` (`src-tauri/src/managers/processing.rs:883`). `check_folder` deletes a marker of an earlier run as an old marker, also when it is fresh (`crates/pipeline/src/empty_note.rs:402-416`).

## Why This Works

- A marker of an earlier run on this computer is this computer's own work. The current run finishes that work, so the marker does not mean that another computer works on the note.
- KTD5 still holds. A fresh marker of a run that is not in this computer's list means InUse.
- The instance lock gate keeps the check correct when two instances run on one computer. Without the lock, another instance can be alive, and its markers show real work.
- `RUNS_KEPT` keeps the table small. A marker is fresh for only 30 minutes after its run stops, so only the runs of the last few launches can have a fresh marker. A marker of a run that is older than the newest 16 counts as work on another computer. That is the safe direction.
- A copy of the app data to a new computer does not break the check. The new computer makes new run IDs. A copied run ID matches only markers that the old computer wrote before the copy.

## Prevention

Tests:

- `the_markers_of_all_excepted_runs_do_not_count` (`crates/session-save/src/in_use_marker.rs:181`).
- `a_fresh_marker_of_an_earlier_run_on_this_computer_does_not_count` (`crates/library/tests/scan.rs:982`).
- `a_fresh_marker_of_an_earlier_run_on_this_computer_refuses_nothing` (`src-tauri/src/commands/session.rs:2108`).
- `a_fresh_marker_of_an_earlier_run_on_this_computer_does_not_keep_the_note` (`crates/pipeline/tests/empty_note.rs:150`).
- `a_run_gets_the_earlier_runs_newest_first_and_the_store_keeps_the_newest` (`crates/session-core/tests/jobs.rs:107`).
- Manual test on a notarized build: quit while a note is processed, then launch again. The note page must not say that the note is in use on another computer.

Rules to keep:

- **A run ID identifies one launch, not one computer.** A check that asks "is this marker mine?" must use the list of this computer's runs, not the current run ID.
- **Do not put a stable computer ID in the marker.** A copy of the app data gives two computers the same ID.
- **Instance lock gate.** Earlier runs count as this computer's runs only with `Liveness::OnlyInstance` (`crates/session-core/src/jobs.rs:319-325`).
- **Safe default when the store fails.** If `add_run` fails, only the current run counts. A marker of an earlier run then counts as work on another computer.
- **Keep `RUNS_KEPT` bound.** The table keeps the newest 16 runs. Do not keep all runs.
- **No bulk removal of earlier-run markers at launch.** A launch pass over all notes would do one file operation for each note and each earlier run, on a synced volume. The markers stop being fresh after 30 minutes. Until then, other computers and audio removal treat the note as in use, which is the safe direction. Only the empty-note pass deletes earlier-run markers, and only in the folder of the empty note that it removes.
- **A new in-use check must take `own_runs`.** When you add an action to R21, give it `ProcessingManager::own_runs()`, not the current run ID.

## Related Issues

- [A computer that received a sync renamed another computer's folder back at launch](receiving-computer-renamed-synced-folder-back.md): the same rule from another side. An automatic step tells this computer's own earlier work from another computer's state with a local record of what this computer did (there `owed_renames`, here `runs`), never with the synced files alone.
- [Parallel worktree PRs that each raise a schema version must renumber at merge](../workflow-issues/parallel-branches-bump-same-schema-version.md): the `runs` migration was renumbered from 0016 to 0017 when `main` added its own 0015.
- PR #33 (twistymaze/longhand) shipped the fix.
