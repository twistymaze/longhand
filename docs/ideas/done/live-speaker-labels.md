---
title: Speaker labels in the live transcript
description: "Join the live text to the live speaker turns during recording, so the recommended route shows speaker labels live"
area: transcript-and-summary
added: 2026-09-29
source: "Shorthand review, 2026-09-29 (docs/reference/reference-source-reviews.md)"
plan: docs/plans/2026-09-29-1143-feat-live-speaker-rows-plan.md
related: [live-transcript-view, alternative-diarization-architectures, system-audio-capture]
---

# Speaker labels in the live transcript

## Idea
During recording, join each live text row to the live speaker turns and show a provisional speaker label on the row. After Stop, the Join stage gives the final labels as it does now.

## Why
The user wants a live transcript with speaker labels. Today only the CombinedNemo route shows live labels (`src/session/LiveTranscript.tsx:131-139`). With the recommended route (Parakeet Unified and a separate speaker model), the live text has no labels, and the live speaker turns show only in the collapsed "Speaker analysis so far" list.

## Notes / Current state
Shipped in PR #26. The Transcript tab shows the live text as speaker rows during recording, with paragraphs that use the final transcript's rule. Numbers restart in each 20-minute speaker analysis window, and a "Speaker labels restart" divider marks each restart. The notes below are from before the plan.

- Shorthand does not identify speakers by voice. Its "Me" and "Them" labels come from the capture lane, so on one microphone it gives one label to all speakers. There is nothing to borrow for the in-person case.
- **Smallest first step:** `LiveRow.words` already have timings (`crates/pipeline/src/live.rs:78-88`), and the live NeMo speaker worker already sends provisional `SpeakerTurn`s. Run the `join_separate` rule on each saved part and on the open text, and emit attributed rows. No new model is necessary.
- **Speakers across NeMo epochs:** each 20-minute epoch has its own speakers, and no code links them (plan KTD6 allows a link only with validated evidence). A possible link: a WeSpeaker centroid for each epoch speaker (`crates/embedding`), merged only above a strict threshold, so that an error splits a person and does not join two people. Without a link, a 3-hour note can show many labels (inferred, not observed).
- **Accuracy is not measured** for Sortformer and Nemotron 3 on the 3-hour one-microphone session. Measure with the collapse gate in `docs/reference/diarization-measurement.md` before live labels become the default.
- **Final pass:** a pass after Stop that replaces live NeMo labels with offline pyannote labels changes plan KTD5 ("no second successful-result refinement pass"). That needs a product decision.
- **Attention budget:** labels that change during recording can pull attention to the screen (U7). The Transcript tab is not open by default (KD5), which limits this.
