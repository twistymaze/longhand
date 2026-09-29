---
title: A Move closed as Finished and forgot notes when the old storage folder went away during a pass
date: 2026-09-29
category: logic-errors
module: Move of notes to a new storage folder (crates/pipeline/src/move_notes.rs check_old and the note steps that drop a note; src-tauri/src/managers/processing.rs forget_outside_roots)
problem_type: logic_error
component: service_layer
symptoms:
  - "Code review found that a Move could close as Finished while notes were still in the old storage folder"
  - "If the old folder went away during a pass (a removed disk, a lost network share, a signed-out sync client), each remaining note looked deleted and the Move dropped it from its journal"
  - "After the Move closed, forget_outside_roots removed the rows, so Longhand did not list notes that were still on the old disk, and Retry could not get them back"
root_cause: missing_validation
resolution_type: code_fix
severity: high
framework_version: rust 1.94.0
tags: [move, storage-folder, old-folder, not-reachable, library-marker, forget-outside-roots, one-library, removable-volume]
related_components: [library, background_job]
---

# A Move closed as Finished and forgot notes when the old storage folder went away during a pass

## Problem

The Move copies each saved note from the old storage folder to a new storage folder, one note at a time, and records each step in the Move journal. Several steps read "the source folder of the note is gone" as "the user moved or deleted the note". These steps dropped the note from the Move journal (`Taken::Dropped`).

`prepare()` checked only once per pass that the old folder was reachable. If the old folder went away during a pass, each remaining note looked gone. The pass then dropped every remaining note and closed the Move as Finished. After Finished, the app removed the old folder from the library roots and forgot every row outside the roots. Longhand then stopped listing notes that were still on the old disk, and Retry could not get them back.

Code review of PR twistymaze/longhand#24 found this defect before it occurred in production.

## Symptoms

- A removed disk, a lost network share or a signed-out sync client during a Move pass. The pass ends as Finished, not Paused.
- The log shows `move: the folder of {id} is gone, so it leaves the move` for each note after the failure (`crates/pipeline/src/move_notes.rs:541`).
- The Notes list no longer shows the notes that did not move. Their folders are still in the old folder when the disk comes back.
- Retry does nothing, because no Move journal is open.

The chain after Finished:

- `pass_inner` closes the Move when no note is left in the Move journal and no save still goes into the old folder (`crates/pipeline/src/move_notes.rs:491-500`).
- On `PassEnd::Finished`, the processing manager calls `roots_changed()` and then `forget_outside_roots()` (`src-tauri/src/managers/processing.rs:949-955`).
- `library::reconcile::forget_outside` forgets each row whose home folder is in no library root (`crates/library/src/reconcile.rs:522-547`). This includes the dropped notes, because their home paths still name the old folder.

## What Didn't Work

A reachability check in `prepare()` only. Before the fix, `prepare()` paused the Move when the old folder was not a folder or had no library marker. The check ran once, before the loop over notes. The per-note steps did not check again. A folder that went away after `prepare()` gave the same result as a set of notes that the user deleted one by one.

Before (in `prepare()`, the only check):

```rust
let open = !lock(self.store).moving_notes()?.is_empty();
// A signed-out sync client can remove the files and the marker: the
// sources then look gone, and are not.
if open && (!old.is_dir() || !library::scan::marker(&old).exists()) {
    return Err(MoveError::Paused(MovePause::NotReachable {
        folder: old.to_string_lossy().into_owned(),
    }));
}
```

Before (the Placed step, one of the drop paths with no check):

```rust
return Ok(match source {
    Some(source) if exists(source) => Taken::Next(MoveStep::Pending),
    _ => Taken::Dropped,
});
```

## Solution

The fix adds one helper, `check_old`, and calls it before each drop that follows a missing source in the old folder (`crates/pipeline/src/move_notes.rs:204-215`):

```rust
/// Pause the Move when the old folder is not reachable. A signed-out sync
/// client, a removed disk or a lost share can remove the files and the
/// marker: the sources then look gone, and are not. Each step checks this
/// before it drops a note whose source is gone.
fn check_old(old: &Path) -> Result<(), MoveError> {
    if old.is_dir() && library::scan::marker(old).exists() {
        return Ok(());
    }
    Err(MoveError::Paused(MovePause::NotReachable {
        folder: old.to_string_lossy().into_owned(),
    }))
}
```

The helper uses the same library marker as the library scan (`crates/library/src/scan.rs:30-33`).

After, `prepare()` uses the helper (`crates/pipeline/src/move_notes.rs:355-357`):

```rust
if !lock(self.store).moving_notes()?.is_empty() {
    check_old(&old)?;
}
```

After, each drop path calls the helper first:

- Pending step, source gone and no placed copy in the new folder (`crates/pipeline/src/move_notes.rs:711-719`):

  ```rust
  if !exists(&source) {
      if let Some(path) = find_placed(new, &name, id) {
          return Ok(Taken::Next(MoveStep::Placed { source: None, path }));
      }
      check_old(&journal.old_folder)?;
      return Ok(Taken::Dropped);
  }
  ```

- Copied step, staging and source both gone (`crates/pipeline/src/move_notes.rs:610-615`):

  ```rust
  if exists(source) {
      return Ok(Taken::Next(MoveStep::Pending));
  }
  check_old(old)?;
  return Ok(Taken::Dropped);
  ```

- Placed step, placed copy and source both gone (`crates/pipeline/src/move_notes.rs:644-652`):

  ```rust
  return Ok(match source {
      Some(source) if exists(source) => Taken::Next(MoveStep::Pending),
      Some(_) => {
          check_old(old)?;
          Taken::Dropped
      }
      None => Taken::Dropped,
  });
  ```

The Placed step with `source: None` still drops with no check. `source: None` means a rename on one volume moved the folder, so no source in the old folder exists to check.

The other drops in the Move do not read a missing source file. They drop a note because its store row is missing, or because its home folder is not in the old folder. They need no check.

`pass()` changes `MoveError::Paused` into `PassEnd::Paused`, and records the pause in the Move journal (`crates/pipeline/src/move_notes.rs:456-465`). The journal stays open, and Retry can continue the Move.

## Why This Works

A missing file and a missing parent folder give the same result from `exists()`. Only a check of the parent folder can tell the two cases apart. `check_old` tests that the old folder is a folder and has its library marker. A removed disk, a lost share and a signed-out sync client all fail this test. A note that the user deleted from a reachable old folder passes the test and still drops.

The check runs at the time of each drop, not once per pass. A folder that goes away between two notes therefore pauses the Move at the next drop. The note steps that completed stay recorded. The remaining notes stay Pending in the open Move journal. Because the Move does not close, the app does not call `forget_outside_roots()`, and no row is forgotten.

## Prevention

**Rule.** In any step that reads "the source is missing" as a user action (moved or deleted), first prove that the folder that contains the source is reachable. A folder that went away and a file that went away look the same. Do this check at the time of the decision, not once at the start of a loop.

The library scan follows the same rule:

- `scan_root` marks a root `Unavailable` when the root is missing, is not a folder, cannot be listed, or has no library marker and no note folders while the store has rows in it (`crates/library/src/scan.rs:290-340`).
- The reconciler forgets a row only when the root of its home folder is available (`crates/library/src/reconcile.rs:359-362`, `crates/library/src/scan.rs:197-202`). A row in an unavailable root stays.
- `scan_folders` returns `None` for a root that is not a folder with its marker, so a targeted scan says nothing about such a root (`crates/library/src/scan.rs:218-231`).

When you add a new step that drops, forgets or deletes because a path is missing, use the same check (`check_old` in the Move, `is_available` in the reconciler). Also look at each later step that acts on the result, for example Finished followed by `forget_outside_roots()`.

**Test pattern.** Make the folder go away in the middle of a pass, not before it. The test `an_old_folder_that_goes_away_during_a_pass_pauses_the_move` (`crates/pipeline/tests/move_notes.rs:566-607`) does this:

```rust
f.save_note("a", "Budget");
f.save_note("b", "Planning");
f.open();
let away = f.working.join("away");
let m = mover(&f, &fs);
let mut done = Vec::new();
let end = m
    .pass(&mut |id, _| {
        if done.is_empty() {
            std::fs::rename(&f.old, &away).unwrap();
        }
        done.push(id.to_owned());
    })
    .unwrap();
assert_eq!(
    end,
    PassEnd::Paused(MovePause::NotReachable {
        folder: f.old.to_string_lossy().into_owned(),
    }),
);
assert_eq!(done.len(), 1);
let left = if done[0] == "a" { "b" } else { "a" };
assert!(f.store.lock().unwrap().move_journal().unwrap().is_some());
assert_eq!(f.step(left), Some(MoveStep::Pending));
// The folder comes back, and Retry finishes the Move.
std::fs::rename(&away, &f.old).unwrap();
move_notes::retry(&f.store).unwrap();
finish(&m);
assert_moved(&f, "a");
assert_moved(&f, "b");
```

The key parts of the pattern:

- Use the progress callback to remove the folder after the first note. A test that removes the folder before the pass tests only the check in `prepare()`.
- Assert the pause, not only "no error": the end is `Paused(NotReachable)`.
- Assert that no data went: the Move journal is open and the remaining note is still Pending.
- Restore the folder, call Retry, and assert that every note moves.
- Run the test on one volume and on two volumes (`for cross in [false, true]`), because each case takes different steps.

## Related Issues

- [receiving-computer-renamed-synced-folder-back](receiving-computer-renamed-synced-folder-back.md): the same pattern. The state on disk alone cannot tell two causes apart, so an automatic step must get more evidence before it acts.
- [save-state-machine-crash-window-recovery](save-state-machine-crash-window-recovery.md): the same step-then-record pattern that the Move journal uses, and a checklist of the window of each step.
- [shared-settings-in-a-synced-folder-rules](../integration-issues/shared-settings-in-a-synced-folder-rules.md): rule 4 makes the same distinction for settings. A folder that is not available (`FolderMissing`) is not a missing file (`Missing`).
- PR twistymaze/longhand#24 added the Move and this fix.
