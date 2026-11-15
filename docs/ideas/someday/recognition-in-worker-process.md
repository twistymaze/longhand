---
title: Run recognition in a worker process
description: "Run transcribe-cpp in a child process, so a GPU fault or a hang stops only that process and Longhand can try again on the CPU"
area: processing
added: 2026-10-15
source: "Handy review, 2026-10-15: commit c8262ab; Shorthand review, 2026-10-15: commit 82a164c"
related: [transcribe-cpp-update, prewarm-gpu-shaders]
---

# Run recognition in a worker process

## Idea
Run all transcribe-cpp code in a child process: the same executable, started with a worker argument. When the worker stops because of a GPU fault, a `GGML_ASSERT` abort or a hang, Longhand starts a new worker on the CPU and continues. When a model unloads, the worker exits and all its memory goes back to the system.

## Why
Longhand runs transcribe-cpp in the app process (`crates/asr/src/engine.rs`). A native fault in the runtime stops the full app. Shorthand found such a fault on Windows: after Modern Standby, Vulkan device creation can fail and leave a half-built device, and the next load stops the process (0xC0000409). Handy made this change in c8262ab, and Shorthand waits for it before it fixes recovery.

## Notes / Current state
- The NeMo helper (`native/nemo-helper`) already runs in its own process. Its supervision may be a model for this.
- Live transcription streams audio, so the worker needs a streaming protocol, not only a file call.
- Large effort.
- Added 2026-11-15 (upkeep review, Handy GPU worker retry): a lost GPU device can also come back as a backend error, not a crash. Longhand maps it to `AsrError::Run` and keeps the same ggml-vulkan device, which ggml keeps in a process-wide cache, so after a driver reset on Windows each later note probably fails until a restart (inferred from the code, not tested). Handy now retries once in a new GPU worker and then uses the CPU. A smaller first step without a worker: on a backend error, run the failed job again on the CPU.
