---
title: ONNX Runtime 1.31 and transcribe-cpp 0.4.1 upkeep - Plan
type: feat
date: 2026-10-24
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# ONNX Runtime 1.31 and transcribe-cpp 0.4.1 upkeep - Plan

## Goal Capsule

- **Objective:** Longhand users get a release whose model loaders have the current ONNX Runtime and ggml security fixes, and whose transcription uses less memory. Transcripts and speaker labels are the same as before, and notes that were recorded before the update still process.
- **Means:** One PR that updates the two native inference runtimes together (KTD1).
- **Authority:** This plan, then `CLAUDE.md` and `docs/development-workflow.md`.
- **Stop conditions:** Stop and report when a result is outside the comparison limits in U1 and U2, when the Windows build needs a new shipped file that this plan does not name, or when a version fails the `cargo deny` checks and an exception has no reason.
- **Execution profile:** Manifest and lock changes, a small code change in `crates/onnx` and `crates/asr`, a compatibility rule for stored notes, and licence data. Proof is the full local checks, CI on macOS and Windows, and a manual test of a signed macOS build.
- **Who finishes:** The implementing session opens the PR, takes it to green, and gives the user the signed build for the manual test.

## Product Contract

### Summary

Update ort and ort-sys to 2.0.0-rc.15 (ONNX Runtime 1.31) with ndarray 0.17.2, and transcribe-cpp with transcribe-cpp-sys to 0.4.1. Change the code that the new APIs need, accept notes that recorded the previous runtime revisions, and update the licence data. Use none of the new transcribe-cpp features (VAD, language identification, prompting) in this PR.

### Problem Frame

Both runtimes were held. ONNX Runtime 1.22.0, which ort-sys rc.10 links, lacks 12 fixes in model loading and in the CPU operators that the pyannote segmentation and WeSpeaker embedding models use (Gather, MaxPool, LSTM, Conv, shape inference, nested graph depth, external-data paths for models loaded from bytes). ort rc.14 and rc.15 also fix a double free, a use-after-free and more than 30 undefined-behaviour faults in the Rust binding. transcribe-cpp-sys 0.2.4 bundles a ggml whose GGUF loader lacks two overflow fixes (llama.cpp PRs 29384 and 26979). transcribe-cpp 0.4.1 maps model weights from the file instead of copying them: upstream measured Parakeet 0.6B Q8_0 peak memory going from 1128 MiB to 423 MiB, and a Whisper Q8_0 model from 897 MiB to 102 MiB. The risk of the loader faults is a crafted model file. Longhand downloads its models from pinned revisions with SHA-256 checks, so the fixes are defence in depth, and the memory gain is the change that users notice.

### Requirements

**ONNX Runtime**

- R1. ort and ort-sys 2.0.0-rc.15 and ndarray 0.17.2. The macOS build links ONNX Runtime 1.31.0 with Core ML, and the Windows build links ONNX Runtime 1.31.0.
- R2. Speaker analysis with pyannote segmentation and WeSpeaker embeddings gives the same speaker turns as before, on Core ML and on the CPU fallback.

**transcribe-cpp**

- R3. transcribe-cpp and transcribe-cpp-sys 0.4.1 on every target: Metal on macOS, Vulkan with dynamic backends on Windows x86_64.
- R4. Each shipped transcribe.cpp model transcribes as before. Live streaming works for the models whose catalog entry streams.
- R5. Peak memory during the transcription of a long note is lower than with 0.2.4, and the long note completes.

**Stored notes**

- R6. A note that recorded transcribe.cpp revision `4807edaf210d0d7e8a6f7fb2a44b65966a2797f0` or ONNX Runtime `1.22.0` processes with the new runtimes. A new note records the new revisions. A note that recorded an unknown revision is still refused.

**Licences and records**

- R7. The licence list, the About acknowledgements and the runtime revisions name ONNX Runtime 1.31.0 and transcribe-cpp 0.4.1.

### Scope Boundaries

- The new transcribe-cpp features (Silero VAD, language identification, prompting, the backend allowlist) are not used. `custom-vocabulary` and `summary-language` can use them later.
- The NeMo helper and its ggml copy do not change. Its update is the `nemo-helper-update` idea.
- `without_unknown` in `crates/asr/src/engine.rs` and the `TRANSCRIBE_CMAKE_ARGS` cache check in the macOS release job stay (the idea file gives the reason).
- Considered and not built: rewriting the revisions in a stored snapshot after processing. The saved note keeps the revision that it recorded at capture. Only the processing gate reads it, and KTD3 accepts it. A feature that shows the runtime of a saved note would change this.

## Planning Contract

### Key Technical Decisions

- KTD1. **One PR for both runtimes.** Both change `Cargo.lock`, `licences/generated/` and the stored runtime revisions, and each merge makes a release. (session-settled: user-approved — chosen over one PR for each runtime: one release and no lock file conflicts.)
- KTD2. **One process-wide ort `Environment` in `crates/onnx`.** rc.14 makes `Session::builder` take an `Environment`. Create it once, lazily, with telemetry off, and use it for every session builder. Pass no execution providers to the environment: Core ML stays per session in `commit_accelerated`, because Silero-sized models are faster on the CPU. The pyke binaries have no telemetry, but the explicit setting keeps the local-first rule if the binary source changes. Error conversion: rc.15 builder methods return `Error<SessionBuilder>`, so `crates/onnx/src/error.rs` converts that type too, or the builder errors map to the plain `ort::Error`.
- KTD3. **Accept the previous runtime revisions.** Two gates refuse a snapshot whose runtime revision is not the current one: `TaskGraph::resolve` (`crates/pipeline/src/recognition.rs`, "the note requires a different runtime revision") and `Models::load_exact` (`src-tauri/src/managers/processing.rs`, "The selected transcription runtime is not installed"). Each capture writes a snapshot (`src-tauri/src/managers/capture.rs`). So a note that is queued, interrupted or waiting for a model when the update installs would fail. Keep the current revisions as constants in `crates/models/src/recognition.rs` (the ONNX Runtime revision becomes a constant too), and add one function there that accepts the current revision or a listed earlier revision of each runtime. Both gates call it, so they cannot disagree. The model artifacts do not change, and both runtimes read the same files. An unknown revision is still refused, so the gate keeps its purpose. The 0.2.3 to 0.2.4 update (`docs/plans/2026-09-29-1011-fix-reference-review-fixes-plan.md`) skipped this only because Longhand had no users then.
- KTD4. **The Windows ONNX Runtime binary changes.** ort-sys rc.15 publishes no Windows x86_64 build without an execution provider. With no ort feature, its resolver takes the first entry for the target, the DirectML build: `onnxruntime.lib` (static) and `DirectML.dll`. On 2026-10-24 the archive (SHA-256 `f99aaef2…`) has those two files only. `copy-dylibs` puts `DirectML.dll` next to the build output, but the Windows bundle takes only `transcribe-libs/`. In U4, from the Windows build (CI artifact or the Windows runner) and before the PR is marked ready, find out whether `longhand.exe` imports `DirectML.dll` at load time:
  - No load-time import (delay-load, or no DirectML use without the `directml` feature): ship nothing new, and record the finding in a code comment near the Windows bundle config.
  - Load-time import: stage `DirectML.dll` with the transcribe runtime libraries in `src-tauri/build.rs`, and add its licence to `licences/manifest.json`. The DLL in Windows `System32` is older and is not a safe dependency.
- KTD5. **Young versions with a reason.** The age check fails a version under 7 days old. Add exact entries to `min-age-exceptions.toml`:
  - ort and ort-sys 2.0.0-rc.15 (2026-10-19): the double free, use-after-free and undefined-behaviour fixes, and the ONNX Runtime 1.31 model loading fixes. rc.14 has the binding fixes but not all ONNX Runtime fixes. Remove on 2026-10-26.
  - transcribe-cpp and transcribe-cpp-sys 0.4.1 (2026-10-22): 0.4.1 adds the memory reduction and its Metal null mapped buffer patch, and 0.4.0 does not have them. Remove on 2026-10-29.
  - ndarray 0.17.2 (2026-01-10), hmac-sha256 1.1.15 and lzma-rust2 0.15.8 (ort-sys download, build only) are old enough.
- KTD6. **The licence generator reads the new ort-sys file.** `scripts/licences/generate.ts` reads `dist.txt` from the ort-sys package. rc.15 has `build/download/dist.tsv` instead, in another column order. The `ms@<version>` pattern of `checkOnnxRuntimeVersion` (`scripts/licences/group.ts`) still matches each URL, so only the path and the test fixture change.

### Sources

- transcribe-cpp 0.2.4 to 0.4.1 public API: `Model::capabilities()` returns `Result<Capabilities>` (two calls in `Engine::load_with`). `RunOptions` gains `vocabulary`, `prompt` and `prefix`, and `ModelOptions` gains `no_mmap`. Longhand builds both with `..Default::default()`, so they need no change, and the mapped weights are on by default. The crate's source commit is `4d7c1ccefd39e5a9ea36cca5f8fefc6dcc14b99b` (`.cargo_vcs_info.json`).
- ort rc.15 moves `ort::execution_providers` to `ort::ep` (`ep::CoreML`, `ep::coreml::{ComputeUnits, ModelFormat}`).
- Places that name the old versions: `crates/pipeline/src/recognition.rs`, `crates/models/src/recognition.rs`, `src/settings/acknowledgements.ts`, `licences/manifest.json` (`onnxRuntimeVersion` and two entries), `licences/sources/onnxruntime-*.txt`, `scripts/licences/group.test.ts`, the comment above `UNKNOWN` in `crates/asr/src/engine.rs`, `docs/reference/borrowing-notes.md` and the `nemo-helper-update` idea note on ONNX Runtime 1.22.0.
- `docs/solutions/workflow-issues/full-licence-list-blind-spots.md` (gap 4: the ONNX Runtime entry is checked against ort-sys) and `docs/solutions/performance-issues/cargo-feature-unification-ci-rebuilds.md` (a feature change rebuilds ggml in CI).

## Implementation Units

### U1. ONNX Runtime 1.31 and ndarray 0.17

- **Goal:** R1, R2.
- **Dependencies:** none.
- **Files:** `crates/onnx/Cargo.toml`, `crates/onnx/src/lib.rs`, `crates/onnx/src/error.rs`, `crates/embedding/src/onnx/mod.rs` and `crates/pyannote-local/src/segmentation.rs` if the ndarray or tensor API needs it, `Cargo.lock`, `min-age-exceptions.toml`.
- **Approach:** Raise the versions, add the environment (KTD2), move the Core ML imports to `ort::ep`, and add the age exceptions (KTD5).
- **Test scenarios:**
  - The existing `fallback_tests` in `crates/onnx/src/lib.rs` pass.
  - A session builder made twice uses one environment (a test that builds two sessions from a small model, or the existing model tests when their fixtures are present).
  - The embedding and pyannote-local model tests give the same results as on `main` with the real models (`LONGHAND_*` fixture paths), on CPU and with `--features coreml`.
- **Verification:** The workspace builds with and without `coreml`. The diarization evaluation (`crates/pyannote-local/src/bin/diarization_eval.rs`) on a fixture note gives the same speaker count and the same speaker for each turn as `main`, and no turn boundary moves by more than one segmentation frame. Kernel changes between ONNX Runtime 1.22 and 1.31 make bit-identical output unlikely, so these are the comparison limits.

### U2. transcribe-cpp 0.4.1

- **Goal:** R3, R4, R5.
- **Dependencies:** none.
- **Files:** `crates/asr/Cargo.toml`, `src-tauri/Cargo.toml`, `crates/asr/src/engine.rs`, `crates/models/src/recognition.rs` (`TRANSCRIBE_REVISION`), `Cargo.lock`, `min-age-exceptions.toml`.
- **Approach:** Raise the three target entries in `crates/asr/Cargo.toml` and the entry in `src-tauri/Cargo.toml`. Map a `capabilities()` error to the existing load error. Set `TRANSCRIBE_REVISION` to `4d7c1cce…`. Update the `UNKNOWN` comment.
- **Test scenarios:**
  - The `crates/asr` tests that load a real model (fixture paths) transcribe the fixture audio with each catalog transcribe.cpp model that the fixtures have. The text of each differs from `main` only in punctuation, case or a few isolated words (the comparison limit).
  - A model whose capabilities say it streams still streams, and one that does not gets the full pass after Stop (the existing `streams` tests).
- **Verification:** Workspace build and tests pass. `cargo tree -i transcribe-cpp-sys` shows one copy at 0.4.1.

### U3. Stored notes keep processing

- **Goal:** R6.
- **Dependencies:** U1, U2.
- **Files:** `crates/models/src/recognition.rs`, `crates/pipeline/src/recognition.rs`, `crates/pipeline/src/recognition/saved_prefix.rs`, `crates/pipeline/tests/recognition_routes.rs`, `src-tauri/src/managers/processing.rs`.
- **Approach:** Add the accepted-revision function of KTD3, call it from both gates, and use the ONNX Runtime constant in both places that now write `"1.22.0"`.
- **Test scenarios:**
  - A snapshot with transcribe.cpp `4807edaf…` resolves, and the note processes through the transcribe stage.
  - A snapshot with transcribe.cpp `4807edaf…` passes the runtime check of `load_exact`.
  - A snapshot with pyannote speakers and ONNX Runtime `1.22.0` resolves.
  - A snapshot with a changed revision (the existing `runtime` case in `saved_prefix.rs` and the test in `recognition_routes.rs`) is still refused.
  - A new snapshot records `4d7c1cce…` and `1.31.0`.
- **Verification:** The pipeline tests pass.

### U4. Licences, Windows binary and records

- **Goal:** R7, KTD4, KTD6.
- **Dependencies:** U1, U2.
- **Files:** `scripts/licences/generate.ts`, `scripts/licences/group.test.ts`, `licences/manifest.json`, `licences/sources/onnxruntime-LICENSE.txt`, `licences/sources/onnxruntime-ThirdPartyNotices.txt`, `licences/generated/*`, `src/settings/acknowledgements.ts`, `docs/reference/borrowing-notes.md`, `docs/ideas/someday/nemo-helper-update.md`, and for KTD4 possibly `src-tauri/build.rs` and a DirectML licence source.
- **Approach:**
  1. Change the generator to read `build/download/dist.tsv` (KTD6) and update its fixture.
  2. Fetch the v1.31.0 `LICENSE` and `ThirdPartyNotices.txt` and update the manifest entries.
  3. Update the acknowledgement versions and URLs.
  4. Settle KTD4 from the Windows CI build or the Windows runner.
  5. Run `scripts/check-licences.sh` and commit what it writes.
- **Test scenarios:** `npm run test:scripts` passes with the new `dist.tsv` fixture, including the case where the file names a version that is not the manifest version.
- **Verification:** `scripts/check-licences.sh` writes no change after the commit. The Windows CI build passes, and the KTD4 finding is recorded.

### U5. Signed build and manual test

- **Goal:** R2, R4, R5, R6 on a real build.
- **Dependencies:** U1 to U4.
- **Files:** none.
- **Approach:** Build a signed macOS bundle, notarized when the user tests on another Mac. Give the open and scp steps. Ask the user before any Windows manual step.
- **Test scenarios,** on the signed build:
  - Record a short note with each shipped transcribe.cpp model. Each gives a transcript, and the streaming models show live text.
  - Record a note with two speakers and pyannote speaker labels. The speakers are labelled as on the previous release.
  - Transcribe a long note (30 minutes or more) with the default model. Compare peak memory with the installed release on the same audio (Activity Monitor or `footprint`). The note completes.
  - A note queued on the installed release, then processed by this build, completes (R6).
- **Verification:** The user reports that each scenario passed, with the memory numbers.

## Verification Contract

| Gate | Command or check |
|---|---|
| Rust | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` |
| Frontend and scripts | `npm run typecheck`, `npm test`, `npm run test:scripts` |
| Windows lint | `scripts/check-windows.sh` |
| Licences | `scripts/check-licences.sh` writes no change after the commit |
| Dependency rules | `cargo deny check`; `scripts/check-dependency-age.ts` passes with only the KTD5 exceptions |
| CI | All PR checks green on macOS and Windows, including CodeQL |
| Manual | U5 scenarios pass on the signed macOS build |

## Definition of Done

- R1 to R7 are true, and every gate in the Verification Contract passes.
- KTD4 is settled and recorded.
- The PR title is a `feat` title that names both runtimes and the lower memory use. The user chose patch or minor before the PR opened.
- The `transcribe-cpp-update` idea is in `next/` with this plan, and it moves to `done/` after the merge.
- The diff has no code from attempts that the work did not keep.
