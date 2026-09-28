---
title: A diarization evaluation loop on real sessions
description: "Measure speaker turns on real notes again and again"
area: transcript-and-summary
added: 2026-09-23
source: "the retired FUTURE-IDEAS.md"
related: [alternative-diarization-architectures]
---

# A diarization evaluation loop on real sessions

## Idea
Run a diarization-error-rate harness (anarlog ships one, with a documented tuning procedure) against real session audio, repeatedly, to answer whether one microphone and eight people actually works.

## Current state
U1 measured the borrowed stack once against a frozen gate: `docs/reference/diarization-measurement.md`. There is no ongoing loop over real sessions.
