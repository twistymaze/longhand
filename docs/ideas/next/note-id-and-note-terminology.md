---
title: Note id and note terminology
description: "Rename the session id to note id in code, longhand.json and the database, so CodeQL stops flagging logs; make 'note' and 'manual notes' the terms"
area: library-and-sharing
added: 2026-10-23
source: "Brainstorm on 2026-10-23 about repeated CodeQL rust/cleartext-logging alerts on session_id"
related: [rename-session-to-note-in-code]
plan: docs/plans/2026-10-23-1220-refactor-note-id-and-note-terminology-plan.md
---

# Note id and note terminology

## Idea
Rename the session id to "note id" in code names, in the `longhand.json` key and in the database column. Make "note" the term for the whole item and "manual notes" the term for the typed text in CONCEPTS.md, living docs and new code.

## Why
CodeQL treats any name that matches `session.?(id|key)` as sensitive, so each PR that logs the id fails the CodeQL check. The id is a random folder name and is not a secret. About 100 alerts were dismissed by hand before 2026-10-23.
