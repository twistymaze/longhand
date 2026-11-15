---
title: Import Opus audio files
description: "Import voice notes in Opus format (.ogg, .opus, .webm), which the current decoder cannot read"
area: capture
added: 2026-11-15
source: "upkeep review 2026-11-15: Handy symphonia-adapter-libopus"
related: []
---

# Import Opus audio files

## Idea
Decode Opus audio files (.ogg, .opus, .webm) in the import path, for example with symphonia-adapter-libopus, as Handy does.

## Why
Many phone and messaging apps save voice notes as Opus. `crates/audio-import/src/decode.rs` uses symphonia, which has no Opus decoder, so the user cannot import these files.

## Notes / Current state
- It adds a native dependency on libopus. Longhand already builds libopus through opusic-sys.
- Handy needed two more fixes for Windows ARM (2696c49, 1472087).
