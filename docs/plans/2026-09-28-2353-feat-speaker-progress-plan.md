---
title: Speaker progress - Plan
type: feat
date: 2026-09-28
topic: speaker-progress
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Speaker progress - Plan

## Goal Capsule

- **Objective:** After Stop, a person can see that speaker recognition is advancing and can tell when it has finished.
- **Means:** Rename the speaker stage and show progress for the full note while that stage runs.
- **Product authority:** The user confirmed the whole-note meaning of progress and the scope on 2026-09-28. This contract governs R1-R6 and AE1-AE4.
- **Open blockers:** None for product scope. Planning must check which speaker routes and final work can report measured progress.

## Product Contract

### Summary

The processing view will call its speaker stage "Finding speakers" and show a whole-number progress percentage with a bar. The bar will advance during the stage and finish when speaker work is complete.

### Problem Frame

The speaker stage can take a long time after Stop. Today it shows a dot while Transcribing shows a percentage and a bar. The user has had to wait without knowing whether speaker work was advancing.

### Key Decisions

- **Whole-note progress.** (session-settled: user-directed — chosen over per-segment progress: the user wants one bar that moves as the note advances.) Governs R2-R4.
- **Whole numbers.** (session-settled: user-directed — chosen over decimal precision: decimal-level progress is unnecessary.) Governs R3.
- **Keep the stage order.** The existing speaker stage remains before Transcribing. Governs R6.

### Requirements

- R1. The processing view and note status call the speaker stage "Finding speakers" without suggesting that Longhand identifies named people.
- R2. While the speaker stage runs, its progress covers the full note and does not reset between audio segments.
- R3. The bar and whole-number percentage advance during measurable speaker work, including long parts of the stage, without waiting a minute or two and then jumping to completion.
- R4. When work continues but its fraction cannot be measured or its whole-number percentage has not changed, the view shows activity instead of a frozen or invented percentage.
- R5. The speaker stage shows 100% only when its work has finished.
- R6. The processing stage order stays as it is, with Finding speakers before Transcribing.

### Acceptance Examples

- AE1. **Covers R1, R6.** After Stop, a note that uses separate speaker recognition shows "Finding speakers" before "Transcribing" in the processing list and on the note's status.
- AE2. **Covers R2, R3.** A note with several audio segments shows one percentage that advances as speaker work proceeds across them; the bar does not return to zero at a segment boundary.
- AE3. **Covers R3, R4.** During a long speaker phase, visible progress changes as work is measured. During a part that cannot yield a reliable fraction, the view shows ongoing activity and does not present a stalled or guessed number as progress.
- AE4. **Covers R5.** The speaker stage does not show 100% while final speaker work remains. It shows completion when that work succeeds.

### Scope Boundaries

- This work changes the speaker stage's status display. It does not change speaker recognition models, quality, or processing speed.
- It does not add an estimated finish time or change the order of processing stages.

### Sources / Research

- `docs/ideas/next/processing-status-clarity.md` records the idea and the earlier decision to keep stage order.
- `CONCEPTS.md` defines speaker recognition as anonymous labels, not identification of named people.
- `src/session/ProcessingStatus.tsx` renders a bar when the running stage reports a total; `src/processing/describe.ts` owns the stage label.
- `crates/pipeline/src/runner.rs` reports a percentage for one speaker-analysis segment, while `crates/pipeline/src/diarize.rs` performs final speaker work after segment analysis. Planning must account for both when meeting R2-R5.
