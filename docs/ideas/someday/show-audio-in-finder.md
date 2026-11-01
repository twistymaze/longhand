---
title: Show the audio of a note in Finder
description: "A Show Audio in Finder (Show Audio in Explorer) item in the note menu, off when the note has no audio"
area: library-and-sharing
added: 2026-10-31
source: "upkeep review 2026-10-31: anarlog 688c0cf9"
related: []
---

# Show the audio of a note in Finder

## Idea
Add an item to the note menu that shows the note's recording in Finder (Explorer on Windows). When the note has no audio, the item is off.

## Why
Longhand keeps the recording of a note on disk, but no action shows it. A user who wants to play, share or keep the audio must find the file by hand. anarlog added this item in 688c0cf9, with handling for a note that has no audio.

## Notes / Current state
- Settings already has Show in Finder for the storage folder (`src/settings/DestinationPicker.tsx`, Rust command in `src-tauri/src/commands/settings.rs`). The same path can reveal one file.
