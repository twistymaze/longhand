---
title: Remove the Audio of Old Notes - Plan
type: feat
date: 2026-09-30
topic: remove-old-audio
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Remove the Audio of Old Notes - Plan

## Goal Capsule

- **Objective:** The audio of a saved note does not stay on disk longer than the user wants. By default, the audio goes 30 days after the note last recorded. The transcript, summary, notes and title stay.
- **Product authority:** This Product Contract. Source idea: `docs/ideas/next/remove-old-audio.md`.
- **Open blockers:** None.

---

## Product Contract

### Summary

A shared setting, on by default at 30 days, permanently deletes the audio of each saved note that many days after the note last recorded. The user can set a different number of days or turn the setting off. A Remove Audio action deletes the audio of one note at once. Any computer that uses the storage folder can remove old audio.

### Problem Frame

The audio is the largest part of a note, and the storage folder grows with each note. The audio is also the most private part: it holds each person's voice and every word, including words that the transcript left out or that the summary does not show. After a few weeks the transcript is the system of record, and it is very unlikely that a user runs Transcribe Again on a note that is 30 days old. An app that keeps all audio forever by default is not private by default.

### Key Decisions

- **On by default, 30 days.** The app is private by default; the user can keep audio longer or always. Governs R1, R2. (session-settled: user-directed — chosen over off by default: the default must be the private choice.)
- **Permanent delete, not the Trash.** Audio in the Trash is not gone. Governs R6. (session-settled: user-directed — chosen over the Trash and over a user choice: privacy wins over undo.)
- **Any computer removes old audio.** Old audio goes away even when the computer that recorded the note is off or no longer used. This is an exception to the library rule that an automatic step changes a home folder only for work that this computer recorded, so R9 guards it. Governs R8, R9. (session-settled: user-directed — chosen over only the recording computer and over a local setting.)
- **The delay starts at the last recording.** Audio lives a known number of days after it was made. Opening a note or running Transcribe Again does not reset it. Governs R4. (session-settled: user-approved — chosen over last opened and last processed.)
- **The setting is a shared setting.** When any computer can remove audio, one value must apply on every computer, or the computer with the shortest delay decides for all. Governs R1.
- **A note does not show that its audio is gone.** Most notes will have no audio after a time, so a mark on each note is noise. The fact shows only where it explains an unavailable action. Governs R13. (session-settled: user-directed — chosen over a visible "audio removed" mark on the note.)

### Requirements

**The setting**

- R1. The audio removal setting is a shared setting. It holds a delay in whole days, or off.
- R2. When the shared settings hold no value for it, the setting is on with a delay of 30 days.
- R3. The user sets the delay or turns the setting off in Settings > Storage.

**Which audio goes, and when**

- R4. The delay of a note starts when its last recording ended: the first Stop, or the last Stop after a Resume.
- R5. When the delay of a saved note has passed, Longhand removes all of the note's audio. The transcript, summary, notes and title stay.
- R6. Removal deletes the audio permanently. It does not move the audio to the Trash or the Recycle Bin.
- R7. Longhand removes audio only from a saved note. A note that records, waits for processing, processes, failed processing, or waits for a storage folder keeps its audio.
- R8. Any computer that uses the storage folder removes old audio, including the audio of notes that another computer recorded.
- R9. A computer never removes the audio of a note that another computer is recording into (Resume) or processing.
- R10. Longhand checks for old audio at launch and at intervals while it runs, so a computer that runs for many days still removes audio on time.
- R11. Removing audio does not download an online-only audio file from the sync service.

**Remove Audio on one note**

- R12. A saved note that has audio has a Remove Audio action. It asks for one confirmation, then removes the note's audio at once (R5, R6). It is available when the setting is off.

**A note with no audio**

- R13. A note whose audio was removed shows no mark or badge for that. Only where Transcribe Again or Resume is unavailable does the note say that its audio was removed.
- R14. On every computer, a note whose audio was removed is complete. It does not show as waiting for the sync to deliver its audio.

**Changes that remove audio now**

- R15. When a change of the setting would remove audio at once, a confirmation comes first. The changes are a shorter delay and turning the setting on again. The confirmation says that the removal is permanent and that those notes cannot use Transcribe Again or Resume.
- R16. The default (R2) removes audio with no confirmation.

### Acceptance Examples

- AE1. **Covers R4, R5.** Given a note that stopped on 1 September with the default delay, when Longhand runs on 1 October, the note's audio is gone and its transcript, summary, notes and title are unchanged.
- AE2. **Covers R4.** Given a note that stopped on 1 September and was resumed and stopped again on 20 September, with a 30-day delay, then on 1 October its audio is still there, and on 20 October it is gone.
- AE3. **Covers R8.** Given computers A and B share a storage folder, and A recorded a note 40 days ago and has been off since, when B runs, B removes the note's audio.
- AE4. **Covers R9.** Given the note of AE3 is old, and A resumes it and records into it, then B does not remove its audio while A records or processes it.
- AE5. **Covers R14.** Given B removed the audio of a note that A recorded, when A scans the library, A shows the note as complete, not as waiting for its audio.
- AE6. **Covers R13.** Given a note whose audio was removed, the note page and Home look the same as for a note with audio. When the user opens the note's menu, Transcribe Again is unavailable and says that the note's audio was removed.
- AE7. **Covers R15.** Given a delay of 90 days and notes that are 40 days old, when the user sets the delay to 30 days, a confirmation appears before any audio goes. If the user cancels, the delay stays at 90 days.
- AE8. **Covers R12.** Given the setting is off, when the user chooses Remove Audio on a note and confirms, the note's audio is gone at once.

### Scope Boundaries

- No "keep audio" exemption for one note.
- No display of the disk space that the setting would free, and no display of the disk use of notes.
- Copies that the sync service keeps, such as its recycle bin or version history, are out of Longhand's control.
- Delete Note does not change: it still moves the whole home folder to the Trash.
- A note with no audio cannot be resumed. Resume that starts new audio in a note whose audio was removed is not in scope.

### Dependencies / Assumptions

- After a complete save, Longhand's own storage keeps no copy of a note's audio: the save removes the working copy (`crates/pipeline/src/save.rs:524-529`). So removal from the home folder is enough. An interrupted or unsaved session keeps its working copy, and R7 keeps its audio.
- A partial sync can show a note on another computer with its audio gone before that computer learns that the removal was intentional. A short time in which the note shows as waiting is acceptable; R14 must hold when the sync is complete.
- A Move to a new storage folder copies notes with no audio correctly. Planning verifies this.

### Outstanding Questions

**Deferred to Planning**

- How another computer knows that a note's audio was removed on purpose (R14), since the library scan treats missing audio as a part that the sync has not delivered yet.
- How a computer knows that another computer is recording into or processing a note (R9). No record today says which computer recorded a session.
- The control for the delay in Settings > Storage (R3): a number field, a list of choices, or both. Use the `apple-design` skill.
- The UI text of the confirmations (R12, R15) and of the explanation on Transcribe Again and Resume (R13).

### Sources / Research

- `crates/pipeline/src/save.rs:110-119` — `home_intact`: Transcribe Again and Resume need every segment's audio.
- `crates/pipeline/src/reprocess.rs:317-318` — Transcribe Again refuses with "The note's folder does not have all of its audio".
- `src-tauri/src/managers/capture.rs:1356` — Resume checks `audio_intact`.
- `crates/library/src/scan.rs:462-467` — the scan adds `Waiting::Audio` for each missing segment audio file; it checks only that the file exists.
- `crates/session-core/src/folder.rs:104-150` — `session.md` records each segment's audio file and channels; it has no computer ID.
- `crates/shared-settings/src/file.rs` — the shared settings file and one `KEY_*` for each field.
- `src-tauri/src/commands/session.rs:666-731` — Delete Note: Trash, then a permanent delete.
- `src/settings/DestinationPicker.tsx` — Settings > Storage.
- `CONCEPTS.md` — Library (the rule for automatic steps), Shared settings, Transcribe Again.
