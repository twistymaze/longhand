---
title: Choose the recognition model
description: "A Settings picker for the transcription model, with download and delete"
area: interface
added: 2026-09-21
source: "Deferred from the walking skeleton (docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md, Scope Boundaries); also in the UX redesign input"
related: [ux-redesign, live-transcript-view]
---

# Choose the recognition model

## Idea
A Settings picker for the recognition model, and a second engine to pick from.

## Notes
- The skeleton hard-codes Whisper large-v3-turbo, which is not the model the user intends to use day to day.
- The shape is already there: the catalog lists models, settings carry `asr_model`, each session records the model it was stopped with, and recognition sits behind the `Recognizer` trait.
- A second engine may need its own runtime; KTD6 records why `transcribe-rs` (Parakeet) is excluded this version.
- Done: `docs/plans/2026-09-25-1121-feat-recognition-models-and-live-transcript-plan.md`. The Transcription pane lists nine models from Handy's catalog, all on `transcribe-cpp`, so no second engine was needed. A pick downloads the model, and a downloaded model that is not selected can be deleted. Parakeet Unified EN is the default for new installs. The speaker-detection weights show as one "Other required models" item.
