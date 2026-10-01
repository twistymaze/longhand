---
title: Title instructions in summary prompts
description: "Each summary prompt controls the note title; the built-ins start the title with the meeting type, such as \"1:1 Meeting: …\""
area: transcript-and-summary
added: 2026-09-30
source: "a ce-brainstorm session, 2026-09-30"
related: [summary-prompts, summary-prompt-by-note-type]
plan: docs/plans/2026-09-30-1604-feat-title-instructions-plan.md
---

# Title instructions in summary prompts

## Idea
Each summary prompt has a "Title instructions" text beside its summary text. After the final summary call, a second turn of the same conversation asks for the title, so the title sees the whole transcript, the notes and the summary. The built-in title text starts the title with the apparent meeting type, for example "1:1 Meeting: Hiring plan and Q3 budget".

## Why
The title today comes from a separate call with fixed instructions. It sees only the summary and the notes, so it often cannot tell the type of meeting, and no summary prompt can change the title pattern.

## Notes / Current state
- Requirements: [the plan](../../plans/2026-09-30-1604-feat-title-instructions-plan.md).
- Builds on saved summary prompts ([done/summary-prompts.md](summary-prompts.md)).
