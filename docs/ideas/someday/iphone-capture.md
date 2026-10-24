---
title: Record on an iPhone
description: "A small iPhone app that records a note when a laptop is not available, and puts it in the storage folder for a computer to process, or transcribes it on the phone"
area: capture
added: 2026-10-13
source: "conversation about iPhone capture, 2026-10-13"
related: [import-recording-file]
---

# Record on an iPhone

## Idea
A small iPhone app that only records. It does not show the library and does not edit notes. Two levels:

1. **Record only.** The phone records the audio and the user can type a title, note type and tags. The phone writes the recording into a folder that syncs to the user's computer. A computer that the user chooses makes the transcript and summary.
2. **Record and transcribe.** The phone also makes the transcript, and writes a complete saved note into the storage folder, as a computer does.

## Why
A laptop is not always available or suitable. The user wants a transcript of those conversations too, in the same library.

## Notes / Current state
- **Do [import-recording-file](../next/import-recording-file.md) first.** With Voice Memos and AirDrop, it gives most of the value of level 1 with no phone app. Build the phone app only if that is not sufficient, for example because the user wants note type, tags and notes at the time of the recording.
- **Sync on iPhone.** iCloud Drive works well. OneDrive and Dropbox show in the Files app, and an app can write to a folder that the user chooses there; the upload happens when the iPhone lets the provider app run. Resilio Sync on iOS is limited: it syncs mainly while its app is open and the peer is reachable. Verify each one before the work. Longhand does not make its own sync. Where no sync is available, the share sheet and AirDrop still move the file.
- **Who processes a level 1 recording.** Today, only the computer that starts a publication finishes it, and a note that another computer has not saved yet does not show. A recording from the phone is a new kind of input: audio with no transcript yet. If each computer that sees it starts the work, two computers can process it at the same time, because a sync folder has no lock. Recommended: a setting on one computer, "Process recordings from my phone". Only that computer takes them, and it publishes the note in the usual way. Then the phone never writes a saved note, and the current rules do not change.
- **Level 2 is much more work.** The Rust core and the models probably build for iOS (whisper.cpp and ggml support Metal on iOS), but: Tauri on iOS is less complete than on macOS; the audio session and background recording need Swift code; iOS can stop heavy processing when the app is in the background; a four-hour transcript on a phone uses much battery and heat; model downloads are large on a phone. The phone would then be one more "computer" that publishes notes, which the rules permit (KD4: the note folder is the authority).
- **Read old notes on the iPhone: do not build.** The notes are plain files. When the storage folder is in an Obsidian vault, Obsidian on iOS already shows them, and the Files app shows them too. `STRATEGY.md` says "Obsidian already exists".
- The phone app must not draw attention during a recording, as U7 says for the desktop.
