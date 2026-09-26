---
title: Faster diarization
status: open
area: processing
added: 2026-09-26
source: "The user, during the PR #12 review of stereo processing"
related: [per-channel-processing, diarization-evaluation-loop, alternative-diarization-architectures]
---

# Faster diarization

## Idea
Make the Diarize stage much faster, so that a long note is ready soon after Stop.

## Why
The user reports that diarization is too slow now. [Per-channel processing](per-channel-processing.md) would diarize two full-length channels, which doubles the cost, so this work comes first.

## Notes
- Needs its own plan. Start by measuring where the time goes, stage by stage, on a real long note (embedding, clustering, windows).
- Related options already recorded:
  - [alternative diarization architectures](alternative-diarization-architectures.md), such as models that report speaker labels during recognition;
  - [a diarization evaluation loop](diarization-evaluation-loop.md), so that a faster path is not also a worse one.
