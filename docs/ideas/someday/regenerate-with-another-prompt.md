---
title: Regenerate a summary with another prompt
description: "A 'Regenerate With' submenu on a note summarizes it again with a saved prompt that is not the active one; the app-wide setting does not change"
area: transcript-and-summary
added: 2026-09-29
source: "a conversation on 2026-09-29"
related: [summary-prompts, summary-prompt-by-note-type]
---

# Regenerate a summary with another prompt

## Idea
A completed note gets a "Regenerate With" submenu beside Regenerate Summary. The submenu lists the saved summary prompts. When the user selects a prompt, the app summarizes the note again with that prompt. The active prompt in Settings does not change.

## Why
Sometimes one note needs a different summary shape. Now the user must change the app-wide prompt in Settings, regenerate, and then change the prompt back.

## Notes / Current state
- Saved summary prompts shipped ([done/summary-prompts.md](../done/summary-prompts.md)): Settings holds named prompts and one active prompt for the whole app.
- The note menu has Regenerate Summary (`src/session/SessionPage.tsx`), which calls `regenerateSummary(id)` with the active prompt. This idea adds a prompt argument.
- Open question: does the note remember the prompt it used, so that a later plain Regenerate uses the same prompt? This overlaps with [summary-prompt-by-note-type](../next/summary-prompt-by-note-type.md), where the type of a note chooses its prompt.
