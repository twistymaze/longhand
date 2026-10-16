---
title: A change of storage folder with Leave them made the library forget notes that had no first save
date: 2026-10-16
category: logic-errors
module: Early home folders (crates/pipeline/src/early_home.rs, crates/pipeline/src/save.rs, src-tauri/src/commands/settings.rs)
problem_type: logic_error
component: service_layer
severity: high
symptoms:
  - A note that was recording or processing when the user changed the storage folder with Leave them was not in the list after its save
  - The note's files were in the old storage folder
root_cause: logic_error
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - early-home-folder
  - storage-folder
  - crash-recovery
  - idempotent
  - race
---

# A change of storage folder with Leave them made the library forget notes that had no first save

## Problem

A note gets its early home folder in the storage folder when it starts. Its first save publishes into that folder. When the user changed the storage folder with **Leave them**, a note with no first save kept its early home folder in the old storage folder. Its first save then went there, and the library, which reads only the current storage folder, forgot the note.

A first fix that rehomed the notes once, at the settings change, was not sufficient. The review found that a crash after the settings write, a crash between the rehome records, a file that could not be read at the change, and an early write that read the old storage folder before the change each left a note in the old folder.

## Solution

1. **One rule that any trigger applies again.** `rehome_outside(store, current)` gives each note that has no first save, and whose home folder is not under the current storage folder, a new early write in the current storage folder. It does nothing while a Move is open. It runs at the change (through `retry_awaiting`), at launch before the early writes resume, on each retry, and after each early write is done. Because the rule reads only the state, a step that a crash stopped is done at the next trigger.
2. **A backstop in the first save.** `first_save_outside` in `save.rs` never publishes a first save into a home folder outside the storage folder. It gives back a claim that has no publication, starts the rehome, and returns `NoDestination`, so the save waits for the new early home folder. When the old home folder cannot be read, the save also waits.
3. **A list of old home folders per note.** `OLD_HOMES` keeps each old home folder with its file fingerprints. An old folder is removed only after the new home is recorded and only when its files match the fingerprints. An entry stays while its storage folder is not available, while a fresh in-use marker of another run is in it, or when the removal failed. A second change keeps the entries of the first.
4. **Gated writers.** Title, tag and note type writes and folder renames check `rehoming` and do not write into an old home folder. The title is written into the new home folder after the rehome.
5. **Refuse only work that runs now.** `check_leave` refuses Leave them only while an early write whose folder is available runs, or while the runner runs a first save (`Runner::runs`). A first save that stopped does not block the change.

Known limit: a first save whose publication began before the change finishes in the old folder.

## Prevention

- Do not do a state change that moves data as one step at a settings change. Write one rule that reads the state, and apply it at every trigger that can see the old state: the change, launch, retries and the end of each writer.
- Add a backstop at the point of the write that the rule protects, so a race with the rule cannot publish into the wrong place.
- Refuse a user action only for work that runs now. A refusal for work that stopped can block the user for good.
