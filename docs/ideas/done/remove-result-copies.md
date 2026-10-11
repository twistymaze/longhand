---
title: Remove the result copies from note folders
description: "Stop keeping .longhand-results copies of every result in each note folder; write results directly and delete recognition.jsonl with the audio"
area: library-and-sharing
added: 2026-09-30
source: "the user, after the audio removal work (archived PR 28), found .longhand-results folders that are never cleaned up"
related: [remove-old-audio]
plan: docs/plans/2026-09-30-0850-refactor-remove-result-copies-plan.md
---

# Remove the result copies from note folders

## Idea
Each save or replacement of a result writes a full copy of the transcript, summary and recognition.jsonl to `.longhand-results/` in the note folder, and the old copies are never deleted. Remove the copies, write the result files directly with `session.md` last, and use the fingerprints in `session.md` to recognize a sync in progress. Delete `recognition.jsonl` when the audio of a note is removed.

## Why
The copies grow with every replacement, sync to every computer, and keep old versions of the transcript. They protect only against a few seconds of mismatch while a sync is in progress, which the user accepts.

## Notes / Current state
- Brainstorm done on 2026-09-30; see the plan.
- Shipped in archived PR 29.
