---
title: Update the NeMo helper to a newer NeMo-Speech.cpp
description: "Move the NeMo helper to a newer NeMo-Speech.cpp revision for the long-recording diarization fix, a faster diarization front end and the newer ggml fixes"
area: processing
added: 2026-10-19
source: "Upkeep review, 2026-10-19 (issue #47, Act now 4–7)"
related: [transcribe-cpp-update]
---

# Update the NeMo helper to a newer NeMo-Speech.cpp

## Idea
Move the NeMo-Speech.cpp revision in `scripts/build-nemo.sh` to a newer upstream commit, and change `scripts/build-nemo.sh` and `scripts/patch-nemo.ts` for the new source layout.

## Why
- Upstream `caa3f47` fixes diarization in a stream longer than about 20 minutes with no finals: older words all get one speaker. Verify the defect in Longhand with a recording longer than 20 minutes first.
- Upstream has a faster diarization front end.
- The ggml in the current pin lacks the GGUF loader overflow and malformed-file fixes, and the Vulkan Flash Attention out-of-bounds fix. The bundled nlohmann json lacks the deep-nesting fix. The risk is low: models are SHA-256 pinned, and the helper reads only the app's own messages (at most 1 MiB).

## Notes / Current state
- NeMo replaced its ggml submodule with one llama.cpp pin. Its current llama.cpp pin does not have all of the ggml GGUF fixes; check the pin at update time.
- ONNX Runtime 1.22.0 and Vulkan-Loader 1.4.309.0 also lack later security fixes. They need a newer `ort` than rc.10 and a newer Vulkan SDK; take them with this update or with `transcribe-cpp-update`, whichever comes first.
