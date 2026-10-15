---
title: Use Handy's recorder and resampler crates
description: "Decide whether to use the external handy-recorder and handy-cpal crates in place of the recorder and resampler files that Longhand copied from Handy"
area: capture
added: 2026-10-15
source: "Handy review, 2026-10-15: commit a42131d"
related: [transcribe-cpp-update]
---

# Use Handy's recorder and resampler crates

## Idea
Handy moved its recorder and resampler out of its repository into the crates `handy-recorder` 0.1.0 and `handy-cpal` 0.18.2 (a fork of cpal). Longhand copied `recorder.rs` and `resampler.rs` from Handy (see the `PROVENANCE.md` files in `crates/audio-cpal`, `crates/audio` and `crates/pipeline`). Compare Longhand's copies with the crates, then either use the crates as dependencies or keep the copies and take upstream fixes from the crates by hand.

## Why
Upstream fixes to this code no longer show in Handy's git history. Without a decision, Longhand's copies drift from upstream and the reference reviews can miss fixes.

## Notes / Current state
- Longhand changed its copies (for example, it streams to disk and does not hold the full recording). The crates may not fit without changes.
- `handy-cpal` is a cpal fork. Longhand uses cpal `=0.18.2`. A switch would also change the cpal that `crates/audio-cpal` uses.
- Handy weak-links CoreAudio because cpal uses macOS 14.2+ functions. Longhand's minimum is macOS 15.0, so it does not need this.
- Until this is decided, each reference review must also read the source of the two crates.
