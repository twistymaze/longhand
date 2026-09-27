---
title: Flexible speaker recognition
type: feat
date: 2026-09-27
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
status: requirements-ready
---

# Flexible speaker recognition

## Goal Capsule

- **Objective:** Users choose how to recognize speakers and how long to wait for a usable transcript and summary after a long recording.
- **Means:** Model-provided labels, a separate speaker model, or no labels, with processing during recording where supported.
- **Product authority:** The user approved this scope on 2026-09-27, including Granite Speech 4.1 2B Plus. This Product Contract governs behavior. Implementation planning enriches this file in place.
- **Open blockers:** None for requirements planning. Runtime feasibility, default selection, and performance validation remain classified below.

---

## Product Contract

### Summary

Longhand will offer speaker recognition from the transcription model or a separate model, plus an off option. The model picker will explain capabilities and tradeoffs. Compatible choices will process during recording, and active processing will prevent idle system sleep.

### Problem Frame

Speaker analysis delays completion of long notes. Live transcription reduces work after Stop, but Longhand does not use speaker labels from that stream. The picker does not explain which models supply those labels. Processing does not hold the idle-sleep assertion that recording holds.

The user's preferred wait is a couple of minutes for a three-hour recording. Other users may accept five minutes per recorded hour or longer. One fixed speaker pipeline cannot serve both preferences.

### Requirements

**Choices and language**

- R1. User-facing text calls diarization "speaker recognition" and describes anonymous speaker labels, without implying identification of named people.
- R2. Users can choose the transcription model's speaker labels, a separate speaker model, or speaker recognition off, where the selected models support the choice.
- R3. Users can set app-wide defaults and override transcription and speaker-recognition choices for one note before recording.
- R4. Each note records its effective models, speaker-recognition source, and processing mode so later default changes do not alter its processing.
- R5. Compatible choices support processing during recording or after Stop; the UI distinguishes live transcription, live speaker analysis, and live text with speaker labels.
- R6. Users can reprocess a finished note with different choices, reusing compatible completed work and keeping the previous result until replacement succeeds.
- R7. Reprocessing explains when a change requires transcription again, and refreshes the summary from the resulting transcript.
- R8. Longhand does not automatically run a second quality-refinement pass after a successful live result.

**Models and runtime selection**

- R9. The transcription catalog adds Granite Speech 4.1 2B Plus and retains the existing nine transcription choices.
- R10. Model-provided speaker recognition includes MOSS Transcribe Diarize, Granite Speech 4.1 2B Plus, and the speaker-capable Multitalker Parakeet bundle.
- R11. Separate speaker choices include Nemotron 3 Diarization, Sortformer, and the existing pyannote/WeSpeaker pipeline.
- R12. Longhand selects the runtime from the model and speaker-recognition choice without exposing pipeline selection as a setting.
- R13. Planning evaluates both combined NeMo-Speech.cpp transcription with speaker recognition and transcription through transcribe.cpp with separate NeMo-Speech.cpp speaker analysis.
- R14. Runtime changes are allowed, but the solution must not require a maintained transcribe.cpp fork.
- R15. The design must provide a documented native macOS and Windows route; platform support and acceleration are assessed separately.
- R16. Longhand downloads and validates the artifacts required for the selected capabilities, including the speaker-capable Multitalker bundle.
- R17. The existing pyannote/WeSpeaker option uses supported acceleration on macOS with a correct fallback when acceleration is unavailable.

**Model information**

- R18. Model selection shows live transcription support, speaker-label support and timing, speaker limits, languages, download size, and available speed and accuracy assessments.
- R19. Longhand reuses Handy's published assessments with attribution and available benchmark context; ASR accuracy does not stand in for speaker accuracy, and missing assessments remain unknown.
- R20. Capability information describes the exact downloaded artifact and runtime combination, rather than assuming all variants of a model have the same capabilities.
- R21. Users see incompatibilities and known speaker limits before recording; an explicit speaker count above a supported maximum cannot silently select that combination.

**Long recordings and completion**

- R22. Recording retains priority over recognition work, and a recognition backlog or failure cannot stop capture or discard recorded audio.
- R23. Longhand handles provisional text and speaker labels, stream restarts, and finalization without presenting unfinished output as final.
- R24. Speaker labels retain their meaning across processing boundaries where the model permits it; uncertain continuity must not silently assign a known label to an unrelated speaker.
- R25. Interrupted processing can resume from durable progress, and cancellation preserves the recording and any prior finished result.
- R26. Active post-recording processing prevents idle system sleep and releases that protection on completion, failure, or cancellation; display sleep and screen locking remain available.
- R27. The preferred fast choice targets a usable transcript and summary within two minutes after a three-hour recording stops on the intended laptop, including all remaining processing and summary time.
- R28. Longhand supports slower user-selected choices without using the fast target as a recording-duration limit or processing cutoff.
- R29. Focused validation uses the existing long RPG recording to check completion time, coverage, memory use, cancellation, and speaker-label continuity; it does not create a model-accuracy leaderboard.

### Key Decisions

- KD1. **Offer integrated and separate speaker recognition.** (session-settled: user-directed) The user explicitly requested both approaches. Governs R2, R10, R11, R13.
- KD2. **Select runtimes inside the application.** (session-settled: user-directed) Two transcription runtimes are acceptable. Existing architecture alone is not a reason to reject a capability. Governs R12-R15.
- KD3. **Prefer a combined NeMo route when the combination works well.** (session-settled: user-approved) Compare combined and separate processing before selecting routes; simplicity alone does not decide. Governs R13.
- KD4. **Use the working Multitalker bundle and add Granite Plus.** (session-settled: user-directed) The installed plain Multitalker artifact is not a constraint on this proposal. Governs R9, R10, R16.
- KD5. **Give users informed choices.** (session-settled: user-directed) Published Handy assessments are sufficient catalog evidence. Governs R18-R21, R28, R29.
- KD6. **Use defaults, per-note overrides, and explicit reprocessing.** (session-settled: user-approved) Governs R3-R8.
- KD7. **Validate the fast finish on the intended laptop.** (session-settled: user-directed) Short M3 Ultra tests do not establish laptop performance. Governs R27, R29.
- KD8. **Protect processing from idle sleep.** (session-settled: user-approved) Locking and display sleep are distinct from system sleep. Governs R26.

### Model and Route Coverage

This candidate mapping supports R9-R15. It does not claim that every integration has passed acceptance tests. "Separate" means Longhand joins transcription timestamps with a speaker timeline.

| Transcription model | Live text | Model-provided speaker labels | Candidate route with a separate NeMo speaker model |
|---|---|---|---|
| Parakeet Unified EN 0.6B | Yes | Not established | transcribe.cpp plus separate analysis |
| Nemotron Speech Streaming EN 0.6B | Yes | Requires a speaker model | Compare combined NeMo with separate analysis |
| Nemotron 3.5 ASR Streaming 0.6B | Yes | Requires a speaker model | Compare combined NeMo with separate analysis |
| Multitalker Parakeet Streaming 0.6B | Yes | Speaker-capable bundle; live attribution needs validation | transcribe.cpp plus separate analysis when chosen |
| Parakeet TDT 0.6B v2 | No | Not established | transcribe.cpp plus separate analysis |
| Parakeet TDT 0.6B v3 | No | Requires a speaker model | Compare combined NeMo with separate analysis; ASR remains offline |
| Parakeet TDT 1.1B | No | Not established | transcribe.cpp plus separate analysis |
| Whisper large-v3-turbo | No | Not established | transcribe.cpp plus separate analysis |
| MOSS Transcribe Diarize | No | Offline; locally demonstrated | transcribe.cpp plus separate analysis when chosen |
| Granite Speech 4.1 2B Plus | No | Offline; upstream documented | transcribe.cpp plus separate analysis when chosen |

NeMo-Speech.cpp supports Parakeet CTC 1.1B; that does not establish support for Longhand's TDT 1.1B. A combined Nemotron route also loads a speaker model. For integrated labels, the candidate route is transcribe.cpp. With speaker recognition off, use the catalog-selected ASR runtime. The existing pyannote/WeSpeaker option remains a separate pipeline beside the selected ASR runtime.

| Speaker option | Capacity and timing | Evidence and limit |
|---|---|---|
| Nemotron 3 Diarization | Up to eight speakers; streaming and offline | NeMo-Speech.cpp documentation; not tested locally |
| Sortformer | Up to four speakers; streaming and offline through a suitable runtime | NeMo-Speech.cpp documents v2; Multitalker's bundle uses v2.1; exact artifacts need selection |
| pyannote/WeSpeaker | Existing clustering pipeline | No equivalent fixed four- or eight-speaker output cap; duration boundaries need validation |
| Multitalker bundle | Up to four speakers through bundled Sortformer | Validate live attributed output, not just live text |
| MOSS / Granite Plus | Offline integrated output | Validate long-audio coverage and timestamps for each artifact |
| Off | No speaker analysis | Transcription and summarization remain available |

A fixed speaker capacity limits distinct labels. Additional speakers can be merged or confused; the limit does not promise a crash or reliable overflow detection. R21 governs known limits before recording.

### Key Flows

- F1. **Choose a configuration.** In Settings, the user compares capabilities, selects transcription and speaker options, and downloads the required artifacts. A new note inherits the defaults and allows an override. Covers R2-R5, R16, R18-R21.
- F2. **Finish a long live recording.** Longhand records continuously while compatible recognition runs. Stop finalizes live results and completes missing work before the summary and Save. Covers R22-R28.
- F3. **Use an offline model.** A user selects MOSS or Granite Plus. The UI shows that transcription and integrated labels arrive after Stop, even when an independently selected speaker model can analyze audio during recording. Covers R5, R10, R18, R20.
- F4. **Change the result.** A user reprocesses a saved note with another speaker option. Longhand explains required retranscription, retains the prior result during processing, and updates the summary after success. Covers R6-R8, R25.

### Acceptance Examples

- AE1. **Fast completion.** A paced replay or signed recording test measures Stop-to-result time for the long RPG recording on the intended laptop. The report records hardware, models, backlog, and summary endpoint time. Covers R27, R29.
- AE2. **Speaker limit.** Five expected speakers cannot silently use a four-speaker option as if five distinct labels were available. The user can select a suitable alternative. Covers R21.
- AE3. **Integrated labels.** MOSS and Granite Plus labels reach the transcript without an unnecessary separate speaker pass. A Multitalker live test demonstrates attributed output from the correct bundle. Covers R10, R16, R20.
- AE4. **Separate choices.** Whisper with Nemotron 3 Diarization joins offline text with the speaker timeline. Nemotron Speech Streaming with that diarizer is compared in combined and separate execution before routing is settled. Covers R11-R13.
- AE5. **Long-session boundaries.** A recording that exceeds existing stream and clustering boundaries has no missing audio span or silent speaker-number reuse caused by a restart. Covers R22-R25, R29.
- AE6. **Lock and idle.** During active processing, screen lock does not cancel the job and idle system sleep does not suspend it. Completion, failure, and cancellation release the assertion. Covers R26.
- AE7. **Failed replacement.** A failed or cancelled replacement leaves the saved note's audio, notes, title, and previous finished files usable. Covers R6, R25.
- AE8. **Capability display.** A missing speaker-accuracy assessment stays unknown. A streaming transcription flag alone does not enable a live-speaker capability. Covers R18-R20.

### Scope Boundaries

Speaker identity, enrollment, voiceprints, and automatic naming remain outside scope under [STRATEGY.md](../../STRATEGY.md). R1 uses familiar UI language for anonymous turn attribution.

Per-channel stereo processing, a scientific diarization benchmark suite, and power-aware scheduling remain separate ideas. The recording surface retains its existing attention budget. R14 excludes a maintained transcribe.cpp fork. R8 excludes automatic refinement.

This brainstorm uses Windows documentation, not a Windows execution test. Under R15, implementation planning must distinguish documented feasibility from subsequent build and release validation.

### Success Criteria

The R2 choices have supported routes, and R18-R21 let users compare capabilities without knowing runtime names. R29 establishes the limits of those routes before a new default is selected.

The R27 report must state whether the target was met on the intended laptop. If no candidate meets it, retain the target, report measured times and limiting stages, and return the default/performance tradeoff to the user. Do not redefine success from excerpt extrapolation.

### Outstanding Questions

All questions are **Deferred to Planning**. None changes the agreed product choices.

| Question | Evidence and decision boundary |
|---|---|
| Which NeMo combinations should use one runtime? | Compare combined and separate execution on the same audio and models; assess timestamps, memory, cancellation, and total time under R13. |
| What does the Multitalker bundle expose live? | Exercise attributed streaming output. If absent, find a no-fork upstream route or report the capability gap under R10 and R14. |
| Which exact artifacts and versions ship? | Pin runtimes and models; verify licences, checksums, platform builds, and metadata under R15, R16, R20. |
| How do live results become durable? | Define provisional-to-final updates, speaker continuity, backlog recovery, and resume under R22-R25. Current fixed stream resets are not a constraint. |
| Which default meets the user's target best? | Measure R27 and R29. Streaming Nemotron with Nemotron 3 Diarization is a leading candidate, not a settled default. Preserve existing selections during migration. |
| How does acceleration fail over? | Verify Core ML availability, numerical validity, and CPU fallback under R17; identify the active backend in diagnostics. |
| How does the picker handle unsupported speaker counts? | Select the interaction under R21 without implying that an over-capacity model can distinguish every speaker. |

### Evidence and Sources

Repository inspection on 2026-09-27 found CPU ONNX sessions with one intra-op and one inter-op thread in [crates/onnx/src/lib.rs](../../crates/onnx/src/lib.rs). Core ML embedding support exists behind an opt-in feature that the application does not enable; segmentation remains CPU. [crates/asr/src/engine.rs](../../crates/asr/src/engine.rs) disables native attribution with its default run options and does not preserve available speaker output. [crates/asr/src/stream.rs](../../crates/asr/src/stream.rs) resets streams at bounded part lengths. [crates/diarize/src/lib.rs](../../crates/diarize/src/lib.rs) bounds clustering chunks. These boundaries require continuity checks. Recording uses [crates/power/src/assertion.rs](../../crates/power/src/assertion.rs); processing does not acquire that protection.

Local tests used release builds, Rust 1.94.0, transcribe-cpp 0.2.3, and a mono 16 kHz excerpt from minutes 30-35 of the 176.2-minute RPG recording. The host was an Apple M3 Ultra, not the intended laptop. Scratch harnesses used separate dependency locks. Original audio and application code were unchanged.

| Test | Observation | Limit |
|---|---|---|
| Existing pipeline, five-minute excerpt, CPU | 60.547 s analysis; 146 windows; 289 embeddings | Excerpt baseline only |
| Same pipeline, warm Core ML embeddings | 9.200 s analysis; same windows and embeddings | About 6.58 times faster analysis, not end-to-end completion |
| Numerical comparison | Segmentation identical; embeddings finite; minimum cosine similarity 0.999994; four speakers in both, 109 versus 110 turns | Numerical agreement, not an accuracy ranking |
| MOSS Q8 on Metal, five-minute excerpt | 14.918 s inference; 89 attributed segments; latest end 299.980 s; truncation flag false | Labels demonstrated; complete word coverage not established |

The Core ML result is an uncontended warm run. A sandbox compilation failure fell back to CPU and was excluded. Host execution assigned 102 of 106 embedding graph nodes to Core ML. Linear extrapolation gives about 5.4 minutes of analysis for the full recording, excluding transcription and summarization. This is not a full-recording measurement.

NeMo-Speech.cpp, Granite Plus, and the speaker-capable Multitalker bundle were not benchmarked locally. No Windows execution test or full three-hour completion test was performed. The installed plain Multitalker artifact's missing labels do not establish a limitation of the required bundle.

- [NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp), [model support](https://github.com/NVIDIA/NeMo-Speech.cpp/blob/main/docs/asr/models.md), [SDK](https://github.com/NVIDIA/NeMo-Speech.cpp/blob/main/docs/sdk.md), and [build instructions](https://github.com/NVIDIA/NeMo-Speech.cpp/blob/main/docs/build.md): combined and separate components, streaming differences, and native platform routes.
- [Nemotron 3 Diarization model card](https://huggingface.co/nvidia/Nemotron-3-Diarization): eight-speaker streaming/offline model, distinct from Nemotron ASR.
- [Multitalker documentation](https://github.com/handy-computer/transcribe.cpp/blob/main/docs/models/multitalker-parakeet-streaming-0.6b-v1.md): speaker-capable bundle and embedded Sortformer.
- [Granite Speech 4.1 2B Plus documentation](https://github.com/handy-computer/transcribe.cpp/blob/main/docs/models/granite-speech-4.1-2b-plus.md): the specific variant in R9.
- [Sortformer source](https://github.com/handy-computer/transcribe.cpp/blob/main/src/arch/sortformer/model.cpp): pinned 0.2.3 and inspected upstream source lack public live-stream hooks. Internal streaming computation does not establish a public live API.
- [Current catalog](../../crates/models/catalog.json): retain Handy assessments and their available context under R19. Model-family claims must not overwrite pinned-artifact capabilities.

### Work Relationships

<!-- ce-section: work-relationships -->

This extends the [recognition-model and live-transcript plan](2026-09-25-1121-feat-recognition-models-and-live-transcript-plan.md). Its fixed speaker pipeline decision, KD4, is superseded for this scope by R2 and R10-R13. Its final-live-text assumption changes under R23. Completed plans remain historical records.

The [faster diarization](../ideas/faster-diarization.md) and [alternative architectures](../ideas/alternative-diarization-architectures.md) ideas now use this plan. [Per-channel processing](../ideas/per-channel-processing.md) stays separate. The sleep fix can be delivered independently within this scope. The [capability display](../ideas/model-capabilities-and-speaker-options.md) and [processing sleep protection](../ideas/prevent-sleep-during-processing.md) entries track their parts of this outcome.
