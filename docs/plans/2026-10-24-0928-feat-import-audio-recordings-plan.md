---
title: Import Audio Recordings - Plan
type: feat
date: 2026-10-24
topic: import-audio-recordings
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Import Audio Recordings - Plan

## Goal Capsule

- **Objective:** A recording that another device made, for example a phone, becomes a normal Longhand note with a transcript and a summary. The user drags it in on the Mac, or the user does nothing on the Mac because a watched folder brings it in.
- **Product authority:** The user chose each decision below in the brainstorm on 2026-10-24. Reading the private Voice Memos folder and an iPhone app are not active scope (see `docs/ideas/someday/iphone-capture.md`).
- **Open blockers:** None.

---

## Product Contract

### Summary

Longhand gets two ways to make a note from an audio file. The user can drop or pick files, and a small sheet sets the note type and tags before processing starts. Or the user can turn on a watched folder on one computer, and each new audio file in it becomes a note with no action. Longhand never changes the files in the watched folder, and a plain-file import record in the storage folder makes sure that no file is imported twice.

### Problem Frame

Some conversations happen where a laptop is not available or not suitable, for example in a hallway or at a table. A phone is available, and any recorder app on it can record the conversation. Today Longhand cannot use that recording, so the transcript and the summary are missing for exactly those conversations.

`STRATEGY.md` permits this input: "A recording you made on another device counts as what you record."

Voice Memos is the most common recorder on an iPhone, but it syncs its recordings only into a private folder that macOS protects (`~/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings/`). Access to it needs Full Disk Access, and its layout is not a public interface. Other recorder apps can save into a normal iCloud Drive, OneDrive or Dropbox folder by themselves, and Voice Memos can do the same with Share › Save to Files.

### Key Decisions

- **Both a manual import and a watched folder.** Some users want each file to process with no action; others want to choose each file. Governs R1–R10. (session-settled: user-directed — chosen over manual import only, and over reading the private Voice Memos folder: one generic folder works with any recorder app and needs no special permission.)
- **Longhand never touches the files in the watched folder.** Governs R7. (session-settled: user-directed — chosen over moving each file into an "Imported" subfolder and over deleting it: the user's folder stays the user's.)
- **The import record is in plain files, not only in the database.** Governs R11, R12. (session-settled: user-directed — chosen over a record in the database alone: the record must survive a database rebuild and reach other computers.)
- **A deleted note does not come back.** Governs R12. (session-settled: user-directed — chosen over importing the file again and over asking at delete time.)
- **The watched folder is a setting of one computer.** Governs R4, R13. (session-settled: user-directed — chosen over a shared setting with claim files and over a shared setting with a named computer: two computers that watch one synced folder can both import a file before the other's record syncs.)
- **Ask once about files that are already in the folder.** Governs R6. (session-settled: user-directed — chosen over importing all of them and over skipping all of them: an old archive could start hours of processing.)
- **The manual sheet has title, type and tags only.** Type and tags go into every summary call, and the type chooses the summary prompt, so setting them first prevents a second summary. Governs R2. (session-settled: user-directed — chosen over a sheet that also has a Notes field: the sheet must stay small.)
- **A watched-folder note gets the default note type and no tags.** Governs R5. (session-settled: user-directed — chosen over a type and tags on the folder setting, and over holding the summary until the user confirms.)
- **The file name gives only the first title.** Governs R15. (session-settled: user-directed — the summary sets the final title, as for a recording.)
- **The audio becomes normal note audio, and a file's identity is its content.** Governs R12, R17. (session-settled: user-approved — proposed in the scoping synthesis with the alternatives of keeping the original file and identifying by name; the user accepted.)

### Requirements

**Manual import**

- R1. The user can drop one or more audio files on Home, or choose them with an Import Audio action, and Longhand makes one new note for each file.
- R2. Before processing starts, a small sheet shows the title, the note type and the tags. The type is preset to the default note type and the tags are empty, so one Import click accepts it. With several files, the sheet sets the same type and tags for all of them, and each note gets its title from its own file. Cancel makes no notes.
- R3. A manual import always makes a new note, also for a file that the import record already holds.

**Watched folder**

- R4. Each computer has its own watched-folder setting, off by default. The user chooses the folder with the system folder dialog.
- R5. While Longhand runs, each new audio file in the watched folder or its subfolders becomes a new note with the default note type and no tags, and it processes fully with no user action. A file that arrives while Longhand is closed is imported at the next launch.
- R6. When the user chooses a folder that already holds audio files that are not in the import record, Longhand says how many there are and asks once: import them, or import only new files. The files that the user does not import go into the import record as skipped.
- R7. Longhand never changes, moves, renames or deletes a file in the watched folder.
- R8. Longhand imports a file only when it is complete. It first downloads a file that is online only, and it waits while a file still changes.
- R9. The watched folder cannot be the storage folder, cannot be inside it and cannot contain it. The setting refuses such a folder and says why.
- R10. The user can turn the watched folder off or choose another folder at any time. Turning it off stops new imports and does not change notes that were already made.

**Import record**

- R11. Each import is recorded in two places: in the note, which records its source file, and in a plain-file import record in the hidden `.longhand` folder of the storage folder.
- R12. The watcher never imports a file that the import record holds, also after the user deletes the note. A file's identity is its content, so a file that was renamed or saved again with the same content is the same file.
- R13. Before it imports a file, a computer reads the shared import record, so it does not import a file that another computer already imported.

**The note**

- R14. An imported note is a normal note. It processes on this computer in the same queue as a recorded note, and it has manual notes, Transcribe Again, Regenerate Summary, type changes and audio removal like a recorded note.
- R15. The note's first title is the file name without its extension. The summary sets the final title, as it does for a recording, unless the user renamed the note before that.
- R16. The note's start time is the recording time from the file's metadata. When the file has none, it is the file's own time.
- R17. Longhand converts the audio to the normal audio of a note. The note keeps no copy of the original file.
- R18. An import never interrupts a recording or makes it worse. It waits in the processing queue.

**Formats and messages**

- R19. Longhand accepts m4a (AAC), wav and mp3. The watcher ignores all other files and shows no message for them.
- R20. Longhand shows a message only when the user must act or the import failed: the watched folder is unavailable, a file in an accepted format cannot be read, or the user dropped or chose a file that is not in an accepted format.

### Key Flows

- F1. Manual import
  - **Trigger:** The user drops audio files on Home, or chooses Import Audio.
  - **Steps:** The sheet shows the title, the default note type and empty tags. The user changes them or not, and selects Import. Each file becomes a note that processes like a recording.
  - **Covered by:** R1, R2, R3, R14–R18
- F2. Phone to watched folder
  - **Trigger:** The user records on the phone. The recorder app saves into a synced folder, or the user selects Share › Save to Files in Voice Memos.
  - **Steps:** The file syncs to the Mac. When it is complete, the computer with the watched folder records it in the import record and makes a note with the default type. The note processes and appears in the library on every computer.
  - **Outcome:** The user finds a finished note. If the type or tags are wrong, the user changes them and selects Regenerate Summary.
  - **Covered by:** R4, R5, R8, R11–R18
- F3. First choice of a folder
  - **Trigger:** The user turns on the watched folder and chooses a folder.
  - **Steps:** Longhand checks R9. If the folder holds audio files that are not in the import record, Longhand asks once (R6). Watching starts.
  - **Covered by:** R4, R6, R9

### Acceptance Examples

- AE1. **Covers R12.** **Given** a note made from `Team sync.m4a` in the watched folder, **when** the user deletes the note and the file stays in the folder, **then** Longhand makes no new note from it, also after a relaunch.
- AE2. **Covers R3, R12.** **Given** the same deleted note, **when** the user drags `Team sync.m4a` onto Home, **then** Longhand makes a new note.
- AE3. **Covers R12.** **Given** an imported file, **when** the user renames it in the folder, **then** Longhand makes no new note.
- AE4. **Covers R6.** **Given** a folder with 40 old recordings, **when** the user chooses it and selects "only new files", **then** no note is made for the 40 files, and a 41st file that arrives later becomes a note.
- AE5. **Covers R8.** **Given** a new file in an iCloud folder that is still online only or still syncing, **when** the watcher sees it, **then** Longhand waits and makes the note only from the complete file.
- AE6. **Covers R13.** **Given** two computers that share a storage folder, where computer A watches a folder and imported a file, **when** the user also turns on watching of the same folder on computer B after the import record synced, **then** computer B does not import that file again.
- AE7. **Covers R15.** **Given** an imported note that is processing, **when** the user renames it before the summary finishes, **then** the summary does not change the user's title.
- AE8. **Covers R19, R20.** **Given** a `.txt` file and a damaged `.m4a` file in the watched folder, **when** the watcher sees them, **then** the `.txt` file gets no message and no note, and the `.m4a` file shows a message that names it.

### Scope Boundaries

- Reading the private Voice Memos folder, and Voice Memos' internal `.qta` files.
- An iPhone app (`docs/ideas/someday/iphone-capture.md`).
- A Notes field in the import sheet.
- A note type or tags on the watched-folder setting.
- More than one watched folder on one computer.
- Holding the summary of a watched-folder note until the user confirms its type and tags.
- Moving, renaming or deleting files in the watched folder.

### Dependencies / Assumptions

- Longhand is not App Sandboxed: `src-tauri/Entitlements.plist` has only the microphone keys, and no security-scoped bookmark code exists. A folder that the user chooses in the system dialog is therefore readable with no extra prompt. Whether access to a protected folder (Documents, iCloud Drive) continues after a relaunch with no new macOS prompt is an assumption. Test it in a signed build.
- Two computers that both watch the same folder at the same moment can still import one file twice before the import record syncs. R4 makes this a deliberate user setup, not a default.

### Outstanding Questions

**Deferred to Planning**

- How to decode m4a, wav and mp3. No decoder for these exists (`crates/pipeline/Cargo.toml` has `hound`, `opus` and `ogg` only).
- How to compute a file's content identity cheaply for large files, and how several computers write the shared import record safely.
- How Longhand tells that a file is complete and stable (R8), and how it asks the sync service to download an online-only file.
- Where Import Audio appears (Home, the app menu, or both) and the exact sheet layout. Invoke `apple-design` first.
- Whether the watcher reuses the library watcher in `src-tauri/src/managers/library.rs` or gets its own.
- How an imported note enters the pipeline: its audio has no recording segments, so the Assemble stage needs an input from a file.

### Sources / Research

- `docs/ideas/next/import-recording-file.md`: the original idea for a manual import.
- `src-tauri/src/managers/library.rs`: the existing `notify` watcher, with a 900 ms debounce and a full scan each minute because FSEvents can drop events and File Provider folders do not always report changes.
- `src-tauri/src/commands/settings.rs` (`pick_storage_folder`): the only folder dialog today.
- `src-tauri/src/managers/local_store.rs`: where settings that belong to one computer are stored.
- `crates/session-core/src/state.rs`: the stages Assemble, Diarize, Transcribe, Join, Verify, Summarize, Save.
- `crates/summarize/src/prompts.rs`: every summary call carries the note's type, tags and title.
- `docs/solutions/integration-issues/shared-settings-in-a-synced-folder-rules.md`: an online-only iCloud file can block a read, so do every read off the main thread.
