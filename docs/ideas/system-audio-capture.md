---
title: System audio capture
status: done
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
- Planned and built in `docs/plans/2026-09-26-0827-feat-system-audio-and-microphone-choice-plan.md`: a Computer audio switch for each note, a stereo recording of the raw microphone and raw system audio, a live mix with an echo gate, echo removal after Stop, and a mono mix for the existing pipeline.
- Windows follows one output device (anarlog's rule). When the followed device is silent while another plays, the recording page warns. Capture of more than one output is a possible later idea.
- The transcript's shape does not change: turns do not show Room or Call. The stereo recording keeps per-channel processing possible later.
