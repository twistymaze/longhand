---
title: Faster Preparing the audio
description: "Make the assemble stage faster for long recordings, for example a long noisy note that the long-silence cut does not shorten"
area: processing
added: 2026-11-15
source: "forgotten-recording brainstorm (conversation, 2026-11-15)"
related: [forgotten-recording]
---

# Faster Preparing the audio

## Idea
Make Preparing the audio (the assemble stage) faster for each hour of kept audio. Today it reads every chunk, resamples it twice and encodes the Opus archive.

## Why
A 43.5-hour note took 13 min 17 s to prepare on the work Mac, and with the live transcript on, this was the longest stage after Stop. The long-silence cut does not shorten a noisy note.

## Notes / Current state
See `crates/pipeline/src/assemble.rs`.
