---
title: Shared settings in the storage folder
description: "Keep the settings that belong to the person, such as summary prompts and models, in the storage folder, so that every computer uses them"
area: library-and-sharing
added: 2026-09-28
source: "the user, after the idea review of 2026-09-28"
related: [cross-machine-library, summary-prompts, summary-prompt-by-note-type]
plan: docs/plans/2026-09-28-2343-feat-one-storage-folder-and-shared-settings-plan.md
---

# Shared settings in the storage folder

## Idea
Some settings belong to the person, not to the computer. Keep them in a hidden Longhand folder in the storage folder, for example `.longhand/settings.json`. Every computer that uses that storage folder reads them there. The settings that belong to one computer stay on that computer.

- **Summary prompts** are files in that folder, for example one markdown file for each prompt. `settings.json` names the active prompt.
- **Shared:** the settings of the person's way of working, such as the transcription model, the live and speaker choices, and the summary model.
- **Local:** the settings that only make sense on one computer, such as the microphone and the path of the storage folder.

## Why
One person with several computers shares one library ([one library across computers](../done/cross-machine-library.md)), but must set up the prompts and the models again on each computer, and keep them the same by hand.

## Notes / Current state
- Today all settings are in one `settings.json` in the app data folder of each computer (`tauri-plugin-store`, `src-tauri/src/managers/settings.rs`). They are not in the SQLite store. The storage folder already has a hidden `.longhand/` folder at its root (`LONGHAND_FOLDER` in `crates/library/src/scan.rs`). It holds the library marker `.longhand/library`, which tells an available storage folder from one whose sync client removed its files. The library scan skips `.longhand/`, so the shared settings can go there.
- A first split of the current fields, for a brainstorm to confirm:
  - **Shared:** `summary_prompts`, `active_summary_prompt`, `endpoint_base_url`, `endpoint_model`, `context_limit_tokens`, `default_speaker_count`, `asr_model`, `live_transcript`, `speaker_source`, `speaker_processing_mode`, `recognition_language`, `system_audio_default`. Possibly `learned_context_limits`.
  - **Local:** `destination_roots`, `current_destination`, `microphone`, `interface_zoom_percent`, `hide_from_dock`, the download state (`pending_asr_model`, `pending_recognition`), and the API key, which stays in the Keychain of each computer.
  - **To decide:** the two shortcuts.
- Questions for a brainstorm:
  - The settings keep several storage roots (R25a). Which root holds the shared settings: the current one only?
  - A shared model that this computer has not downloaded yet: download it, or keep a local choice until it is ready.
  - Two computers that change the settings at the same time, through a sync service that can give conflicting copies.
  - A change on another computer while Longhand runs: follow it, as the folder watcher follows notes.
  - What happens when the user changes the storage folder to one that has its own shared settings.
