---
title: A folder in use was taken for a taken name, and a blocked change of case could hide the note
date: 2026-10-16
category: logic-errors
module: Folder renames that never replace (crates/session-save/src/rename.rs, rename_home.rs, the Staged step in lib.rs; crates/pipeline/src/early_home.rs and move_notes.rs)
problem_type: logic_error
component: service_layer
severity: medium
symptoms:
  - On Windows, the first save gave a note the name "<name>-2" when "<name>" was free, or failed with "has no free name for <name> after 100 tries"
  - The Move paused with "the new folder has no free name" while the real cause was a file open in another app
  - A rename that changed only the case of a folder name, with the new and old names both taken, could leave the folder under the hidden name ".longhand-rename-<id>" with no record of it
root_cause: logic_error
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - rename
  - in-use
  - permission-denied
  - windows
  - change-of-case
  - temporary-name
  - step-then-record
---

# A folder in use was taken for a taken name, and a blocked change of case could hide the note

## Problem

Longhand renames a note folder with a rename that never replaces (`rename_noreplace`). When the rename fails because the name is taken, the caller tries the next name (`<name>-2`, `<name>-3`, ...). Two error classes were mixed up:

- `rename::is_taken` counted `PermissionDenied` as a taken name on Windows. But the Windows `rename_noreplace` checks the target first and reports a taken name as `AlreadyExists`. On Windows, `PermissionDenied` comes from a file that another process holds open, for example antivirus that scans the copied audio, or a sync client. The first save, the early write and the Move then tried all 100 names at once, with no wait. The note got a suffix it did not need, or the step failed with a false "no free name" message. The home folder rename already treated this case correctly (`is_in_use`, with tries).
- A change of case moves the folder to a temporary name first (`.longhand-rename-<id>`), because a case-insensitive volume sees the two names as one. When the second move and the move back were both refused as taken, `second_move` passed `Taken` up. `rename_into` read `Taken` as "the folder is at `from`", and recorded the next move from the old name with no temporary name. The folder was at the hidden name, which the library scan skips, and no record named it.

PR #26 fixed both. Code review found a third gap in the first fix: the later records named the temporary folder as `from`, with no `temp`. After a crash, recovery found `from` and read the record as "not moved".

## Solution

- **One rename that waits while a folder is in use.** `rename::rename_new_waiting(fs, from, to)` tries `rename_new` again `IN_USE_RETRIES` times after `IN_USE_DELAY` while `is_in_use(&e)`. Every caller uses it: the Staged step, the early write, the Move and the home folder rename.
- **Only `is_name_taken` gives the next name.** It matches `AlreadyExists` and `DirectoryNotEmpty` only. An in-use error after the tries returns `rename::in_use_error`, which says "is in use by another app, or access was refused". `is_taken` is removed. `is_in_use` also matches `PermissionDenied` on macOS, so a real refusal of access is tried again and then reported with the same text.
- **A move back that fails says where the folder is.** `second_move` returns `Refused::AtTemp(temp)` when the move back is refused as taken, and an error that names `temp` for any other failure. `resolve` returns an error for `AtTemp`, so the record stays.
- **After `AtTemp`, each later record keeps the old name and the temporary name.** `rename_into` records `Move { from: <old>, to: <name>-N, temp: Some(<temp>) }`. `take` sees that `from` is gone and `temp` is there, and does only the second move. After a crash, `resolve` finds the folder at `temp` and finishes the move.

## Why This Works

A taken name and a folder in use need different actions: the first needs another name, the second needs time. Only the error kind tells them apart, and the platform pre-check makes `AlreadyExists` the only taken-name signal on every platform.

A recorded move is the only evidence of where a folder is while it has a hidden name. Each record must name every place the folder can be at that point. A record that names the hidden folder as `from` reads as "the move did not happen" once the hidden folder exists, because `resolve` checks `from` first.

## Prevention

- Decide "try the next name" only with `is_name_taken`. A new rename site uses `rename_new_waiting` and maps `is_in_use` to `in_use_error`.
- When a multi-step rename leaves the folder at an intermediate name, every later record carries that name, and recovery handles "only the intermediate name exists".
- Test both with a test `Fs`: a `rename_new` that gives `PermissionDenied` a number of times (the name stays without a suffix; a permanent refusal says "in use" and tries one name), and a record function that panics after the record of the next name (`resolve` of the last record finds the folder). The tests are in `crates/session-save/tests/save.rs`, `crates/session-save/tests/rename_home.rs`, `crates/pipeline/tests/move_notes.rs` and `crates/pipeline/tests/early_home.rs`.

## Related Issues

- [save-state-machine-crash-window-recovery](save-state-machine-crash-window-recovery.md): the step-then-record rules that the rename records follow.
- [move-dropped-notes-when-old-folder-went-away](move-dropped-notes-when-old-folder-went-away.md): the Move's own placed-copy rules.
