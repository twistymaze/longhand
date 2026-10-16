---
title: A Record or Resume could start while a delete removed the folders of the note
date: 2026-10-15
category: logic-errors
module: Delete claim and Record/Resume (crates/session-core/src/jobs.rs, src-tauri/src/managers/capture.rs)
problem_type: logic_error
component: service_layer
severity: high
symptoms:
  - A codebase sweep found that delete_session checks the state, then removes the folders without the store lock
  - A Resume or Record of the same note in that time started a recording in the working folder that the delete then removed; the delete then refused, and the recorded audio was gone
root_cause: concurrency
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - race-condition
  - claims
  - delete
  - record
  - resume
  - store-lock
---

# A Record or Resume could start while a delete removed the folders of the note

## Problem

`session_core::jobs::delete_session` takes a delete claim, reads the note state under the store lock, releases the lock, and moves the folders to the Trash on a blocking thread, which can be slow on a network volume. The claim excluded only a summary regeneration and the Move. Record and Resume did not read the claims. So a Resume in that time took the note to Capturing, and the recorder wrote a new segment into the working folder that the delete removed next. PR #24 fixed it.

## Solution

- `Claims::is_deleting(id)` tells whether a delete holds its claim.
- The store-side functions that take a note to Capturing check it first, under the store lock: `record_in_store` (Record) and the new `resume_in_store` (Resume). Both refuse with "This note is being deleted."
- The order makes the check complete. The delete takes its claim before it reads the state under the store lock. If the Record check runs first, the delete then reads Capturing and refuses. If the claim comes first, the Record check sees it.
- `record_session` and `resume_session` also check at their start, so a Resume does not read or repair the chunk folder that the delete removes. The check under the lock stays: it settles a delete that starts after the first check.

## Prevention

- When a destructive step releases the lock before it acts, list every action that can start in that gap, not only the actions the claim already names. Each one must read the claim under the same lock that the destructive step used for its check.
- Put the check in the store-side function that changes the state, not only in the command, so that a test of that function runs the code the app calls.

## Related Issues

- [resume-during-processing-deleted-live-text-proofs.md](resume-during-processing-deleted-live-text-proofs.md): a Resume that overlapped a destructive step.
- [runner-store-lock-before-suspended-check-deadlocks.md](runner-store-lock-before-suspended-check-deadlocks.md): order rules between a claim and the store lock.
