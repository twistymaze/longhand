---
title: Prevent sleep during processing
status: done
area: processing
added: 2026-09-27
source: "User-confirmed speaker recognition brainstorm"
related: [faster-diarization, alternative-diarization-architectures]
---

# Prevent sleep during processing

## Idea

Prevent idle system sleep while post-recording work runs. Release protection on completion, failure, or cancellation. Allow display sleep and screen locking. This extends recording sleep protection; it does not add power-aware scheduling.

## Current state

Shipped in PR #18 ([flexible speaker recognition plan](../../plans/2026-09-27-1526-feat-flexible-speaker-recognition-plan.md)). The pipeline runner holds a processing lease while it works (`crates/power/src/processing.rs`).
