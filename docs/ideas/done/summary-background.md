---
title: Background for summaries
description: "A free-form Markdown Background in Settings (names, work, terms, common misspellings) that every summary and title call includes; prompt boxes get the Markdown editor too"
area: transcript-and-summary
added: 2026-10-05
source: "the user, in the brainstorm of 2026-10-05, as a smaller step before agentic-summary"
related: [agentic-summary, summary-prompts, title-instructions]
plan: docs/plans/2026-10-05-1222-feat-summary-background-plan.md
---

# Background for summaries

## Idea
A Background section on the Summaries page of Settings, above the summary prompts. The user writes free-form Markdown about themselves, their work, the names and terms that often occur, and words the transcript often gets wrong. Every summary and title call includes it in the system prompt. It uses the My Notes editor, and the summary prompt and title instruction boxes change to the same editor.

## Why
The transcript often spells names and terms wrong, and the summary repeats the errors. The model knows nothing about the user. The prompt text is per prompt, and built-in prompts cannot be changed.

## Notes / Current state
- Shipped in [PR #38](https://github.com/twistymaze/longhand/pull/38).
- Later, the [summary agent](../someday/agentic-summary.md) can read this text as its "context about the user".
- The Background counts as evidence for names in the summary (the naming rule changes).
