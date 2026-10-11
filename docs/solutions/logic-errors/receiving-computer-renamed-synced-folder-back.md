---
title: A computer that received a sync renamed another computer's folder back at launch
date: 2026-09-26
category: logic-errors
module: Folder rename reconcile at launch (crates/pipeline/src/folder_rename.rs reconcile_saved, crates/session-core owed_renames)
problem_type: logic_error
component: service_layer
symptoms:
  - "After a rename on computer B, computer A started while the sync was in progress and renamed the folder back to the old title"
  - "The later session.md sync then left a folder name that did not match its title, or a conflict copy"
root_cause: data_integrity
resolution_type: code_fix
severity: high
tags: [sync, storage-folder, folder-rename, launch-reconcile, owed-renames, idle-writes, one-library]
related_components: [library, session_core]
---

# A computer that received a sync renamed another computer's folder back at launch

## Problem

At launch, Longhand renamed every saved folder whose name did not match its stored title. In a storage folder that several computers share, this rename can act on a note that another computer changed. A partial sync can make the folder name and `session.md` disagree for a short time. The launch then "corrected" the new name back to the old title.

## Symptoms

- Rename a note on computer B from "Planning" to "Budget". The folder rename syncs to computer A before the new `session.md` does.
- A starts in that gap, for example at login. The library scan reads the old title "Planning" from `session.md`. `reconcile_saved` then renames the folder from "… Budget" back to "… Planning".
- When the new `session.md` arrives, the folder name and the title disagree, or the sync service makes a conflict copy. No content is lost, but the user sees a change that nobody made.

## What Didn't Work

- **Compare the folder name with `session.md`.** Two cases produce the same state on disk, a folder name that does not match the title in `session.md`:
  - The legitimate retry: this computer changed the title and wrote it into `session.md`, but the folder move failed.
  - The partial sync: another computer moved the folder, and its new `session.md` has not arrived yet.

  No rule on the disk state alone tells them apart.
- **Trust the store title.** The launch library scan runs first and refreshes the row from `session.md` (KTD9). So after the scan, the store title always matches the old `session.md`, and it gives no extra information.

## Solution

Record, on this computer, each rename that this computer owes. The launch reconcile renames only those folders.

- Migration `crates/session-core/migrations/0008_owed_renames.sql` adds the table `owed_renames (session_id PRIMARY KEY REFERENCES sessions ON DELETE CASCADE)`. The store methods are `Store::set_rename_owed` and `Store::owed_renames` (`crates/session-core/src/jobs.rs:885`).
- `folder_rename::reconcile` (`crates/pipeline/src/folder_rename.rs:117`) records its outcome:
  - `Later` (the session is busy) or `Kept` (the move failed) sets the record.
  - `Unchanged` or `Moved` clears it.
- The local title writers set the record before the move, so a crash between the title change and the rename leaves the record in place:
  - `rename_session` (`folder_rename.rs:58`);
  - the summarize stage when it proposes a new title (`crates/pipeline/src/summarize.rs:629`).
- `settle` sets the record when a recorded rename intent resolves as `NotMoved` (`folder_rename.rs:279`): the move never happened, so this computer still owes it.
- `reconcile_saved` (`folder_rename.rs:242`) goes through `owed_renames()` in place of every Saved session. It skips a row that is not Saved.

```rust
// Before: every saved folder.
for record in guard.sessions_in(SessionState::Saved.name())? { … }
// After: only the renames this computer owes.
for id in guard.owed_renames()? { … }
```

The regression test is `the_launch_reconcile_does_not_rename_a_folder_whose_new_session_md_has_not_synced` in `crates/pipeline/tests/runner.rs`. Two existing tests guard the retry paths that the record must keep:
- `a_crash_after_the_intent_and_before_the_rename_resolves_at_launch_to_the_old_path`;
- `a_crash_after_done_and_before_a_deferred_rename_ends_renames_at_the_next_launch`.

In the first version of the fix, both failed, until `settle` and the title writers also set the record.

## Why This Works

The disk state is the same in both cases. The difference is where the change came from, and only the computer that made the change knows that. A local record of "this computer owes this rename" carries that fact. A note that arrived through a sync never gets a record, so no automatic step on the receiving computer moves its folder. A local rename that failed keeps its retry at the next launch.

This fix supports a wider rule that the user asked for in this work. **A computer that only receives a sync writes nothing into a note folder.** The folder watcher and the scan every 60 s update only this computer's database. The only file that an idle computer can write is the library marker `.longhand/library` (`crates/library/src/scan.rs:53`). It writes the marker only when the marker is missing.

## Prevention

- Any automatic step that writes into the storage folder must act only on work that this computer started. Examples are a launch step, a watcher callback, a timer, and a future sync of settings through the hidden `.longhand` folder in a storage folder. Record that work locally (a table like `owed_renames` or `rename_intents`). Do not infer it from the shared files, because a partial sync can make them inconsistent for a time.
- For each new automatic writer, write a test with the partial-sync order. Deliver one file of a change (the folder move) without the other (the new `session.md`), run the step, and assert that it changes nothing.
- When an existing retry path depends on "every row" behavior, list every place that can leave the work undone: a busy session, a failed move, a crash after the title change, and an intent that did not move. Then set the record at each of them. The two crash-window tests above caught the missing places.
- `owed_renames` is migration 0008. A parallel branch that also adds a migration must renumber at merge. See [parallel-branches-bump-same-schema-version](../workflow-issues/parallel-branches-bump-same-schema-version.md).

## Related Issues

- Archived PR 14 (One library across computers) shipped this fix.
- [save-state-machine-crash-window-recovery](save-state-machine-crash-window-recovery.md): the same pattern of a local record that survives a crash between a disk step and its record.
- Plan: `docs/plans/2026-09-25-1217-feat-one-library-across-computers-plan.md` (KTD8, KTD9, R9).
