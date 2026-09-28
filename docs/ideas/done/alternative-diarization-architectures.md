---
title: Alternative diarization architectures
description: "Speaker labels from the transcription model or a separate speaker model, or off"
area: transcript-and-summary
added: 2026-09-23
source: "the retired FUTURE-IDEAS.md"
related: [diarization-evaluation-loop, faster-diarization, model-capabilities-and-speaker-options]
---

# Alternative diarization architectures

## Idea

Offer model-provided speaker labels and separate speaker models, plus an off option. Keep runtime selection inside Longhand.

## Current state

Shipped in PR #18. The [simplify recognition models plan](../../plans/2026-09-28-1337-feat-simplify-recognition-models-plan.md) then removes the speaker labels that the transcription models give, so every model uses the separate speaker model. The text below describes the PR #18 scope.

The [flexible speaker recognition plan](../../plans/2026-09-27-1526-feat-flexible-speaker-recognition-plan.md) covers MOSS, the speaker-capable Multitalker bundle, and the new Granite Speech 4.1 2B Plus catalog entry. Separate choices include Nemotron 3 Diarization, Sortformer, and the existing pyannote/WeSpeaker pipeline.

Planning compares combined NeMo-Speech.cpp transcription and speaker recognition with transcribe.cpp transcription plus separate NeMo speaker analysis. Both approaches are in scope. No maintained transcribe.cpp fork is planned. User-facing text calls this "speaker recognition"; named speaker identity remains outside scope.
