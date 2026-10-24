---
title: Update transcribe-cpp to the current release
description: "Move from transcribe-cpp 0.2.4 to the release that Handy uses (0.4.0 or later), so Longhand keeps the upstream fixes and features"
area: processing
added: 2026-10-15
source: "Handy review, 2026-10-15: commits ffbc950, 1b011a1"
related: [custom-vocabulary, recognition-in-worker-process, handy-recorder-crates]
plan: docs/plans/2026-10-24-1354-feat-ort-and-transcribe-cpp-upkeep-plan.md
---

# Update transcribe-cpp to the current release

## Idea
Update the transcription runtime from transcribe-cpp 0.2.4 to the release that Handy uses (0.4.0 on 2026-10-15). Move `TRANSCRIBE_REVISION` in `crates/models/src/recognition.rs` with the update, so that a new note records the new runtime revision.

## Why
Handy went from 0.2.4 to 0.3.0, 0.3.1 and then 0.4.0 between 2026-09-29 and 2026-10-15. Longhand is still on 0.2.4 (`crates/asr/Cargo.toml`, `src-tauri/Cargo.toml`). Each release that Longhand skips makes the next update larger, and later Handy fixes assume the new API. `custom-vocabulary` needs 0.4.

## Notes / Current state
- 0.4.0 changes the API: `model.capabilities()` returns a `Result`. Longhand calls it in `crates/asr/src/engine.rs` (two calls near line 174).
- Read the transcribe.cpp changelog for 0.3.0 to 0.4.0 before the update. The 2026-10-15 review did not read it: the clone does not contain it, and the cargo registry had only 0.2.3 and 0.2.4.
- Shorthand reports that the Vulkan half-initialized-device crash after Modern Standby is still in 0.3.0 (see `docs/reference/borrowing-notes.md`). The update does not fix it.
- Handy 3985508 runs Silero VAD through transcribe-cpp. The backend mask given to `init_backends_with` is fixed at the first registration in a process.
- Do a real-audio route check before and after, as for the 0.2.4 update (archived PR 25).
- Upkeep review, 2026-10-19 (issue #47): 0.4.0 also has the ggml GGUF loader overflow and malformed-file fixes and the ggml Vulkan Flash Attention out-of-bounds fix, which the 0.2.4 ggml pin does not have. It also adds Silero VAD, a backend allowlist, a repetition guard and a langid role (see `summary-language`). 0.3.0 removes `<unk>` from NeMo output, and 0.4.0 builds with `GGML_NATIVE=OFF` on macOS. PR #51 fixes these two in Longhand before the update: `without_unknown` in `crates/asr/src/engine.rs`, and `TRANSCRIBE_CMAKE_ARGS` with a cache check in the macOS release job. After the update, keep both: the removal does nothing when no marker comes, and the cache check finds a regression.
