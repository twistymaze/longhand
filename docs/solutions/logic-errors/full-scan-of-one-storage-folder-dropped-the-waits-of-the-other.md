---
title: A full scan of one storage folder dropped the waits of the other, and the fix lost a notice event
date: 2026-10-15
category: logic-errors
module: Library reconcile and status (crates/library/src/reconcile.rs, crates/library/src/status.rs, src-tauri/src/managers/library.rs)
problem_type: logic_error
component: service_layer
severity: medium
symptoms:
  - During a Move, the "waiting for sync" and "storage unavailable" status of notes in one storage folder came and went
  - Hand-edit and title waits started again, so a hand edit could stay unreported for the whole Move
  - After the first fix, Home kept a problem notice for a storage folder that was removed from the library
root_cause: logic_error
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - reconcile
  - coverage
  - waits
  - notices-revision
  - move
  - library-watcher
---

# A full scan of one storage folder dropped the waits of the other, and the fix lost a notice event

## Problem

During a Move, the library has two roots: the old and the new storage folder. The watcher scans one root at a time (`Target::Full([one root])`) after a top-level event in it. `Coverage::keeps_wait` returned `false` for every folder under `Coverage::Full`, also for a folder in the root that the scan did not list. So each scan of one root dropped the waits, the status, the other-version timers and the problems of the notes in the other root. PR #24 fixed it.

## Solution

- `Coverage::keeps_wait` now returns `!self.read(scan, folder)` for both coverages: a scan keeps the wait of each folder that it did not read. `Coverage::read` already checks the folder against the roots of the scan.
- A removed root is never scanned again, so its waits must go at a different place. `Reconciler::retain_roots` drops the mismatch and title waits outside the new roots, and `LibraryStatus::retain_roots` drops `notes`, `seen` (also the other versions of a kept note that are in a removed root), `problems`, `old_layout` and `unavailable`. `Worker::set_roots` calls it before its full scan.

## The second defect: a counter bumped before the baseline

`LibraryStatus::retain_roots` increased `notices_revision` when it dropped a notice. `run_at` reports `changes.notices` by comparing the revision before and after its own scan. The retain ran before `run_at` took its baseline, so the full scan saw no change and emitted nothing. Home reads the notices again only at a library change event, so the old notice stayed. Two reviewers (Claude and Codex) found this independently. The fix: `Reconciler` keeps a `notices_dropped` flag, and the next `run_at` adds it to `changes.notices` once.

## Prevention

- A "full" scan is full only for its own roots. A rule that drops state for "everything the scan did not find" must check that the scan could have found it.
- When a fix moves cleanup out of the normal pass into a separate call, check how the normal pass reports changes. A change made outside the pass that compares before and after is invisible to that comparison.
- Test the event, not only the state: the worker test checks that a Destination change emits one library change, not only that the status is empty.

## Related Issues

- [move-dropped-notes-when-old-folder-went-away.md](move-dropped-notes-when-old-folder-went-away.md): another case where the library acted on a storage folder that a scan did not read.
