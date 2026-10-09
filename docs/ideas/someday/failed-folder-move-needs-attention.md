---
title: Failed folder move needs attention
description: "Tell the user when a folder move or rename still fails after many tries, so they can fix the cause"
area: library-and-sharing
added: 2026-10-09
source: "docs/plans/2026-10-09-1404-fix-folder-notices-plan.md (deferred)"
related: [no-folder-notices-on-the-note-page]
---

# Failed folder move needs attention

## Idea

When a folder move for a type change, or a folder rename, still fails after many tries, show it in one place, for example as a row status on Home. Give the reason, so that the user can fix the cause.

## Why

The note page no longer reports a move or rename that waits or must try again, because the app corrects it without help. The message rule in `CLAUDE.md` (UI design) says to report a problem when the app's tries to correct it do not succeed. Today nothing reports a move that keeps failing.

## Notes / Current state

The pipeline records an owed move or rename (`crates/pipeline/src/folder_rename.rs`, `crates/pipeline/src/type_move.rs`). It does not count the tries or record their age. A cause that lasts can be a file that another app keeps open, or a folder without write permission.
