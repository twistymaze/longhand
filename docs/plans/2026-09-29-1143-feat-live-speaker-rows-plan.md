---
title: Live Speaker Rows - Plan
type: feat
date: 2026-09-29
topic: live-speaker-rows
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Live Speaker Rows - Plan

## Goal Capsule

- **Objective:** During recording, a user who opens the Transcript tab can follow the back-and-forth of the conversation, because the live text shows as speaker rows and readable paragraphs, not as one large block.
- **Means:** The live events give the UI the word times that the live recognizer has. The UI matches the words to the live speaker turns and groups them with the final transcript's paragraph rule.
- **Product authority:** This plan. The live speaker labels are best effort. The final transcript after Stop stays the canonical result.
- **Open blockers:** None.

---

## Product Contract

### Summary

During recording, the Transcript tab shows the live text in the layout of the final transcript: a speaker heading, and that speaker's paragraphs under it.
Long speech from one speaker breaks into paragraphs, with or without speaker analysis.
The processing pipeline, the models, and the result after Stop do not change.

### Problem Frame

With the recommended route (a streaming transcription model and a separate speaker model), the live text shows with no labels as one large block.
The live speaker turns show only in a collapsed "Speaker analysis so far" list, as times, and the user must match them to the text.
A 2-person exchange and a long monologue look the same.
The data to do better is already in the app: the live speaker worker sends provisional speaker turns, and the live recognizer has word times.
The UI does not get the word times.

### Key Decisions

- **Give the UI more of the existing data. Do not change the processing pipeline.** The UI does the join for display only. Governs R1, R2. (session-settled: user-directed — chosen over a join in the backend and over a UI-only estimate of word times: the user wants no change to the pipeline approach but accepts more data sent to the UI.)
- **Give word times for the open part too.** The open part is the newest 4 to 8 minutes of text, and it has no word times today. Without them, the speaker rows would lag 4 to 8 minutes behind the speech. Governs R2, R3, R3a. (session-settled: user-directed — chosen over speaker rows for saved parts only: rows that lag 4 to 8 minutes do not show the back-and-forth.)
- **Numbers restart at each speaker analysis window, with a divider.** The live speaker model restarts its labels at each new window: at the start of each capture segment, and after each 20 minutes of a segment. Nothing links one window to the next. Governs R7, R8. (session-settled: user-directed — chosen over new numbers for each window and over restarting with no marker: fewer numbers, and the divider tells the reader that the numbers restart.)
- **The newest text continues the last speaker.** When its speaker turn arrives and shows a change, the app inserts the new heading at that point. Governs R6. (session-settled: user-directed — chosen over a block with no heading and over a "Speaker not known yet" heading, after a sketch of all three: it reads as a continuous conversation, and a heading appears when the change is known.)
- **A speaker number on the screen does not change in its window.** Governs R7. (session-settled: user-directed — chosen over strict renumbering by order of first speech: a heading that the reader already saw must not change to a different person; numbers can be out of order after a revision.)
- **Use the final transcript's paragraph rule.** One rule for live and final text keeps the two views alike. Governs R4.

### Requirements

**Data for the UI**

- R1. The live events give the UI the word times of each saved part and the start offset of its capture segment, in addition to its text.
- R2. The live events give the UI the word times of the open part as it grows, and the start offset of its capture segment, so the newest text can split by speaker.
- R3. When a streaming model cannot give word times for the open part, the open part shows as paragraphs under the last speaker, and it splits by speaker when the part saves.
- R3a. The UI matches the timed words of the open part to the displayed text by their text, not by their position. Displayed words that do not match a timed word stay under the speaker before them, and they split by speaker when the part saves.

**Rows and paragraphs**

- R4. The live text groups into paragraphs with the final transcript's rule: a new paragraph after a pause longer than 2 seconds, or at the first sentence end after 400 characters (`crates/pipeline/src/join.rs:31-34`, `:236`).
- R5. When live speaker turns are available, each change of speaker starts a new speaker row: a "Speaker N" heading with that speaker's paragraphs under it, in the layout of the final transcript's Transcript tab.
- R6. Text that no speaker turn covers yet continues the row of the last speaker. When a turn arrives that shows a different speaker, the app inserts the new heading at the first word of that turn and moves the words after it under the heading.
- R7. Speaker numbers count from 1 in each speaker analysis window, in order of first speech in that window. Once a speaker has a number in a window, the speaker keeps it: a revision can move or join headings, but it does not change a number already on the screen.
- R8. Where a new speaker analysis window starts (a change of the epoch in the live speaker events), a divider line with the text "Speaker labels restart" shows, in the style of the final transcript's markers.
- R9. When live speaker analysis is off or has no turns, the live text shows as paragraphs with no speaker headings.

**Changes and notices**

- R10. When a speaker turn is revised, the rows and headings change in place with no animation, so that the recording surface does not draw attention (U7). When a change is above the visible area, the row that the reader sees stays at the same place on the screen.
- R11. The one existing notice "Speaker labels may change" stays while any live label is provisional.
- R12. The collapsed "Speaker analysis so far" list does not show when the text shows speaker rows. It stays only when live speaker analysis runs without live text.
- R13. The tentative text of the open part keeps its present secondary style inside the rows.

### Acceptance Examples

- AE1. **Covers R5, R6.** **Given** Speaker 1's row is the last row and new text arrives with no speaker turn yet, **when** the text shows, **then** it continues Speaker 1's row. **When** the turn for that text arrives and names a different speaker, **then** a "Speaker 2" heading shows before the first word of the turn, and the words after it move under it.
- AE2. **Covers R7, R8.** **Given** a note at minute 20, **when** the second speaker analysis window starts, **then** a "Speaker labels restart" divider shows, and the first speaker after it is "Speaker 1", also if that person was "Speaker 2" before the divider. **When** the user changes the microphone at minute 25, **then** a new capture segment starts, a divider shows there, and numbering starts again at "Speaker 1".
- AE3. **Covers R4, R9.** **Given** one person speaks for 5 minutes with no pause longer than 2 seconds, **when** the text passes 400 characters and a sentence ends, **then** a new paragraph starts under the same heading (or with no heading when speaker analysis is off).
- AE4. **Covers R3.** **Given** a streaming model that gives no open-part word times, **when** the open part grows, **then** its text shows as paragraphs under the last speaker, and when the part saves, it splits into speaker rows.
- AE5. **Covers R10.** **Given** a revised speaker turn moves a speaker change 3 words later, **when** the revision arrives, **then** the heading moves with no animation, and the scroll position does not jump while the view follows new text. **When** the reader has scrolled up and the moved heading is above the visible area, **then** the text that the reader sees does not move.

### Scope Boundaries

- No change to the processing pipeline, the models, the speaker analysis windows, or the transcript after Stop.
- No link of speakers across windows. That needs validated evidence (`docs/ideas/next/live-speaker-labels.md`) and is a pipeline change.
- No measurement or improvement of live speaker accuracy.
- No change to when the Transcript tab opens during a recording. It still does not open by default.

### Dependencies / Assumptions

- Each streaming model has token times for the open part while it streams, and can give them without a change to how it recognizes speech. This is not verified. R3 is the fallback where it is false.
- The token times describe the model's current hypothesis. The displayed committed text is append-only, so it can differ from that hypothesis after the model revises a committed word (`transcribe.h`, transcribe-cpp-sys 0.2.4). R3a handles the difference. Such revisions are expected to be rare, because the library commits only a prefix that it judges stable. This is not measured.
- Speaker turns use samples at 16 kHz from the start of the note. Words use milliseconds from the start of their capture segment. With the segment start offset from R1 and R2, the UI converts word times to the note's timeline.

### Outstanding Questions

**Deferred to Planning**

- Which streaming models can give open-part word times, and how each exposes them.
- How often the timed hypothesis and the displayed committed text differ during a real note, and how the text match of R3a works.
- How the CombinedNemo route, which already shows live labels in the UI (`src/session/LiveTranscript.tsx`), fits the same rows so that one layout serves every route.
- Whether the paragraph rule moves to shared code or the UI copies it with a test that holds both copies to the same values.

### Sources / Research

- `src/session/LiveTranscript.tsx` — the present live view, the collapsed speaker list, and the notices.
- `src/session/TranscriptTab.tsx` — the final transcript layout (speaker heading, paragraphs, `Marker` divider).
- `src/session/liveRecognition.ts` — `recognitionParagraphs`, the present grouping of native timed rows.
- `crates/asr/src/stream.rs:36-59` — `LiveWord`, `LivePart` (saved part with words), `LiveText` (open part, text only).
- `crates/pipeline/src/live.rs:78-88` — `LiveRow` with words.
- `crates/pipeline/src/recognition/nemo.rs:17` — `MAX_EPOCH_SAMPLES`, the 20-minute window.
- `crates/pipeline/src/recognition.rs:284` — the note that labels restart at each window.
- `docs/ideas/next/live-speaker-labels.md` — the idea this plan comes from.
