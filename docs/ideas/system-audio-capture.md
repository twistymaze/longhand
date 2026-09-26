---
title: System audio capture
status: in-progress
area: capture
added: 2026-09-21
source: "Deferred from the walking skeleton (docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md, Scope Boundaries and How This Work Fits Together)"
related: [live-transcript-view]
---

# System audio capture

## Idea
A second capture path for the computer's own audio, alongside the microphone, with its permission surface. The same work lets the user choose the microphone instead of only the system default.

## Why
Needed for meetings held on calls. Irrelevant to an in-person room, which is why the skeleton left it out.

## Notes
- Brainstormed in `docs/plans/2026-09-26-0827-feat-system-audio-and-microphone-choice-plan.md`: a Computer audio switch for each note, a stereo recording of the raw microphone and raw system audio, echo removal after Stop, and a mono mix for the existing pipeline.
- The transcript's shape does not change: turns do not show Room or Call. The stereo recording keeps per-channel processing possible later.
