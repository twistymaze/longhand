---
title: Suggested tags from the summary
description: "The summary model gets the tag list and the note type list and suggests tags (and a type) for the note"
area: transcript-and-summary
added: 2026-10-07
source: "the user, in the note types and tags brainstorm of 2026-10-07"
related: [note-folders, summary-prompt-by-note-type]
---

# Suggested tags from the summary

## Idea
The summary model gets the existing tag list, and possibly the note type list, with the transcript. Its output includes suggested tags for the note. The user accepts or ignores each suggestion.

## Why
Tagging every note by hand is work that the user can forget. The model already reads the whole session to write the summary.

## Notes / Current state
- Depends on [Note types and tags](../next/note-folders.md) (plan: `docs/plans/2026-10-07-1940-feat-note-types-and-tags-plan.md`).
- Open questions: whether the model can suggest a new tag or only existing tags; whether it can suggest the note type; how the note page shows a suggestion that the user has not accepted yet.
