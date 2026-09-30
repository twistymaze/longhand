---
title: Resume survives an early audio removal
description: "A Resume keeps working when another computer removes the old audio before the Resume has copied it"
area: capture
added: 2026-09-30
source: "Risks of docs/plans/2026-09-30-0357-feat-remove-old-audio-plan.md (PR #28); Scope Boundaries of docs/plans/2026-09-30-0850-refactor-remove-result-copies-plan.md"
related: []
---

# Resume survives an early audio removal

## Idea

A Resume of a saved note copies the old audio into the working folder while recording runs (KTD8 of the audio removal plan). When another computer removes that audio before the copy is complete, the Resume fails. The Resume could instead keep the audio that it recorded and save the note with that audio only, as it does when the removal happens during the Resume (R16 of the audio removal plan).

## Why

The in-use marker guards a Resume only when the other computer can see it. A computer that is offline or slow to sync can remove the audio in the short time before the copy completes. The user then loses the Resume, not only the old audio.

## Notes / Current state

- PR #28 records this as a known risk.
- The window is short: it lasts until the copy of the old audio completes.
