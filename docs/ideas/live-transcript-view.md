---
title: Live transcript while recording
status: open
area: transcript-and-summary
added: 2026-09-21
source: "Deferred from the walking skeleton (docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md, KD8, R10); the retired FUTURE-IDEAS.md"
related: [ux-redesign, recognition-model-picker]
---

# Live transcript while recording

## Idea
Show the transcript as it forms, when the selected model streams.

## Notes
- Settled in the plan (KD8): a live transcript is a capability of the selected model, not a promise of the product. When the model does not stream, the surface says the transcript arrives when processing finishes.
- The default model (Whisper large-v3-turbo) does not stream, so this depends on a streaming model being selectable.
- The UX redesign input asks whether the live view comes with the redesign or after it, and warns that a scrolling transcript pulls eyes back to the screen (the attention budget).
