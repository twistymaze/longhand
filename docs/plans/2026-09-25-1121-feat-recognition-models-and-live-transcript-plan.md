---
title: Recognition Models and Live Transcript - Plan
type: feat
date: 2026-09-25
topic: recognition-models-and-live-transcript
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Recognition Models and Live Transcript - Plan

## Goal Capsule

- **Objective:** The user chooses the recognition model that suits them. With a streaming model, the user can see the transcript while recording and read the complete note moments after Stop. When the fast transcript is not good enough, the user can reprocess the note.
- **Product authority:** This plan covers the recognition-model picker, the live transcript, the fast finish, Reprocess and the simpler download of the speaker-detection weights. It covers areas B and C of `docs/plans/2026-09-24-1956-feat-ux-redesign-plan.md` (How This Work Fits Together). Speaker labels from the recognition model, a choice of speaker-detection models and system audio capture are not active scope.
- **Open blockers:** None.

---

## Product Contract

### Summary

Settings gets a recognition-model picker. It offers a curated list of nine models from Handy's catalog, and all nine run on the engine that Longhand already uses. When the selected model streams, the Transcript tab fills in during recording, and the transcript is ready moments after Stop. A Reprocess action runs a full pass on the original audio when the user wants more accuracy. Parakeet Unified EN becomes the default for new installs. The speaker-detection weights become one "other required models" download.

### Problem Frame

The walking skeleton hard-codes Whisper large-v3-turbo. The user does not want to use this model every day (`docs/ideas/recognition-model-picker.md`). Whisper does not stream, so recognition starts only after Stop and a long recording takes a long time to finish. The Transcript tab exists during recording, but it shows only "The transcript is ready after you stop recording." (`src/session/SessionPage.tsx`). The Transcription settings pane shows one row for each required weight file: "Speech recognition", "Speaker segmentation" and "Speaker embedding" (`src/settings/Models.tsx`). The user has no choice to make about the last two, but must still read about them.

The skeleton prepared for this change. The catalog lists models and has a `streaming` flag. Settings carry `asr_model`. Each session records its model. Recognition sits behind the `Recognizer` trait. Handy's catalog is a bundled `catalog.json` that a build-time script generates from the `handy-computer` Hugging Face organization, and every entry runs on `transcribe-cpp`. Longhand already uses `transcribe-cpp` 0.2.3, which has a streaming API. Thus the second-engine problem in skeleton KTD6 (a second `ort` pin through `transcribe-rs`) does not apply.

### Key Decisions

- KD1. **A curated list of nine models, not Handy's full catalog.** (session-settled: user-directed — chosen over all compatible models (about 50) and over a verbatim copy of Handy's file: a short list where each model is tested. A later plan can add models.) Governs R1.
- KD2. **The catalog stays bundled in the app. The app does not fetch it at run time.** This keeps the local-first rule and follows Handy. A script refreshes the list from Handy's catalog when a developer runs it. Governs R2.
- KD3. **For a streaming model, the streamed result is the final transcript.** (session-settled: user-directed — chosen over a full pass after every Stop: the user wants the fast finish. Reprocess (KD6) gives full accuracy on request.) Governs R11, R13.
- KD4. **Speaker detection stays the same for every model.** (session-settled: user-approved — chosen over the use of a model's own speaker labels: their quality is unknown, and a second speaker-detection path adds cost.) MOSS Transcribe Diarize is in the list only as a recognizer. Governs R10, R12.
- KD5. **The live transcript uses the existing Transcript tab. That tab does not open by default during recording.** (session-settled: user-directed — the user named the existing tab.) The recording page still opens on My notes, so the attention budget in ux-redesign KD2 continues to apply. Governs R8, R9.
- KD6. **Reprocess uses the model that is selected now, in a full pass, and it replaces the transcript and the summary.** (session-settled: user-approved — proposed with the tradeoff shown and confirmed: the user can also try an old note with a different model.) Governs R15, R16, R17.
- KD7. **When the user picks a model, the download starts. The model becomes active when it is ready.** (session-settled: user-directed — chosen over a separate download step before a pick: "like how Handy does it now".) Governs R5.
- KD8. **Parakeet Unified EN 0.6B is the default for new installs. An existing install keeps its model.** (session-settled: user-directed — chosen over Whisper large-v3-turbo: a streaming default makes the live transcript and the fast finish work at once.) Governs R3.

---

### Requirements

**Model list**

- R1. The catalog lists these nine recognition models, with the identifiers from Handy's catalog:

  | Model | Streams | Languages | Handy accuracy / speed |
  |---|---|---|---|
  | Parakeet Unified EN 0.6B (`parakeet-unified-en-0.6b`) | yes | English | 90 / 79 |
  | Nemotron Speech Streaming EN 0.6B (`nemotron-speech-streaming-en-0.6b`) | yes | English | 86 / 80 |
  | Nemotron 3.5 ASR Streaming 0.6B (`nemotron-3.5-asr-streaming-0.6b`) | yes | 28 | 82 / 84 |
  | Multitalker Parakeet Streaming 0.6B (`multitalker-parakeet-streaming-0.6b-v1`) | yes | English | 86 / 96 |
  | Parakeet TDT 0.6B v2 (`parakeet-tdt-0.6b-v2`) | no | English | 89 / 85 |
  | Parakeet TDT 0.6B v3 (`parakeet-tdt-0.6b-v3`) | no | 25 | 88 / 79 |
  | Parakeet TDT 1.1B (`parakeet-tdt-1.1b`) | no | English | 91 / 76 |
  | Whisper large-v3-turbo (the existing entry) | no | 100 | 88 / 35 |
  | MOSS Transcribe Diarize (`moss-transcribe-diarize`) | no | 2 | 88 / 31 |

- R2. Each catalog entry records what the picker and the pipeline need: whether the model streams, its timestamp level, its languages, its size, its licence and attribution, and a pinned source with a checksum. Every entry must produce timestamps, because the speaker join needs them.
- R3. A new install selects Parakeet Unified EN 0.6B. An install that already has a stored model keeps that model.

**Picker**

- R4. The Transcription pane in Settings shows the nine models. For each model it shows the name, a one-line description, the size, the languages, and a tag when the model gives a live transcript. It marks the selected model and the recommended model.
- R5. When the user picks a model that is not downloaded, its download starts. The previous model stays in use until the download and the check are complete. Then the picked model becomes the selected model.
- R6. The user can delete a downloaded model that is not selected, to recover disk space. The selected model cannot be deleted.
- R7. The speaker-detection weights appear as one item, "Other required models", with one download action and one combined progress. The pane does not name the individual weight files.

**Live transcript**

- R8. When the selected model streams, the Transcript tab shows the transcript as it forms during recording. The recording page still opens on My notes, and nothing moves the user to the Transcript tab.
- R9. When the selected model does not stream, the Transcript tab during recording says that the transcript is ready after the user stops recording.
- R10. The live text is recognition text only. It has no speaker labels until processing after Stop adds them.

**Fast finish**

- R11. After Stop, a note that has a complete streamed transcript skips the full recognition pass. Speaker detection, the speaker join, the check, the summary and the save still run.
- R12. Speaker detection runs for every model in the same way as today (KD4).
- R13. A note gets a full recognition pass after Stop when its stream did not complete. This includes a crash, an app quit during recording, and a stream failure. When the check finds speech that got no text, only those spans get a full pass.
- R14. A streamed note records the model that it started with. When the user changes the model during a recording, the change applies to the next note.

**Reprocess**

- R15. A completed note has a Reprocess action. It runs a full recognition pass on the original audio with the model that is selected now, and then runs the steps after recognition again.
- R16. Reprocess replaces the note's transcript and summary. It does not change My notes or a title that the user set.
- R17. When the selected model is not downloaded, Reprocess waits for it in the same way that processing waits for missing models today.

---

### Acceptance Examples

- AE1. **Covers R8, R10.** Given Parakeet Unified EN is selected, when the user records and opens the Transcript tab, text appears as people speak, with no speaker labels. The page opened on My notes.
- AE2. **Covers R9.** Given Whisper large-v3-turbo is selected, when the user opens the Transcript tab during recording, the tab says the transcript is ready after the user stops recording.
- AE3. **Covers R11.** Given a streamed recording of 30 minutes, when the user stops, the transcript with speaker labels is ready without a full recognition pass. Only speaker detection and the later steps run.
- AE4. **Covers R13.** Given the app quits during a streamed recording, when the user opens the app again and the recovered note processes, it gets a full recognition pass.
- AE5. **Covers R5.** Given Whisper is selected and Parakeet TDT 1.1B is not downloaded, when the user picks Parakeet TDT 1.1B, its download starts. A note recorded during the download uses Whisper. After the check completes, Parakeet TDT 1.1B is selected.
- AE6. **Covers R14.** Given a recording started with Parakeet Unified EN, when the user picks Whisper during the recording, the note keeps Parakeet Unified EN, and the next note uses Whisper.
- AE7. **Covers R15, R16.** Given a streamed note with notes the user wrote and a title the user set, when the user runs Reprocess with Parakeet TDT 1.1B selected, the transcript and the summary are replaced. My notes and the title do not change.
- AE8. **Covers R6.** Given Parakeet Unified EN is selected, the user cannot delete it. The user can delete a downloaded Whisper.
- AE9. **Covers R3.** Given an existing install with Whisper stored as its model, after the update Whisper stays selected.

---

### Scope Boundaries

- Speaker labels from the recognition model (MOSS, Multitalker Parakeet). `docs/ideas/alternative-diarization-architectures.md` records this.
- A choice of speaker-detection models.
- A language setting. The multilingual models detect the language.
- Import of custom model files, and models from Handy's catalog outside R1.
- Models that produce no timestamps (Canary, Cohere, Granite, Qwen3, Voxtral).
- A choice of model for one Reprocess run other than the selected model.

### Dependencies / Assumptions

- `transcribe-cpp` 0.2.3 streams with the four streaming models in R1, and `Stream::snapshot()` gives timed segments or words that the speaker join can use. Planning confirms this for each model.
- Capture has no 16 kHz feed during recording. Audio is resampled to 16 kHz only in Assemble. Streaming needs a live feed at 16 kHz.
- Streaming uses the CPU and GPU during recording, which are idle today. This can use more power on battery (`docs/ideas/power-aware-processing.md`).
- Handy's catalog and generation script are MIT-licensed. Borrowed data or code needs a `PROVENANCE.md` record. The model licences include CC-BY-4.0, which needs attribution.

### Outstanding Questions

**Deferred to Planning**

- Which quant each model uses. Handy's default quant is the starting point.
- Where Reprocess sits on the note page, and its UI text.
- How the stream checkpoints its text during recording so that a crash keeps what was already recognized.
- Whether the speaker join needs a change for streamed text, because today diarization cuts the recognition windows at speaker changes.
- Whether the picker shows Handy's accuracy and speed scores. The `apple-design` skill decides this.

### Sources / Research

- `reference-source/Handy/src-tauri/src/catalog/catalog.json` and `catalog/mod.rs`: the bundled catalog and its fields (`capabilities.streaming`, `timestamps`, scores, `recommended_rank`).
- `crates/models/catalog.json` and `crates/models/src/catalog.rs`: Longhand's catalog, `DEFAULT_ASR_MODEL` and `required_for`.
- `crates/asr/src/engine.rs`: the `Recognizer` trait and the `transcribe-cpp` engine.
- `crates/pipeline/src/runner.rs` and `crates/pipeline/src/verify.rs`: the stage order, and the check that lists the windows to transcribe again.
- `src-tauri/src/managers/capture.rs`: `record_model_at_stop`.
- `src/settings/Models.tsx` and `src/session/SessionPage.tsx`: the download rows and the Transcript tab during recording.
- `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`: KTD6, KD8 and R10.
- `docs/ideas/recognition-model-picker.md` and `docs/ideas/live-transcript-view.md`.
