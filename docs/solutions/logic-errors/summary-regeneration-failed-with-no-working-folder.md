---
title: Summary regeneration failed on a computer that had not recorded
date: 2026-10-03
category: logic-errors
module: Summary regeneration (crates/pipeline/src/save.rs summarize_again, src-tauri/src/managers/capture.rs working root)
problem_type: logic_error
component: service_layer
symptoms:
  - "Regenerate Summary failed with: No such file or directory (os error 2) at path \".../net.ewal.longhand/sessions/.longhand-summary-XXXXXX\""
  - "The failure occurred only on a computer that had not recorded a note and used a storage folder that another computer synced to it"
root_cause: missing_workflow_step
resolution_type: code_fix
severity: medium
tags: [working-location, two-computers, sync, summary-regeneration, tempfile, first-launch]
related_components: [tauri_backend, testing_framework]
---

# Summary regeneration failed on a computer that had not recorded

## Problem

On a second computer that only reads notes from a synced storage folder, Regenerate Summary failed. The regeneration stages its result in the Working location, and that folder did not exist, because only a recording makes it. Fixed in PR #35.

## Symptoms

- The error was `No such file or directory (os error 2) at path ".../Library/Application Support/net.ewal.longhand/sessions/.longhand-summary-eJq9FE"`.
- The note's transcript and summary were in the storage folder (a Resilio Sync folder). The computer had never recorded, so it had no `sessions/` folder.
- The `at path ...` suffix comes from the `tempfile` crate: its error adds the path that it tried to make. The path names the folder that the call tried to create, not a file that the app expected to find. This made the error look like missing note data.

## What Didn't Work

- The user expected that a regeneration needs nothing in the app folder. That is not correct. The local staging is deliberate: the regeneration writes the new `summary.md` and the publication record into a local temporary folder (`summarize_again` in `crates/pipeline/src/save.rs`). After a crash during the write into the storage folder, the next launch uses that record to complete the write. Do not move this staging into the storage folder to "fix" this kind of error.

## Solution

Make the Working location before the temporary folder is made in it (`crates/pipeline/src/save.rs:1296-1300`):

```rust
// A computer that has not recorded has no working folder yet.
std::fs::create_dir_all(working_root)?;
let local = tempfile::Builder::new()
    .prefix(".longhand-summary-")
    .tempdir_in(working_root)?
    .keep();
```

Regression test: `regenerate_summary_on_a_computer_with_no_working_folder_makes_the_folder` (`crates/pipeline/tests/runner.rs:4992`). It saves a note, deletes the rig's working root, and regenerates the summary. Before the fix, it failed with the reported error.

## Why This Works

- `CaptureManager::new` makes only the app data folder (`src-tauri/src/managers/capture.rs:1182`). It sets the Working location to `<app data>/sessions` but does not make it (`capture.rs:1186`).
- The first `sessions/` folder appears as a side effect of a recording: the recorder calls `create_dir_all` on `<working root>/<session id>` (`crates/recorder/src/chunks.rs:126`).
- `tempfile::Builder::tempdir_in` makes one folder. It does not make missing parents, so it failed with ENOENT when `sessions/` did not exist.
- `create_dir_all` makes the Working location when it is missing and does nothing when it exists.

## Prevention

- Do not assume that the Working location exists. A computer that only syncs a storage folder can use every note action before it records anything. Before a write into the Working location, make the folder with `create_dir_all`. The recording status already reads free space from the parent folder for the same reason (`src-tauri/src/managers/capture.rs:1939`).
- In tests, the runner rig's working root is a `tempfile::tempdir()` that always exists. For a new action that writes into the Working location, add a test that deletes `r.root` first, as the regression test above does.

## Related Issues

- PR #35.
- [Publication checked for another result only at start and end](publication-checked-for-another-result-only-at-start-and-end.md) describes the publication record that the local staging holds.
