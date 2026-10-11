---
title: Faster diarization
description: "Make speaker recognition fast, so that a long note is ready soon after Stop"
area: processing
added: 2026-09-26
source: "The user, during the archived PR 12 review of stereo processing"
related: [system-audio-capture, alternative-diarization-architectures]
plan: docs/plans/2026-09-27-1526-feat-flexible-speaker-recognition-plan.md
---

# Faster diarization

## Idea
Make the Diarize stage much faster, so that a long note is ready soon after Stop.

## Why
The user reports that diarization is too slow now. [Per-channel processing](../parked/system-audio-capture.md#process-each-channel-on-its-own) would diarize two full-length channels, which doubles the cost, so this work comes first.

## Current state

Shipped in archived PR 18. On 2026-09-28 the user confirmed that the speed is good enough in normal use; the three-hour laptop test is not a condition.

The [flexible speaker recognition plan](../../plans/2026-09-27-1526-feat-flexible-speaker-recognition-plan.md) covers accelerated embeddings, recognition during recording, and user-selected model routes. The preferred target is a usable transcript and summary within two minutes after a three-hour recording stops on the intended laptop. Slower choices remain available.

A five-minute M3 Ultra probe reduced existing-pipeline analysis from 60.547 seconds on CPU to 9.200 seconds with Core ML embeddings. This does not establish full-note or laptop performance. Focused long-recording checks are in scope; an accuracy leaderboard is not.
