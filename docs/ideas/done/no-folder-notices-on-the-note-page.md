---
title: No folder notices on the note page
description: "A type change or rename shows no notice when the folder moves later; only errors show"
area: interface
added: 2026-10-09
source: "brainstorm in conversation, 2026-10-09: a new note with its type changed during the recording showed a folder notice"
related: [failed-folder-move-needs-attention]
plan: docs/plans/2026-10-09-1404-fix-folder-notices-plan.md
---

# No folder notices on the note page

## Idea

A type change or a rename on the note page shows a notice only when it fails with an error. It does not show a notice when the folder moves or is renamed later, or when a move must try again.

## Why

The user started a new note without a type and changed the type at once. The page showed "The type changed. The folder moves when the note is free." That is normal behaviour, and the user has nothing to do. The message rule in `CLAUDE.md` (UI design) now records the general rule.

## Notes / Current state

The plan also removes `type_move_waits` from the session entry. A move that keeps failing is a separate idea: [Failed folder move needs attention](../someday/failed-folder-move-needs-attention.md).

## Outcome

Shipped in PR #49.
