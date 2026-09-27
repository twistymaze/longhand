---
title: An open note page kept the old summary after a sync, because the watcher reported only changed rows
date: 2026-09-26
category: logic-errors
module: Library watcher change report (crates/library/src/reconcile.rs Reconciler::run)
problem_type: logic_error
component: service_layer
symptoms:
  - "Computer A ran Regenerate Summary. The new summary.md synced to computer B, but B's open note page kept the old summary"
  - "Leaving the note and opening it again showed the new summary"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags: [sync, library-watcher, library-changed, summary, note-page, test-trap, one-library]
related_components: [frontend, library]
---

# An open note page kept the old summary after a sync, because the watcher reported only changed rows

## Problem

The library watcher sends a `LibraryChanged` event with the ids of the notes that changed. An open note page reads its documents again when its note is in that list. The reconciler built the list only from the notes whose store row changed. A new `summary.md` from another computer does not change the row, so the note was not in the list, and the open page kept the old summary.

## Symptoms

- On computer A, run Regenerate Summary. On computer B, with the note open, `summary.md` and `session.md` update on disk, but the Summary tab does not change.
- The page shows the new summary when you leave the note and open it again.
- A rename from another computer did update the open page. The title and the home path are in the row, so that change was in the list.

## What Didn't Work

- **The first regression test passed without the fix.** It wrote only `summary.md` and left the fingerprint in `session.md` as it was. The scan then treated the summary as "waiting for the sync". That changed the note's library status, and a status change is also in the list. So the test showed a partial delivery, not the real case. The test failed as it should only after it wrote `summary.md` and the matching `summary_fingerprint` in `session.md` together, as one sync batch. Always confirm that a new regression test fails without the fix (red first).

## Solution

`Reconciler::run` (`crates/library/src/reconcile.rs:151`) now adds a list of touched notes to `changes.changed` (`reconcile.rs:190`). For a targeted scan (`Target::Folders`), these are the notes whose real folder is one of the folders that the watcher saw change. A full scan (the scan every 60 s, and the launch scan) adds none. Without this rule, each full scan would reload every open page.

```rust
let touched = match target {
    Target::Folders { folders, .. } => result
        .notes
        .iter()
        .filter(|note| folders.contains(&note.path))
        .map(|note| note.doc.session_id.clone())
        .collect(),
    Target::Full(_) => Vec::new(),
};
```

The frontend needed no change:
- `useSessions` (`src/session/useSessions.ts:80`) raises each changed note's version.
- `SessionPage` reads the document again when the version changes (`src/session/SessionPage.tsx:254`).

The regression test is `a_targeted_scan_reports_a_note_whose_summary_alone_changed` in `crates/library/tests/two_computers.rs`. The test `a_targeted_scan_adopts_a_new_folder_and_starts_no_delete_for_the_others` now expects that a second targeted scan of the same folder reports the note again.

## Why This Works

A targeted scan runs only because the watcher saw a file event in those folders. So "a file in this note's folder changed" is a fact, even when nothing in the row changed. That fact is what an open page needs. The row is only a copy of part of `session.md`. It does not hold the transcript, the summary or the notes text. So a row comparison cannot see a change to those files.

## Prevention

- Do not use a store-row comparison as the only change signal for data that lives in files. For each view, ask what it shows that the row does not hold. The note page shows `summary.md`, `transcript.md` and `notes.md`.
- In a test of a synced change, deliver the whole change: every file that the other computer writes, with the fingerprints in `session.md`. A partial delivery tests the "waiting for the sync" path instead.
- Extra reports are cheap, but lost ones are not. A local write also causes a watcher event and a report. The note page reads the documents again, and its notes editor keeps a local edit that has not been saved.

## Related Issues

- PR #14 (One library across computers) shipped this fix after a manual two-computer test found the problem.
- [receiving-computer-renamed-synced-folder-back](receiving-computer-renamed-synced-folder-back.md): another problem that a partial sync caused in the same feature.
