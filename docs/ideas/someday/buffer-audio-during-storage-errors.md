---
title: Keep audio in memory while storage writes fail
description: "Hold encoded audio in memory while disk writes fail or are slow, write it in order when storage recovers, and warn before the disk is full"
area: capture
added: 2026-10-15
source: "anarlog review, 2026-10-15: commit 80ed31ad"
related: []
---

# Keep audio in memory while storage writes fail

## Idea
When a write of recorded audio fails or is slow, hold up to 64 MB of encoded audio in memory and write it in order when storage recovers. Warn the user when free space is low, before the recording stops at the floor.

## Why
In Longhand, a write error ends the segment and a new segment starts, so the audio in the gap is lost (`crates/recorder` and `src-tauri/src/managers/capture.rs`, near line 637). `LowDisk` stops the recording at the free-space floor with no earlier warning (`crates/recorder/src/chunks.rs`, near line 223). A storage folder on a synced or network drive makes short write failures more likely. anarlog added this in 80ed31ad.
