---
title: Pause Recording - Plan
type: feat
date: 2026-10-09
topic: pause-recording
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Pause Recording - Plan

## Goal Capsule

- **Objective:** A user can leave a part of a meeting out of a note (a break, or a sensitive discussion) without stopping the note. The note continues after that part, and processing starts only at Stop.
- **Product authority:** This Product Contract. The always-visible recording widget is a separate idea (`docs/ideas/someday/recording-state-widget.md`) and is not active scope.
- **Open blockers:** None.

---

## Product Contract

### Summary

A Pause action, separate from Stop. Pause closes the microphone and Computer audio, so Longhand captures and saves nothing until the user continues the note. The paused note stays an open note on the recording bar, on Home and in the tray, which all say "Paused". The finished transcript and the summary input mark each pause and its duration.

### Problem Frame

Long meetings often have a part that should not be in the note: a break with casual talk, or a discussion that is too sensitive to record. Today the only way to leave it out is Stop and then Resume. That works, but it is the wrong action for the user's intent. Stop means "I'm done", and a Resume after it feels like a correction of a mistake. Stop also starts processing in the middle of the meeting, and the Resume then makes the result again.

### Key Decisions

- **Pause is its own action, separate from Stop.** Governs R1, R4. (session-settled: user-directed — chosen over Stop then Resume: Stop means the meeting is over, and a pause is not that.)
- **A pause captures nothing.** The microphone and Computer audio close, so the macOS microphone indicator turns off and no paused audio exists on disk. This is necessary for the sensitive-discussion case. Governs R2, R6.
- **No reminder for a forgotten pause in this plan.** The always-visible widget idea covers a forgotten pause and a recording left running after a meeting. Governs nothing here. (session-settled: user-directed — chosen over a reminder that listens for speech while paused: the user prefers one widget that solves both problems.)
- **The pause marker goes to the summary model.** The model then treats the parts before and after a pause as separate parts of the meeting. Governs R13. (session-settled: user-approved — chosen over no trace, and over a marker in the transcript only: without it, the summary can join two unrelated discussions.)
- **The Pause shortcut has no default chord.** Governs R16. (session-settled: user-directed — chosen over a default chord: the user sets it, as for Show Longhand.)
- **The un-pause action is not called "Resume".** In Longhand, Resume means recording into a stopped or saved note. The un-pause label is "Continue". Governs R8.

### Requirements

**Pause and continue**

- R1. While a note records, the user can pause it from the recording bar, and continue it from the same place.
- R2. While paused, the microphone and Computer audio are closed, and Longhand writes no audio to disk, including the crash-recovery data.
- R3. Continue records into the same note. The microphone choice and the Computer audio switch keep the values they had before the pause.
- R4. Stop works while paused. Processing then runs over the audio recorded before the pause, as after a usual Stop.
- R5. Pause is available only while the note records. It is not available while a capture is interrupted, or for a stopped or saved note.
- R6. If Longhand stops unexpectedly while paused, recovery treats the note as stopped at the start of the pause.
- R7. A pause has no maximum length.

**Paused state on screen**

- R8. While paused, the recording bar says "Paused", the elapsed time stops, and the Pause control becomes "Continue". Nothing on the bar moves or flashes (U7 attention budget).
- R9. While paused, Home keeps the note as the open-note row. The row says "Paused", its time stops, and its icon is neutral grey. Recording stays red and interrupted stays the caution colour.
- R10. While paused, the tray title says "Paused" in place of the recording time.
- R11. A paused note is an open note. New Note stays unavailable until Stop, as during recording.
- R12. While paused, the live transcript shows that the note is paused. It does not show the message for a live transcript that fell behind ("The missing part is transcribed after you stop").

**Transcript and summary**

- R13. The finished transcript shows a marker at each pause, with the pause duration, for example "Paused · 14 min". The summary model receives the same marker at the same position.
- R14. The pause marker is different from the marker for time between a Stop and a Resume ("Not recorded for …").
- R15. Processing does not treat a pause as missing audio. Gap fill does not try to transcribe it.

**Shortcuts**

- R16. Settings has a Pause shortcut with no default chord. It pauses a recording note and continues a paused note. At other times it does nothing.
- R17. The Recording shortcut stops a paused note, as it stops a recording note.

### Acceptance Examples

- AE1. **Covers R1, R2, R13.** Given a note that records for 50 min, when the user pauses for 15 min and then continues for 40 min, then the saved audio is 90 min long, and the transcript has one "Paused · 15 min" marker between the two parts.
- AE2. **Covers R4, R13.** Given a paused note, when the user selects Stop, then processing starts at once over the recorded audio, and the transcript ends with no pause marker.
- AE3. **Covers R6.** Given a paused note, when Longhand quits unexpectedly and the user opens it again, then the note recovers with the audio recorded before the pause and nothing after it.
- AE4. **Covers R3.** Given a note that records with Computer audio on, when the user pauses and continues, then Computer audio is on again.
- AE5. **Covers R16, R17.** Given a paused note with Longhand in the background, when the user presses the Pause shortcut, then the note continues. When the user presses the Recording shortcut, then the note stops.
- AE6. **Covers R14.** Given a saved note with one pause, when the user stops it, resumes it later and stops it again, then the transcript shows a pause marker and a "Not recorded for …" marker, and the two read differently.

### Scope Boundaries

- The always-visible recording widget, for a forgotten pause or a forgotten Stop (`docs/ideas/someday/recording-state-widget.md`).
- Any reminder while paused, by time or by speech.
- Automatic pause on silence.
- Removing a part of a recording after the fact, for when the user did not pause before a sensitive part.
- A default chord for the Pause shortcut.

### Outstanding Questions

**Deferred to Planning**

- How the session data records a pause, so that processing, recovery and other computers can tell it from a Stop/Resume gap (R6, R14, R15).
- The exact marker text in `transcript.md` and in the summary input (R13).
- Whether the Pause control also goes in an app menu or the tray menu, in addition to the recording bar (R1).
- How the live transcript shows the paused state (R12).

### Sources / Research

- Segments and gaps: a segment boundary at each input change is in `src-tauri/src/managers/capture.rs` (lines 20-22). The gap between segments comes from segment offsets in `crates/pipeline/src/join.rs` (lines 76-80).
- Elapsed time excludes the time between segments: `recorded_ms` in `src-tauri/src/managers/capture.rs` (lines 2893-3000).
- The transcript gap line and its view: `crates/session-core/src/folder.rs` (line 506) and `src/session/TranscriptTab.tsx` (lines 49-53).
- The summary input keeps only turns and drops gap lines today: `crates/pipeline/src/summarize.rs` (lines 270-292).
- Recovery chunks are written continuously: `crates/recorder/src/chunks.rs`.
- The microphone stream closes at each segment end: `src-tauri/src/managers/capture.rs` (lines 2847-2885).
- The open-note row on Home: `src/home/RecordingRow.tsx`. The tray title: `src-tauri/src/tray.rs`.
- The Recording shortcut action: `press_action` in `src-tauri/src/shortcut.rs` (lines 373-391).
- Resume of a stopped or saved note: `resume_session` in `src-tauri/src/managers/capture.rs` (lines 1593-1691).
