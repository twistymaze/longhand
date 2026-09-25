---
title: Schedule processing around power and thermals
status: open
area: processing
added: 2026-09-23
source: "the retired FUTURE-IDEAS.md"
related: [prevent-sleep-opt-out]
---

# Schedule processing around power and thermals

## Idea
Defer processing to when the machine is idle and plugged in, rather than starting it immediately on battery.

## Why
A four-hour session is twenty to sixty minutes of sustained local compute.

## Notes
- The live transcript (`docs/plans/2026-09-25-1121-feat-recognition-models-and-live-transcript-plan.md`) runs recognition during recording, so a streaming model keeps the CPU and GPU busy for the whole session. On battery, a rule could turn the live transcript off and let the gaps fill after Stop. The Live transcript switch already turns it off by hand.
