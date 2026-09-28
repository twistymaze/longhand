---
title: Transcribe Again from Settings - Plan
type: feat
date: 2026-09-28
topic: transcribe-again-from-settings
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Transcribe Again from Settings - Plan

## Goal Capsule

- **Objective:** A user can transcribe a saved note again after one plain confirmation. The user does not choose models, read a technical explanation, or tick a checkbox. The run uses the models in Settings.
- **Product authority:** This Product Contract. The user confirmed the scope on 2026-09-28.
- **Open blockers:** None.

---

## Product Contract

### Summary

Transcribe Again becomes one step. The menu action opens a small confirmation dialog that fits the window. After the user confirms, the run uses the models and language in Settings and the speaker count of the note. The model choice step, the preview step, the reuse explanation and the consent checkbox go away.

### Problem Frame

Today the "Transcribe Again…" menu action opens a dialog that shows all the recognition controls, prefilled from Settings. The dialog is too tall for the app window, so part of it is not visible. After "Preview changes", the dialog shows a second page. When the chosen route cannot reuse the saved timed text, the page shows "This change requires transcription again. The note has no compatible verified timed text, or its transcript or audio changed." It also shows a checkbox that the user must tick before the "Reprocess note" button operates. The user saw this page when they tried the Granite model and did not understand the text. The user expects the app settings to decide the models, so the choice step has no function for them. The explanation and the checkbox make a routine action look dangerous.

### Key Decisions

- **Settings decides the models, the speaker source, the timing and the language.** Governs R1. (session-settled: user-directed — chosen over a per-run choice dialog: the function of the action is to use the app settings, not to ask the user for choices)
- **The speaker count of the note has priority over the Settings default.** The count describes that meeting, not a preference. Governs R2. (session-settled: user-directed — chosen over the Settings default speaker count only)
- **One plain confirmation replaces the preview and the checkbox.** Governs R4, R5. (session-settled: user-directed — chosen over the preview step with a required consent checkbox: that flow is too complex and too alarming for a routine action)
- **The tabs show progress during the run, as now.** The dialog text states this, and `CONCEPTS.md` changes to agree with it. Governs R8. (session-settled: user-directed — chosen over keeping the previous transcript and summary visible during the run)
- **To use a different model for one note, the user changes Settings first.** That change also applies to new notes. Governs R1. (session-settled: user-approved — the user accepted this trade-off at the scope check)

### Requirements

**Configuration of the run**

- R1. Transcribe Again uses the transcription model, speaker source, speaker timing and language that are in Settings when the user confirms. The flow shows no recognition controls.
- R2. The run uses the speaker count that is saved with the note. When the note has no speaker count, the run uses the Settings default speaker count.
- R3. When the saved timed text is compatible with the configuration, the run uses it again, as it does now. The UI does not mention this.

**Confirmation**

- R4. The menu action opens one dialog with the title "Transcribe this note again?". The body says that the transcript and summary are not available until the new ones are ready, and that the current ones come back if the run fails. The body also names the transcription model from Settings. The buttons are "Cancel" and "Transcribe Again".
- R5. The dialog has no checkbox, no preview step and no explanation of timed-text reuse.
- R6. All of the dialog, with both buttons, is visible and not clipped at every window size that the app permits.
- R7. When the Settings configuration cannot run on this note, the menu action shows the reason and tells the user to change it in Settings. It does not open the confirmation dialog. Two examples: the model is not downloaded, or the model does not support the language. Nothing goes into the queue.

**The run**

- R8. After confirmation, the run starts as it does now. The Transcript and Summary tabs show progress, and the recording, My Notes and the title stay the same. When the run fails, the note shows the current failure message and the previous transcript and summary.
- R9. The confirmation is the permission to transcribe again. A confirmed run is never refused because it needs transcription.

### Acceptance Examples

- AE1. A new model that cannot reuse text
  - **Covers R4, R5, R9.**
  - **Given:** The note was transcribed with Parakeet, and Settings now selects Granite.
  - **When:** The user selects "Transcribe Again…" and then "Transcribe Again" in the dialog.
  - **Then:** The run starts. The user saw no checkbox and no explanation of timed text.
- AE2. The speaker count of the note
  - **Covers R2.**
  - **Given:** The note has a speaker count of 3, and the Settings default is 2.
  - **When:** The user confirms Transcribe Again.
  - **Then:** The run uses 3. For a note with no speaker count, the run uses 2.
- AE3. The model is not downloaded
  - **Covers R7.**
  - **Given:** Settings selects a transcription model that is not downloaded.
  - **When:** The user selects "Transcribe Again…".
  - **Then:** The note shows the reason and tells the user to change it in Settings. No dialog opens, and nothing goes into the queue.
- AE4. Only the speaker source changed
  - **Covers R3, R4.**
  - **Given:** Only the speaker source in Settings is different, and the saved timed text is compatible.
  - **When:** The user confirms.
  - **Then:** The run uses the saved timed text again. The dialog had the same text as in AE1.
- AE5. The run fails
  - **Covers R8.**
  - **Given:** A confirmed run fails before it saves.
  - **Then:** The note shows "The new transcript failed. The previous transcript is unchanged." and the previous transcript and summary.

### Scope Boundaries

- There is no per-run model choice. There is also no "advanced" or "other models" path in the dialog.
- The previous transcript and summary are not visible during the run.
- The recognition controls in Settings do not change.
- The dialog does not download a missing model. The user goes to Settings (R7).

### Dependencies / Assumptions

- A note has no language field of its own, so the language always comes from Settings (R1). The only record of the previous language is in the recognition snapshot of the note.
- The `CONCEPTS.md` entry "Transcribe Again" changes to agree with this contract.

### Outstanding Questions

**Deferred to Planning**

- Does the frontend still send a preview to the backend, or does one command resolve the configuration from Settings and the note? Also decide how a note or Settings change between dialog open and confirmation is handled. The current backend text for that case says "review the replacement again", which has no meaning when there is no review step.
- Should the new dialog use the `RecognitionDialog` shell, or a different one? `RecognitionDialog` has no other use. Which window size is the smallest to test for R6?
- What is the exact wording of the R7 reason for each resolve error?

### Sources / Research

- `src/session/TranscribeAgain.tsx` — the current three-step flow: the recognition controls, then the preview, then the consent checkbox (lines 102-145).
- `src/session/RecognitionDialog.tsx:30-36` — a full-window overlay with `items-start` and `py-[8vh]`. The panel has no maximum height.
- `src/settings/RecognitionOptions.tsx:26-37` — `recognitionDefaults` takes the speaker count from the Settings default, not from the note.
- `crates/session-core/src/folder.rs:115` — `SessionDoc.speaker_count` holds the speaker count of the note.
- `crates/pipeline/src/reprocess.rs:29-42` — the preview and its two explanation texts.
- `crates/pipeline/src/runner.rs:489-530` — `reprocess` refuses a stale preview, and refuses the run when transcription is necessary and `allow_transcription` is false.
- `src/session/SessionPage.tsx:376-388` — the menu item is off only during regeneration or an audio hold. It does not check the Settings model.
- `src/session/SessionPage.tsx:420-448` — during a Transcribe Again run, both tabs show `ProcessingStatus`.
- `src/session/SessionPage.test.tsx:906-929` and `src/session/RecognitionChoices.test.tsx` — the current tests for the preview and consent flow. They must change.
