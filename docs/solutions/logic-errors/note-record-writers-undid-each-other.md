---
title: Writers of longhand.json on one computer undid each other's records
date: 2026-10-16
category: logic-errors
module: Note record writers (crates/session-save/src/publication.rs, crates/pipeline/src/save.rs, crates/pipeline/src/early_home.rs, crates/pipeline/src/type_move.rs, crates/pipeline/src/folder_rename.rs)
problem_type: logic_error
component: service_layer
severity: high
symptoms:
  - A codebase sweep found that a title or type set during Save, Resume, Transcribe Again or Regenerate Summary could be lost, or could put back the record of the earlier result
  - The flag update of the early write and a title write could each undo the other
  - A notes edit during the first write of a note folder could be replaced on disk by the older notes
root_cause: concurrency
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - store-lock
  - read-modify-write
  - longhand-json
  - publication
  - timed-read
  - lock-order
---

# Writers of longhand.json on one computer undid each other's records

## Problem

Several writers change `longhand.json` with a read-modify-write: they read the records, change one field, and write all the records back. The title writer and the type move held the store lock for that. The publication of a result, the accept of a hand-edited transcript in Regenerate Summary and the flag update of the early write did not. The publication read the records again "just before the write", but a writer on another thread could still write between that read and the write. The later write then put back the field that the earlier write changed. The notes sync of the early write had the same gap for `notes.md`. PR #25 fixed it.

## Solution

- **One lock for each read-modify-write of `longhand.json`: the store lock.** The doc comment of `note_files::write_record` states the rule.
- **The publication holds the lock only for its last step.** `Publication::publish(home, local, hold)` and `publish_with` take a function that returns a guard. The publication calls it just before the last read of the records and keeps the guard until `longhand.json` is written. The result files are copied before, without the lock. `session-save` does not know the store, so the caller gives the guard: `|| lock(store)`, or `|| ()` when the caller holds the lock already (`recover_summary_publication`).
- **Writers that run under the lock take `&Store`** (`write_flag`, `sync_notes`), so the type shows that the caller holds the lock. `sync_notes` needs no retry loop any more, because no notes save can run between its read and its write.
- **Reads under the lock have a time limit.** `READ_TIMEOUT`, `Read` and `read_timed` moved to `session_core::note_folder` (`library::scan` re-exports them), and `note_files::new_layout_record` uses `NoteFiles::read_timed`.
- **A file that does not download fails before the lock.** The early write reads `session.md`, `longhand.json` and `notes.md` once with the time limit before it takes the store lock. Without this, a cloud placeholder held the lock for up to 20 seconds, and the job retried every 10 seconds.

## Why a re-read is not enough

A re-read just before the write only makes the window smaller. A thread can still be scheduled between the re-read and the write, and a `write_atomic` with `fsync` takes tens of milliseconds on macOS. Only a lock that every writer holds from its read to its write makes the two steps one step for the other writers.

## Prevention

- A new writer of `longhand.json` (or of another file that two writers change with a read-modify-write) holds the store lock from the read to the write. Give the function `&Store`, not `&SharedStore`.
- Keep the lock order: the capture state lock, then the store lock; the note guard, then the store lock. Do not call an environment closure that can take the capture lock while the store lock is held ([runner-store-lock-before-suspended-check-deadlocks.md](runner-store-lock-before-suspended-check-deadlocks.md)).
- Under the store lock, read files only with a time limit. Before the lock, read the same files once with the time limit, so a file that cannot be read fails while the lock is free.
- A test of a lock rule needs a point between the read and the write. The publication has one (its `hold` function and the `after_write` hook, with no `session.md` in the folder); the flag write and the notes sync do not, so their tests check only the end state.
- The store lock covers writers on this computer only. Writes from other computers follow [shared-settings-in-a-synced-folder-rules.md](../integration-issues/shared-settings-in-a-synced-folder-rules.md).

## Related Issues

- [publication-checked-for-another-result-only-at-start-and-end.md](publication-checked-for-another-result-only-at-start-and-end.md): the check of another computer's result next to each write.
- [app-owned-note-record-file.md](../integration-issues/app-owned-note-record-file.md): which files Longhand owns and the write order.
