---
title: Start a New Note Without Recording - Plan
type: feat
date: 2026-09-30
topic: new-note-without-recording
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Start a New Note Without Recording - Plan

## Goal Capsule

- **Objective:** A user can open a note before a meeting, type into it, and start to record at the right moment, or keep a note with no recording at all. The notes of every new note are on the user's other computers from the start.
- **Means:** Each new note gets its home folder at once. A Recording setting and a split New Note button choose whether a new note records. One Record/Resume button on the note page records into a note.
- **Product authority:** The Product Contract wins on behavior. Source idea: `docs/ideas/next/new-note-without-recording.md`.
- **Open blockers:** None.

---

## Product Contract

### Summary

Every new note gets its home folder in the storage folder at once, so its notes sync to the user's other computers while it records. New Note can also open a note that does not record. A Recording setting sets the default, the New Note split button gives the other choice, and a button on the note page records into any note that can record. The button says "Record" on a note with no audio and "Resume" on a note that has audio.

### Problem Frame

Today New Note starts to record at once, and nothing of the note is in the storage folder until processing ends and Save moves the complete note there. This causes three problems. A user cannot open a note early to type an agenda and then record at the right moment. A user cannot keep notes about a sensitive topic without a recording. And the notes of a note that records are only on one computer for the full length of the meeting and its processing, which can be hours for a long session.

### Key Decisions

- **Every new note gets its home folder at once, not only a note that does not record.** One save point for all notes, and the notes of a recording note sync early. Governs R1, R2, R3. (session-settled: user-directed — chosen over early folders only for notes that do not record: the user wanted one rule for all notes.)
- **An early folder, with the audio kept local until the result is saved.** Recording keeps today's start speed and crash safety. The result goes into the existing folder with the Publication rules. Governs R1, R3, R4. (session-settled: user-approved — chosen over "every new note is an empty saved note and every recording is a Resume": that path made the start of each recording depend on the storage folder. R7 later made New Note need the folder, but the early folder still keeps the folder write off the start of the recording.)
- **New Note needs an available storage folder.** Governs R7. (session-settled: user-directed — chosen over recording without a storage folder and writing the folder later: simpler.)
- **A note that never records gets no summary.** It stays a notes-only note. Governs R14. (session-settled: user-directed — chosen over a summary from the typed notes and over a summary on request.)
- **One button records into any note that can record.** It replaces the "Resume Recording" menu item, so there is one way to record more. Its label says what it does. Governs R11, R12. (session-settled: user-directed — chosen over a button only on notes with no audio; the user added the Record/Resume label.)
- **The recording shortcut follows the setting.** The shortcut and a click on New Note give the same result. Governs R10. (session-settled: user-directed — chosen over a shortcut that always records.)
- **Other computers can view and edit the notes of a note that records elsewhere.** Governs R20, R21. (session-settled: user-approved — chosen over view only and over hidden until done.)
- **An empty note that never recorded is removed.** A mis-click leaves nothing in the storage folder or on Home. Governs R18, R19. (session-settled: user-approved — chosen over keeping it.)
- **An explicit flag marks a note that has no result yet.** Missing files mean "waiting for a sync" today, so the state must not be inferred from them. Governs R6. (session-settled: user-directed — chosen over inferring the state from missing files: no ambiguity.)
- **The Recording setting is local to each computer.** It sits with the other new-note defaults (microphone, Computer audio), which are local. Governs R8.

### Requirements

**Early home folder**

- R1. When a note starts, recording or not, Longhand writes its home folder in the storage folder with `session.md` and `notes.md`. The start of the recording never waits for this write.
- R2. While a note records and processes, each notes save on this computer writes `notes.md` in the home folder, as a saved note does today.
- R3. The audio and the processing files stay in the working location until the result is complete. The result then goes into the note's existing home folder with the Publication rules (`session.md` last). The note keeps this one home folder for life, also when processing later proposes a title and the folder is renamed.
- R4. After a crash at any point, the note has at most one home folder, and its recording recovers as it does today.
- R5. When the storage folder becomes unavailable during a recording, the recording continues. The notes writes and the result wait until the folder is available, as Save waits today. This includes the first write of the home folder (R1): it tries again when the folder is back, and Save waits for that one folder and never makes a second one.
- R6. `session.md` of a note that has no result yet states this explicitly, with its own flag, from the first write of the home folder until the first result is saved. Every computer reads the flag and does not infer the state from missing files. A note with this flag is not waiting for a sync: a missing transcript, summary or audio does not hold its notes editor or its Record button.

**Starting a note**

- R7. While the storage folder is unavailable, New Note (both choices) and the recording shortcut do not start a note. New Note shows why.
- R8. Settings > Recording has a setting for what New Note does: start to record (the default for a new install, as today) or open a note that does not record.
- R9. New Note is a split button. A click does the default of R8. Its menu gives the other choice.
- R10. The recording shortcut stops a recording when one runs. Otherwise, when the default is "does not record" and the main window shows a note whose button says "Record" and is available (R11, R12), it records into that note, also when another app has the focus. Otherwise it does the default of R8.

**Record and Resume on the note page**

- R11. The note page shows one button when the note does not record and can record: "Record" on a note with no audio, "Resume" on a note that has audio. It replaces the "Resume Recording" menu item.
- R12. The button is unavailable, with the reason, in each case where Resume is unavailable today (audio removed, waiting for a sync of a part the note has, storage folder unavailable), and while another computer records into or processes the note.
- R13. The button and the start of a recording follow the U7 attention budget of the recording surface: no motion and no ornament that pulls the eye.

**A note with no audio**

- R14. A note that has never recorded has no transcript and no summary. The note page shows its notes and the Record button.
- R15. Home lists a note that has never recorded and does not record or process now like other notes, with its title or its start time, and shows no length and no processing status for it.
- R16. A note with no audio keeps its start time as its title until the user renames it. Processing proposes a title only after the note records, and never replaces a title that the user set.
- R17. The first recording into a note with no audio makes a transcript and summary through the usual processing, with the notes that the user typed before and during the recording.

**Empty notes and discarded recordings**

- R18. A note with no audio, no text in its notes and no title from the user is removed, with its home folder, when the user leaves it. The computer that started the note removes it; when the app closed first, it removes it at the next launch. It removes the note only when the storage folder is available and `notes.md` in the home folder is still the file that this computer last wrote. Otherwise it keeps the note, because another computer can have typed into it.
- R19. When a recording is discarded automatically because every segment is shorter than one second, the note stays as a note with no audio if its notes have text, from this computer or another one. Otherwise R18 removes it.

**Other computers**

- R20. Another computer lists a note that records or processes elsewhere, shows that it records or processes on another computer, shows its notes, and lets the user edit them. Notes edits follow the current rules: the last change wins, and a conflict copy keeps an edit that would be lost.
- R21. On that computer, Record, Resume, Transcribe Again, Regenerate Summary, Remove Audio and Delete wait until the other computer is done with the note.
- R22. On the recording computer, notes edits that arrive from another computer show in My Notes while the note records.

### Key Flows

- F1. Open the note early, then record
  - **Trigger:** The user sets the default to "does not record" and clicks New Note, or picks the other choice from the New Note menu.
  - **Steps:** The note opens with no recording and its home folder is written. The user types an agenda. At the start of the meeting, the user clicks Record or presses the recording shortcut. The note records, the user stops it, and processing saves the transcript and summary into the same home folder.
  - **Covered by:** R1, R9, R10, R11, R17
- F2. Notes only
  - **Trigger:** The user opens a note that does not record for a sensitive topic.
  - **Steps:** The user types notes and goes back to Home. The note stays in the list with its start time as its title, and has no transcript or summary.
  - **Covered by:** R14, R15, R16
- F3. Notes from another computer during a recording
  - **Trigger:** Computer A records a note.
  - **Steps:** Computer B lists the note as recording on another computer. The user types into its notes on B. The edit shows on A. Actions that need the audio wait on B. When A saves the result, B shows the transcript and summary.
  - **Covered by:** R20, R21, R22

### Acceptance Examples

- AE1. **Covers R10.** Given the default is "does not record" and the main window shows a note with no audio, when the user presses the recording shortcut, the note records. When the user presses it again, the recording stops.
- AE2. **Covers R10.** Given the default is "starts to record" and the main window shows a note with no audio, when the user presses the recording shortcut, a new note starts to record.
- AE3. **Covers R11, R12.** Given a saved note whose audio was removed, the note page shows "Resume", unavailable, with the reason that the audio was removed.
- AE4. **Covers R18.** Given the user clicks New Note with "does not record" and goes back to Home without typing, the note is not on Home and its home folder is gone.
- AE5. **Covers R18.** Given the user types only in the title of a note with no audio, when the user leaves it, the note stays.
- AE6. **Covers R19.** Given a recording note with typed notes whose every segment is shorter than one second, when the user stops it, the note stays as a note with no audio and keeps its notes.
- AE7. **Covers R5.** Given a recording and a storage folder on a network volume that disconnects, the recording continues, and the notes and the result are written when the volume is back.
- AE8. **Covers R7.** Given the storage folder is unavailable, New Note and its menu are unavailable with the reason, and the recording shortcut starts nothing.
- AE9. **Covers R10.** Given the default is "does not record", the main window shows a note with no audio, and another app has the focus, when the user presses the recording shortcut, that note records.
- AE10. **Covers R10, R12.** Given the default is "does not record" and the main window shows a note with no audio that another computer records into, when the user presses the recording shortcut, no recording starts into that note, and the shortcut does the default of R8.
- AE11. **Covers R18.** Given computer A opened a note with no audio, computer B typed into it, and B's edit has synced to A, when A launches, A keeps the note.

### Scope Boundaries

- A summary made from typed notes only.
- Recording into a note whose audio was removed. The button is unavailable there (AE3). See `docs/ideas/someday/resume-survives-early-audio-removal.md` for the related Resume gap.
- A change to how Delete works: it stays unavailable while a note records or processes.

### Dependencies / Assumptions

- The Publication rules (`CONCEPTS.md`, Publication) already write a new result into an existing home folder for Resume and Transcribe Again; R3 assumes Save can use the same rules.
- The in-use marker of audio removal (`CONCEPTS.md`, Audio removal) already tells other computers that a note records or processes; R20 and R21 may use it.
- `CONCEPTS.md` must be updated with the change: Session (a session can have no recording), Save (the result goes into an existing home folder), Home folder and Staging.

### Outstanding Questions

**Deferred to Planning**

- The form of the R6 flag in `session.md`, how `session.md` of an early folder shows that the note records or processes, and how another computer tells a stale state after a crash on the recording computer from a live one.
- How the note page of a note with no audio shows the Transcript and Summary tabs: hidden, or present with a short message.
- The exact labels of the R8 setting and of the New Note menu item, and whether the Settings name "recording shortcut" changes now that the shortcut can open a note that does not record.
- What computer B does when it saves an edit to a note whose home folder computer A removed under R18 before B's edit synced, so that B's edit is not lost.
- The anarlog Record button in `reference-source/anarlog/` must be read before the button design, with the `apple-design` skill.

### Sources / Research

- `src/home/Home.tsx:179` — New Note calls `startRecording(null)`.
- `crates/pipeline/src/save.rs:706-735` — the render step writes `notes.md` and `session.md` in the working location before Save.
- `crates/pipeline/src/save.rs:904-907` and `:971-1017` — a saved note writes `notes.md` into its home folder on each notes save.
- `src/session/SessionPage.tsx:403-409` — the "Resume Recording" menu item and its unavailable cases.
- `src-tauri/src/shortcut.rs:346-382` — the recording shortcut stops a recording or starts a new one.
- `src-tauri/src/managers/settings.rs:220-238` and `src/settings/Recording.tsx` — the local new-note defaults.
- `crates/session-core/src/state.rs:65-68` — the AwaitingDestination state.
- `crates/session-save/src/publication.rs:1-31` — the Publication write order.
- `crates/pipeline/src/in_use.rs:1-17` — the in-use marker.
- `src/session/RecordingBar.tsx:1-9` — the U7 two-tier bar and its no-motion rule.
