---
title: Alternative diarization architectures
status: parked
area: transcript-and-summary
added: 2026-09-23
source: "the retired FUTURE-IDEAS.md"
related: [diarization-evaluation-loop]
---

# Alternative diarization architectures

## Idea
If the pyannote path disappoints: MOSS does joint transcription and diarization with no speaker ceiling; Sortformer runs as a separate speaker track joined against a strong transcription model.

## Notes
Both have caveats recorded in `docs/reference/borrowing-notes.md`. Parked until an evaluation shows the current path falling short.
- `transcribe-cpp` 0.2.3 already reports speaker labels for models that produce them (`Feature::Diarization`: MOSS, Sortformer). Handy's catalog also has Multitalker Parakeet Streaming. The recognition-model plan (`docs/plans/2026-09-25-1121-feat-recognition-models-and-live-transcript-plan.md`, KD4) adds MOSS Transcribe Diarize and Multitalker Parakeet as recognizers only. Their own speaker labels are the next step.
