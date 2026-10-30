---
title: Remove the Transcripts of Old Notes - Plan
type: feat
date: 2026-10-30
topic: remove-old-transcripts
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Remove the Transcripts of Old Notes - Plan

## Goal Capsule

- **Objective:** A user can choose that old notes keep only their summary, manual notes, title and tags. The full transcript of a note does not stay on disk longer than the user wants.
- **Means:** A second removal setting and a Remove Transcript action, built on the mechanisms of audio removal (`docs/plans/2026-09-30-0357-feat-remove-old-audio-plan.md`).
- **Product authority:** The Product Contract wins on behavior. Audio removal behavior does not change, except where R4 links the two settings. Source idea: `docs/ideas/next/remove-old-transcripts.md`.
- **Open blockers:** None.

---

## Product Contract

### Summary

A second shared setting in Settings > Storage, off by default, permanently deletes the transcript of each saved note a set number of days after the note last recorded. Its delay is never shorter than the audio delay, so a note never keeps audio after its transcript is gone. A Remove Transcript action removes the transcript of one note, and any audio that is left, at once. Any computer that uses the storage folder can remove old transcripts.

### Problem Frame

Audio removal deletes the most private part of a note after 30 days by default. The transcript stays forever. A transcript holds every word that each person said, including words that the summary leaves out, so it is almost as private as the audio. Some users want old meetings to keep only what they need later: the summary and their own notes. Today the only way to remove a transcript is to edit `transcript.md` by hand or delete the whole note.

### Key Decisions

- **The transcript delay is never shorter than the audio delay.** A note with audio and no transcript can make the transcript again with Transcribe Again, so the transcript would not really be gone. Governs R4, R5. (session-settled: user-directed — chosen over "removing the transcript also removes the audio" and over two independent settings: the rule keeps the two settings in a fixed order.)
- **Off by default.** Current users keep every transcript after the update. The transcript stays the record until the user chooses otherwise. Governs R2. (session-settled: user-approved — chosen over on by default at a long delay and over on for new installs only: no surprise loss for current users.)
- **Remove Transcript also removes audio that is left.** One action takes a note down to its summary, manual notes, title and tags, and it keeps the order of R4 for one note. Governs R17. (session-settled: user-approved — chosen over an action that is available only after Remove Audio, and over no per-note action.)
- **Copy audio removal for everything else.** Permanent delete, the delay from the last recording, any computer removes, the in-use guard, no removal while the shared settings cannot be read, and a confirmation when a change removes data at once. Users learn one model, and the plan reuses tested mechanisms. Governs R6–R13, R19, R20.
- **A note shows no mark that its transcript is gone.** The audio decision applies here too: most old notes will have no transcript, so a mark is noise. The fact shows only where it explains an unavailable action. Governs R15.

### Requirements

**The setting**

- R1. The transcript removal setting is a shared setting. It holds a delay in whole days, or off.
- R2. The default of the setting is off. It follows the same copy rules as every other shared setting. While the shared settings cannot be read, no transcript is removed automatically.
- R3. The user sets the delay or turns the setting off in Settings > Storage, next to the audio removal setting.
- R4. The transcript delay is never shorter than the audio delay. The transcript setting cannot be on while audio removal is off.
- R5. Changes to the audio setting keep R4 true without a confirmation. When audio removal is turned off, transcript removal turns off. When the audio delay becomes longer than the transcript delay, the transcript delay becomes equal to the audio delay. The settings page shows the new transcript value.

**Which transcripts go, and when**

- R6. The delay of a note starts when its last recording ended, as for audio removal.
- R7. When the delay of a saved note has passed, Longhand removes the note's transcript: the transcript text and every other copy of the transcript text or timed recognition data that Longhand keeps for the note. The summary, manual notes, title and tags stay.
- R8. Removal deletes the transcript permanently. It does not move it to the Trash or the Recycle Bin.
- R9. A transcript that a person or an agent edited by hand is removed like any other.
- R10. Longhand removes transcripts only from a saved note, by the same rule as audio removal.
- R11. Any computer that uses the storage folder removes old transcripts, including those of notes that another computer recorded. The in-use guard of audio removal applies.
- R12. Longhand checks for old transcripts when it checks for old audio.
- R13. Removal never makes the transcript come back on another computer, and it never makes the note show as waiting for sync when the sync is complete.

**A note with no transcript**

- R14. A note whose transcript was removed cannot use Regenerate Summary, Transcribe Again or Resume.
- R15. A note whose transcript was removed shows no mark or badge for that, and no empty Transcript tab. Only where an action of R14 is unavailable does the note say that its transcript was removed.
- R16. A note whose transcript was removed is complete on every computer.

**Remove Transcript on one note**

- R17. A saved note that has a transcript has a Remove Transcript action. It asks for one confirmation, then removes the transcript at once (R7, R8). It is available whether the setting is on or off.
- R18. When the note still has audio, Remove Transcript also removes the audio, and the confirmation says so.

**Changes that remove transcripts now**

- R19. When a change of the transcript setting would remove transcripts at once, a confirmation comes first. The changes are a shorter delay and turning the setting on. The confirmation says that the removal is permanent and that those notes cannot use Regenerate Summary, Transcribe Again or Resume.
- R20. A change that R5 makes never removes a transcript, because it only makes the delay longer or turns the setting off.

**Agent access**

- R21. The Longhand notes agent skill knows that a saved note can have no `transcript.md`, and it does not treat that as an error or create the file.

### Acceptance Examples

- AE1. **Covers R6, R7.** Given audio removal at 30 days and transcript removal at 90 days, and a note that stopped on 1 June, then on 1 August the note has its transcript and no audio, and on 1 September it has only its summary, manual notes, title and tags.
- AE2. **Covers R4, R5.** Given audio at 30 days and transcripts at 60 days, when the user sets audio to 90 days, the transcript delay becomes 90 days and no confirmation appears.
- AE3. **Covers R4, R5.** Given audio at 30 days and transcripts at 60 days, when the user turns audio removal off, transcript removal is off too.
- AE4. **Covers R4.** Given audio removal is off, the transcript setting cannot be turned on until audio removal is on.
- AE5. **Covers R19.** Given transcripts at 365 days and notes that are 100 days old, when the user sets the transcript delay to 90 days, a confirmation appears before any transcript goes. If the user cancels, the delay stays at 365 days.
- AE6. **Covers R17, R18.** Given the setting is off and a note that is 2 days old with audio, when the user chooses Remove Transcript and confirms, the transcript and the audio are gone at once, and the summary, manual notes, title and tags stay.
- AE7. **Covers R14, R15.** Given a note whose transcript was removed, the note page shows the summary and manual notes with no Transcript tab and no mark. When the user opens the note's menu, Regenerate Summary says that the note's transcript was removed.
- AE8. **Covers R11, R13, R16.** Given computers A and B share a storage folder, when B removes the transcript of a note that A recorded, A shows the note as complete, without a transcript, and does not write the transcript again.
- AE9. **Covers R2.** Given a storage folder from before this feature, after the update every transcript stays until the user turns the setting on.

### Scope Boundaries

- No "keep transcript" exemption for one note.
- No display of the disk space that the setting would free.
- No change to the summary: a summary that quotes the transcript keeps its quotes.
- No change to audio removal, except R5.
- Copies that the sync service keeps, such as its recycle bin or version history, are out of Longhand's control.
- Delete Note does not change.

### Dependencies / Assumptions

- Audio removal (`docs/ideas/done/remove-old-audio.md`) shipped, with its in-use guard, removal markers and removal pass. This work extends them.
- A partial sync can show a note on another computer with its transcript gone before that computer learns that the removal was intentional. A short waiting time is acceptable; R13 and R16 must hold when the sync is complete.
- Today a missing `transcript.md` in a saved note makes the note wait for sync for about 60 seconds, then counts as a hand edit (`crates/library/src/scan.rs`, `crates/library/src/reconcile.rs` `HAND_EDIT_WAIT`). Regenerate Summary then fails with "The transcript is missing." (`crates/pipeline/src/save.rs` `summarize_again`). R14, R15 and R16 replace that path for a removed transcript.
- No feature searches transcript text today, so removal breaks no search.

### Outstanding Questions

**Deferred to Planning**

- How another computer tells an intentional removal of `transcript.md` from a partial sync or a hand delete. The audio removal marker is the pattern to consider.
- Which stored copies R7 covers. Known copies: `recognition.jsonl` (today removed only with the last audio); the `recognition_snapshot`, `recognition_artifact`, `recognition_audio` and `found_speakers` fields of `longhand.json`; the transcript fingerprints in `longhand.json`, `session.md` and the store; and, in the local store, the `recognition_output` and `recognition_epochs` rows, the Transcribe, Diarize and Assemble checkpoints, and `sessions.recognition_snapshot`.
- The controls for the two linked delays in Settings > Storage (R3–R5). Use the `apple-design` skill.
- The UI text of the confirmations (R17, R18, R19) and of the explanation on the unavailable actions (R15).
- What a user hand delete of `transcript.md` means after this feature: a removal, or a hand edit as today.

### Sources / Research

- `docs/plans/2026-09-30-0357-feat-remove-old-audio-plan.md` — the pattern: removal markers, the in-use marker, one pass on any computer, the confirmation of its R17.
- `crates/pipeline/src/remove_audio.rs`, `src-tauri/src/managers/audio_removal.rs` — the audio removal step and pass to extend.
- `crates/shared-settings/src/file.rs` — `KEY_AUDIO_REMOVAL`, `DEFAULT_AUDIO_REMOVAL_DAYS` (1 to 3650 days, or off).
- `src/settings/AudioRemoval.tsx`, rendered in the Storage pane of `src/settings/DestinationPicker.tsx`; `src/session/RemoveAudio.tsx` — the audio setting, the Remove Audio action and its confirmation.
- `crates/session-core/src/note_record.rs` — the fields of `longhand.json`.
- `CONCEPTS.md` — Remove Audio, Note record file, the waiting state for a fingerprint mismatch, Regenerate Summary, Transcribe Again.
- `plugins/longhand/skills/longhand-notes/SKILL.md`, `references/files.md` — list `transcript.md` as an editable file and do not say that it can be missing.
