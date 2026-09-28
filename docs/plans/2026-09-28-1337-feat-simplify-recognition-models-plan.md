---
title: Simplify Recognition Models - Plan
type: feat
date: 2026-09-28
topic: simplify-recognition-models
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Simplify Recognition Models - Plan

## Goal Capsule

- **Objective:** A user chooses from a short list of transcription models, and every model on the list has a clear reason to be there. The recommended models are fast, live and accurate. Speaker labels come from one path for every model.
- **Product authority:** This Product Contract. The user confirmed the scope on 2026-09-28. The Transcribe Again changes are in `docs/plans/2026-09-28-1244-feat-transcribe-again-from-settings-plan.md` and are not active scope here.
- **Open blockers:** None.

---

## Product Contract

### Summary

The catalog keeps five transcription models: three live models and two models for After Stop only. The five other models go away. Every model uses the separate speaker model, and the model's own speaker labels go away. "Recommended" marks one model for English and one for other languages.

### Problem Frame

The catalog has ten transcription models. The six models that cannot run live do not give better transcripts than the live models:

- **Accuracy:** In the Handy assessments in the catalog, the best live model (Parakeet Unified, 90) is equal to Granite (90) and one point below Parakeet TDT 1.1B (91).
- **Speed:** Granite, MOSS and Whisper are about 2.5 to 3 times slower. A one-hour note with Granite and Nemotron 3 took more than 18 minutes after Stop on an M2 Ultra.
- **Context:** After Stop gives these models no more context than live transcription. Transcription after Stop sends windows of at most 30 s. A live model keeps its context for a live part of 4 to 8 minutes.

Granite, MOSS and Multitalker also offer their own speaker labels. These labels restart in each window: a five-minute test gave 30, 38 and 14 labels. The automatic choice therefore never uses them. Multitalker's transcription of overlapping speech into speaker channels needs this label mode too.

Granite is marked Recommended only because it offers its own labels. This sent the user to the slowest model.

### Key Decisions

- **Keep five models and remove five.** Governs R1, R2. (session-settled: user-directed — chosen over the full list of ten: Parakeet TDT v2 and 1.1B give almost nothing over live Parakeet Unified, and MOSS and Granite are slow and give no accuracy advantage)
- **Remove Multitalker.** Governs R2. (session-settled: user-approved — chosen over keeping it for crosstalk: its per-speaker transcription of overlapping speech needs the model-label mode, which this plan removes, and without that mode it is a plain English live model with a lower score than Parakeet Unified)
- **Keep Whisper and Parakeet TDT v3 for languages.** They cover languages that no live model covers, and they are more accurate than Nemotron 3.5 in the languages that they share. Governs R1. (session-settled: user-directed — chosen over a live-only list: no live model covers 73 of Whisper's languages or 5 of Parakeet TDT v3's languages)
- **Remove the model's own speaker labels completely.** They are not hidden under Advanced. Governs R6, R7. (session-settled: user-directed — chosen over keeping "Transcription model labels" for Multitalker: the labels restart in each window, and one speaker path is simpler)
- **Recommend one English model and one multilingual model.** Governs R4. (session-settled: user-directed — chosen over two English live models, over Parakeet Unified alone, and over Whisper for other languages: each recommendation answers a different need, and Nemotron 3.5 is the only multilingual model that runs live)
- **Use Parakeet Unified when the Settings model is not in the catalog.** Governs R9. (session-settled: user-approved — chosen over the current "unknown model" failure: New Note and Transcribe Again both start from the Settings model)
- **Leave the files of removed models on disk.** Governs R12. (session-settled: user-directed — chosen over deleting them at launch and over a "No longer supported" list in Models: the change happens once)

### Requirements

**Catalog**

- R1. The catalog offers exactly five transcription models:

  | Model | Live transcription | Languages |
  |---|---|---|
  | Parakeet Unified EN 0.6B | Yes | 1 |
  | Nemotron Speech Streaming EN 0.6B | Yes | 1 |
  | Nemotron 3.5 ASR Streaming 0.6B | Yes | 28 |
  | Parakeet TDT 0.6B v3 | No | 25 |
  | Whisper large-v3-turbo | No | 100 |

- R2. Parakeet TDT 0.6B v2, Parakeet TDT 1.1B, MOSS Transcribe Diarize, Granite Speech 4.1 2B Plus and Multitalker Parakeet Streaming 0.6B are not in the app. They do not appear in Settings, in the model downloads or in the About licences.
- R3. Parakeet Unified EN 0.6B stays the model for a new install.

**Recommended**

- R4. Exactly two models show as Recommended: Parakeet Unified EN 0.6B first and Nemotron 3.5 ASR Streaming 0.6B second. The model descriptions already state "English" and "28 languages", so no new text is necessary.

**Speaker recognition**

- R5. Every model uses the separate speaker model that the speaker source selects. The automatic choice does not change: Nemotron 3, or pyannote / WeSpeaker when the expected count is more than 8.
- R6. The speaker source "Transcription model labels" is not in Settings or in any other place where a speaker source shows.
- R7. The app has no processing path for speaker labels from the transcription model. The path is removed, not only hidden.

**Saved values**

- R8. A saved note that a removed model transcribed keeps its transcript and summary. They show as before.
- R9. When the transcription model that Settings saved is not in the catalog, the app uses Parakeet Unified EN 0.6B. Settings shows Parakeet Unified as selected, and New Note and Transcribe Again use it.
- R10. When the speaker source that Settings saved is "Transcription model labels", the app uses the automatic speaker choice (R5).
- R11. Settings shows no error, notice or migration text for R9 or R10.
- R12. The downloaded files of removed models stay on disk. The app does not delete them or show them.

### Acceptance Examples

- AE1. The Settings model was removed
  - **Covers R9, R11.**
  - **Given:** Settings saved Granite as the transcription model before the update.
  - **When:** The user opens Settings, or starts a New Note.
  - **Then:** Settings shows Parakeet Unified EN 0.6B as selected, with no error. The note records with Parakeet Unified.
- AE2. The Settings speaker source was removed
  - **Covers R10.**
  - **Given:** Settings saved "Transcription model labels" as the speaker source.
  - **When:** The user opens Settings.
  - **Then:** The speaker source shows the automatic choice. "Transcription model labels" is not in the list.
- AE3. A note from a removed model
  - **Covers R8, R9.**
  - **Given:** A saved note was transcribed with Granite, and Settings saved Granite.
  - **When:** The user opens the note and selects "Transcribe Again…".
  - **Then:** The note shows its Granite transcript and summary unchanged. Transcribe Again uses Parakeet Unified.

### Scope Boundaries

- No new models are added.
- The app does not delete the files of removed models (R12).
- The transcription window length does not change. Longer windows for After Stop models are a separate idea.
- The runner does not get new log lines for the start and end of each stage. That is a separate change.
- The Transcribe Again flow does not change here. The Transcribe Again plan owns it.
- `docs/reference/recognition-timings-2026-09-27.md` and other measurement records stay unchanged. They are records of past measurements.

### Dependencies / Assumptions

- Nobody else uses Longhand yet, so R9 and R10 serve only the user's installs. Notes need no migration.
- The Transcribe Again plan (`docs/plans/2026-09-28-1244-feat-transcribe-again-from-settings-plan.md`) interacts with this plan in three places. Neither plan must be implemented first:
  - Its AE1 and its U3 test use Granite as the Settings model. The plan that is implemented second changes these examples to Whisper or Parakeet TDT v3, which also cannot reuse Parakeet live text.
  - Its KTD4 speaker-limit text names the model "when the source is the transcription model's own labels". That case does not occur after R6 and R7.
  - With R9, a Settings model that is not in the catalog does not get to its KTD4 "Other configuration errors" text.
- The NeMo runtime artifacts that serve the kept models stay. They are internal and are not picker entries.

### Sources / Research

- `crates/models/catalog.json` — the ten transcription entries, their `streaming`, `languages`, Handy `assessment` scores and `speaker_preferences`.
- `crates/models/src/catalog.rs` — `DEFAULT_ASR_MODEL`, and the tests `recommended_models_cover_live_and_integrated_afterwards` and `integrated_models_prefer_separate_speakers_and_keep_explicit_labels`.
- `src/settings/RecognitionModelPicker.tsx` — a hard-coded `recommendationOrder` with Nemotron Speech Streaming, Parakeet Unified and Granite.
- `crates/models/src/recognition.rs` — `SpeakerSource::Integrated` is accepted only for Multitalker, MOSS and Granite, and an unknown model gives `ResolveError::UnknownModel`.
- `crates/pipeline/src/recognition.rs` and `crates/pipeline/src/recognition/stages.rs` — `Route::Integrated` and its transcription branch.
- `crates/asr/src/result.rs` and `crates/asr/src/engine.rs` — `normalize_moss_transcript`, `normalize_multitalker_transcript` and the MOSS and Multitalker checks.
- `src/settings/RecognitionOptions.tsx` — the "Transcription model labels" source.
- `src-tauri/src/managers/settings.rs` — the settings migrations. None of them maps an unknown saved model.
- `crates/models/PROVENANCE.md` — the list of the ten catalog entries that came from Handy.
- `docs/reference/recognition-timings-2026-09-27.md` — the label counts, the change of the automatic order, and the Granite processing time.
- `docs/solutions/logic-errors/granite-word-timing-misclassified-as-absent.md` — the effect of window context on Granite.
- The transcribe.cpp Multitalker documentation (https://github.com/handy-computer/transcribe.cpp/blob/main/docs/models/multitalker-parakeet-streaming-0.6b-v1.md) — without diarize mode, the model runs its `single_speaker_mode` path, and the per-speaker channels need diarize mode.
