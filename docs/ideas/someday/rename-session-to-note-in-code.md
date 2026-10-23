---
title: Rename session to note in existing code
description: "Rename the remaining session_* code names, crates, SQL tables and session.md to note, and existing 'notes' names to manual notes"
area: library-and-sharing
added: 2026-10-23
source: "Brainstorm on 2026-10-23 (docs/plans/2026-10-23-1220-refactor-note-id-and-note-terminology-plan.md, Scope Boundaries)"
related: [note-id-and-note-terminology]
---

# Rename session to note in existing code

## Idea
After the note id rename, rename the other code names that say "session" (for example `SessionState`, `session_dir`, the `session-core` and `session-save` crates and the SQL tables) to "note". Rename existing code that says "notes" for the typed text to "manual notes". The `session.md` file name can stay if a rename costs too much compatibility work.

## Why
CONCEPTS.md makes "note" the term for the whole item, but most existing code still says "session". Two terms for one concept make the code harder to read. A full rename touches about 410 files, so it is separate from the note id change.
