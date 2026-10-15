---
title: Prepare the GPU shaders at launch
description: "List the compute devices on a background thread at launch, so the first recording after an update does not wait for the Metal shader compile"
area: processing
added: 2026-09-29
source: "Handy review, 2026-09-29: commit 29bd2c0"
related: [recognition-in-worker-process]
---

# Prepare the GPU shaders at launch

## Idea
At launch, call the compute-device listing on a background thread. This opens the GPU and compiles ggml's Metal shaders before the user records.

## Why
Longhand's `init_backends()` (`crates/asr/src/engine.rs:111`) runs at the first `Engine::load_with`, on the processing or live worker thread. After an install or update, the first recording or processing probably waits for the shader compile (inferred, not measured). Handy moved this listing off its startup path in 29bd2c0.

## Notes / Current state
- Handy review, 2026-10-15 (20a6685, 417dc6a): Handy now shows "Loading model..." during a slow model load, in place of a "Transcribing..." label that does not change. Longhand includes the model load in its "Transcribing" stage (`src/processing/describe.ts`). A separate label is a small part of this idea.
- Handy c8262ab lists the devices in a short-lived worker process, and the shader cache still fills. See `recognition-in-worker-process`.
