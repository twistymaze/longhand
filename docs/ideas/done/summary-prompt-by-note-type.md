---
title: A summary prompt for each type of note
description: "The type of a note chooses its summary prompt: a recap for Pathfinder, decisions and follow-ups for work"
area: transcript-and-summary
added: 2026-09-23
source: "the retired FUTURE-IDEAS.md; changed from 'summary kinds chosen by folder' in the idea review of 2026-09-28"
related: [summary-prompts, note-folders, prompt-usage-in-summary-prompts]
plan: docs/plans/2026-10-09-0715-feat-summary-prompt-by-note-type-plan.md
---

# A summary prompt for each type of note

## Idea
Each note has a type, and the type chooses one of the saved summary prompts. For example, a Pathfinder note gets a recap of the session, and a work note gets decisions and follow-ups.

The note type of [Note types and tags](note-folders.md) gives a note its type. This idea stays separate from that idea so that each idea stays small.

## Why
One summary shape does not fit every kind of note. The walking skeleton used one shape because that got to a usable app faster.

## Notes / Current state
- Saved summary prompts shipped ([done/summary-prompts.md](summary-prompts.md)): Settings holds a list of named prompts that control the output, with one active prompt for the whole app. This idea adds the choice of a prompt for each note, from its type.
- Open questions: what happens when a note has no type (the app-wide prompt); and whether the user can change the type after the summary exists (Regenerate with the new prompt).
- Brainstormed on 2026-10-09. The plan answers both open questions: General and each type set to "Default" use the default prompt, and a type change after a summary offers a regeneration. The plan also redesigns the Types and Tags pane: plain-text names, a prompt popup on each type, and a ⋯ menu for Rename and Delete.
- Shipped in archived PR 50.
