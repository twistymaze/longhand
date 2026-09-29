---
title: One Storage Folder and Shared Settings - Plan
type: feat
date: 2026-09-28
topic: one-storage-folder-and-shared-settings
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# One Storage Folder and Shared Settings - Plan

## Goal Capsule

- **Objective:** A person who uses Longhand on several computers sets up the summary prompts and the way of working one time. Every computer that uses the same storage folder then uses those settings. The library always has exactly one storage folder, and a new install records with no setup.
- **Product authority:** This Product Contract. The user confirmed the scope on 2026-09-28. It covers two ideas together: [one storage folder](../ideas/next/one-storage-folder.md) and [shared settings in the storage folder](../ideas/next/shared-settings-in-storage.md).
- **Open blockers:** None.

---

## Product Contract

### Summary

Longhand has exactly one storage folder. On a new install it is a folder in the app data folder, which is not synced. When the user changes the folder, the user chooses Move or Leave them. The storage folder holds every setting that does not belong to one computer, and each custom summary prompt is a markdown file there. Each computer that uses the folder uses those settings and follows changes while it runs.

### Problem Frame

One person with several computers already shares one library through a synced storage folder ([one library across computers](../ideas/done/cross-machine-library.md)). But all settings are kept on each computer, so the person must set up each computer again. Custom summary prompts cause the most trouble: the person cannot remember a long prompt, so each copy must be typed again by hand, and the copies then drift apart.

The library also keeps every storage folder that the user has chosen (R25a of the walking skeleton plan) and lists the notes of all of them. Nobody needs several storage folders at the same time. The rule makes "which folder holds the shared settings?" a question with no good answer. A new install also has no storage folder, so the recording shortcut does nothing until the user chooses one.

### Key Decisions

- **One plan covers both ideas.** With one storage folder, the shared settings have exactly one place. (session-settled: user-approved — chosen over shared settings alone on top of several folders, and over a separate plan for one storage folder first: one folder removes the "which root" rule)
- **A setting is shared unless there is a reason to keep it on one computer.** Governs R10, R11, R12. (session-settled: user-directed — chosen over a small hand-picked shared set such as prompts only: the user wants to review every setting from the side of "share it")
- **The transcription model is shared. The shortcuts, interface zoom, Computer audio default and Hide from Dock stay on each computer.** Governs R11, R12. (session-settled: user-directed — the user chose these from the undecided settings)
- **A shared transcription model that this computer does not have downloads automatically.** Recording is never blocked. Governs R16. (session-settled: user-approved — chosen over asking first, and over notes that wait for the model)
- **The default storage folder is in the app data folder.** Audio files are large and private, and Documents is often synced on macOS and Windows. Governs R2, R3. (session-settled: user-directed — chosen over Documents/Longhand and Home/Longhand)
- **After a change of folder, the folder's shared settings win, and custom prompts merge into them.** A second computer joins a library this way, and no prompt is lost. Governs R18, R19. (session-settled: user-directed — chosen over "folder wins" with no merge, and over a question when both exist)
- **Everything works during a Move.** A large library can take minutes to move. Governs R6. (session-settled: user-approved — chosen over read-only notes during the move, and over a blocking progress window)
- **A computer whose notes another computer moved away gets no notice.** Its library empties as the sync removes the files. Governs R9. (session-settled: user-directed — chosen over a marker in the old folder and a notice)
- **Each custom prompt is its own file.** The user can edit a prompt in any editor, and a sync conflict copy of a prompt is kept as a second prompt, so no text is lost. Governs R13. (session-settled: user-approved — chosen over prompts inside the one settings file)
- **Each computer keeps a local copy of the last shared settings.** A missing, moved or unreachable storage folder never resets the prompts to the defaults. Governs R15. (session-settled: user-approved)
- **For the settings file, the last save wins.** A lost change costs one setting, and prompts are safe in their own files. Governs R14. (session-settled: user-approved)
- **A change of folder copies the shared settings and does not remove them from the old folder.** Another computer that still uses the old folder keeps working settings. Governs R18.
- **This plan replaces KD10 of the one-library plan** ("An action uses the settings of the computer that runs it"). A summary still names its prompt, so a summary made before a prompt changed still says which prompt made it.

### Requirements

**One storage folder**

- R1. Longhand uses exactly one storage folder at a time, and the library lists only the notes in it.
- R2. A new install uses a default storage folder in the app data folder, so the recording shortcut and Save work before the user chooses a folder.
- R3. Settings > Storage shows the storage folder, a Change action, and an action that opens the folder in Finder (in Explorer on Windows).
- R4. When the user changes the storage folder, Longhand asks what to do with the notes in the old folder: Move or Leave them. The user can also cancel.
- R5. Move puts every saved note in the new folder. At every moment each note has one complete copy, and after a crash or a full disk the move continues or stops with no lost note.
- R6. While a Move runs, Settings > Storage shows its progress. Recording, the shortcut and note edits keep working, new notes save to the new folder, and the library never lists a note twice or loses one.
- R7. Leave them keeps the notes in the old folder and does not delete them. The library forgets them.
- R8. The list of earlier storage folders goes away. The current folder becomes the one storage folder, and the other folders are forgotten. No installs need migration.
- R9. When another computer moves the notes out of a shared storage folder, this computer shows no special notice. Its library empties as the sync removes the files.

**Shared settings**

- R10. The storage folder holds the shared settings in its hidden Longhand folder. Every computer that uses the storage folder reads and changes them there.
- R11. These settings are shared: the custom summary prompts and the active prompt, the summary endpoint URL, the summary model, the context limit, the learned context limits, the default speaker count, the transcription model, live transcript, the speaker source, the speaker processing mode, the recognition language, and "check for updates automatically".
- R12. These settings stay on each computer: the storage folder path, the microphone, the API key (in the Keychain or the Windows secret store), the model download state, both shortcuts, the interface zoom, the Computer audio default, and Hide from Dock. A setting added later is shared unless it depends on the hardware, operating system, disk or secrets of one computer.
- R13. Each custom summary prompt is its own markdown file in the hidden Longhand folder. The built-in prompts stay in the app. A prompt file that the user changes outside Longhand shows the new text, and a sync conflict copy of a prompt file shows as one more prompt.
- R14. When two computers save the settings file at nearly the same time, the last save wins. Longhand ignores sync conflict copies of the settings file.
- R15. Each computer keeps a copy of the last shared settings that it read. When the storage folder is missing or not reachable, the computer uses that copy, not the defaults.
- R16. When the shared transcription model is not on this computer, the download starts automatically. Until the download is verified, this computer uses its previous model. A failed download shows in Settings, and the previous model stays in use.
- R17. A change of shared settings from another computer applies on this computer while Longhand runs, with no restart.

**Change of storage folder and shared settings**

- R18. After a change of storage folder, by Move or by Leave them: when the new folder has shared settings, this computer uses them. When it has none, the current shared settings are copied into it. The old folder keeps its copy.
- R19. When the new folder has shared settings, each custom prompt of this computer that the folder does not have is added to the folder. A prompt with the same name and the same text is not added again. A prompt with the same name and different text is added with a free name, as Duplicate makes one.

### Key Flows

- F1. Change the storage folder
  - **Trigger:** The user selects Change in Settings > Storage and chooses a folder.
  - **Steps:** Longhand asks Move or Leave them (R4). It applies the shared settings rule and merges the prompts (R18, R19). Then it moves the notes with progress (R5, R6), or it forgets them (R7).
  - **Outcome:** The library shows the notes of the new folder, and this computer uses the shared settings of the new folder.
- F2. A second computer joins
  - **Trigger:** Computer A uses a synced storage folder. The user installs Longhand on computer B.
  - **Steps:** B records into its default folder (R2). Later the user changes B's storage folder to the synced folder (F1). B adopts A's shared settings, adds its own custom prompts (R19), and downloads A's transcription model (R16).
  - **Outcome:** A and B show the same notes and use the same prompts and models.

### Acceptance Examples

- AE1. **Covers R2.** Given a new install, when the user presses the recording shortcut, recording starts and the note saves with no folder choice.
- AE2. **Covers R17.** Given A and B share a storage folder and both run Longhand, when the user changes the active prompt on A, B shows the new active prompt after the sync, with no restart.
- AE3. **Covers R18, R19.** Given B has a custom prompt "Standup" and the synced folder has A's settings with no "Standup", when B changes to that folder, B uses A's settings, the folder gets "Standup", and A shows "Standup" too.
- AE4. **Covers R19.** Given B and the folder each have a prompt "Recap" with different text, when B changes to the folder, both prompts are kept and B's copy gets a free name.
- AE5. **Covers R16.** Given A selects a transcription model that B does not have, when B receives the change, B starts the download. A note that B records before the download ends uses B's previous model.
- AE6. **Covers R6.** Given a Move runs, when the user records a note and stops, the note saves to the new folder and shows one time in the library.
- AE7. **Covers R15.** Given the storage folder is on a disk that is not connected, when Longhand starts, it uses the last shared settings, and the custom prompts are there.
- AE8. **Covers R13.** Given A and B change the same prompt while offline, when the sync makes a conflict copy, both texts show as two prompts.
- AE9. **Covers R9, R18.** Given A and C share a folder, when A moves the notes to a new folder, C's library empties with no notice, and C keeps the shared settings of the old folder.

### Scope Boundaries

- A notice on a computer whose notes another computer moved away (R9).
- Several storage folders at the same time.
- Shared settings with no shared storage folder, such as through an account or a cloud service.
- A shared API key. Each computer keeps its own key.
- A merge of settings other than custom prompts. The folder's values win (R18).
- A summary prompt for each note type ([summary prompt by note type](../ideas/someday/summary-prompt-by-note-type.md)).
- The removal of old audio ([remove the audio of old notes](../ideas/next/remove-old-audio.md)).

### Dependencies / Assumptions

- The sync service delivers the hidden Longhand folder with the notes. The library marker `.longhand/library` already depends on this.
- A prompt file changed in an editor is text that the user typed, so it agrees with the "Nothing writes into Longhand" boundary in `STRATEGY.md`.
- When the storage folder is not available (for example, a disconnected disk), Save waits as it does today.

### Outstanding Questions

**Deferred to Planning**

- How a prompt file's name relates to the prompt name that Settings shows, and how a rename in Longhand changes the file.
- Where each computer keeps the local copy of the shared settings (R15), and how it tells a missing folder from an empty one.
- How Move stays safe after a crash or a full disk (R5): use the Save mechanism again, or a rename when both folders are on one volume.
- How Longhand finds conflict copies of prompt files and of the settings file for each sync service (R13, R14).
- What the default folder is called in the app data folder on macOS and Windows (R2).

### Sources / Research

- `src-tauri/src/managers/settings.rs`: all settings are one `settings.json` in the app data folder (`tauri-plugin-store`). The API key is in the OS secret store (`src-tauri/src/managers/secrets.rs`).
- `crates/library/src/scan.rs`: `LONGHAND_FOLDER` (`.longhand`) and the library marker. The scan skips hidden names.
- `src-tauri/src/shortcut.rs` and `crates/pipeline/src/save.rs`: with no storage folder, the shortcut does nothing and Save waits (`StageError::NoDestination`, "Waiting for a folder").
- `src-tauri/src/managers/models.rs` (`settle_pending`): a pending model becomes the transcription model when its download is verified. R16 uses this path.
- `src-tauri/src/managers/settings.rs` (`rename_summary_prompt`, `free_prompt_name`): prompt names are unique, and Duplicate makes a free name. R19 uses this rule.
- `src/settings/DestinationPicker.tsx`: Settings > Storage has "Choose Folder…" or "Change…" and a list of earlier folders. No action opens the folder in Finder yet.
- `docs/plans/2026-09-25-1217-feat-one-library-across-computers-plan.md`: KD10, which this plan replaces, and the folder watcher that follows notes from other computers.
- `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`: R25a, the rule that keeps every chosen folder, which R8 removes.
