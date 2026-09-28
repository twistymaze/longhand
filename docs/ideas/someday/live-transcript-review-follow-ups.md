---
title: Follow-ups from the live transcript review
description: "Eight small risks that the PR #8 review found and did not fix"
area: processing
added: 2026-09-25
source: "Unapplied P3 findings of the code review of PR #8 (run 20260925-rmlt-01), docs/plans/2026-09-25-1121-feat-recognition-models-and-live-transcript-plan.md"
related: [recognition-model-picker, live-transcript-view]
---

# Follow-ups from the live transcript review

## Idea
Fix the small residual risks that the code review of the recognition models and live transcript work found but did not apply.

## Why
Each one is rare and has a workaround, so none blocked the merge. Two of them can confuse a user, and they are the first to fix.

## Notes / Current state
First to fix:
- `crates/pipeline/src/live.rs`: a panic inside the live worker does not clear the live-model mark. Until the app restarts, Delete refuses that model.
- `src-tauri/src/commands/settings.rs`: pick model X, then Y, then X again before the cancelled download of X ends. The last pick can be lost.

The others:
- `crates/asr/src/stream.rs`: replay dedup at a forced cut uses the end time of the last kept word. An inflated end time could drop replay words. This is not confirmed with real token timings.
- `crates/pipeline/src/runner.rs`: if the store call in `fail_transcribe_again` fails, the working copy and the Transcribe Again mark stay.
- `src-tauri/src/commands/models.rs`: Delete checks and removes files without one lock over both steps, so a select of the same model between the two steps is possible.
- `crates/pipeline/src/transcribe.rs`: `PART_GAP_MIN_MS` (10 ms) ignores tiny gaps between parts. The real drift between part ends and voiced ends is not measured.
- `crates/session-core/src/jobs.rs` and `src-tauri/src/managers/capture.rs` are longer than 1000 lines. A split would make them easier to read.
- `src-tauri/src/managers/live.rs`: `LiveManager::start` and its wiring in `capture.rs` need an `AppHandle` and have no direct test.
