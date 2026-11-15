---
title: Speaker turns without ONNX Runtime
description: "Speculative: run speaker segmentation and embeddings as GGUF on ggml, so ONNX Runtime and its build steps can go"
area: processing
added: 2026-11-15
source: "upkeep review 2026-11-15: Handy v0.10.0 removed ONNX Runtime (3985508, cfcb650, 5197faf)"
related: [nemo-helper-update]
---

# Speaker turns without ONNX Runtime

## Idea
Run pyannote segmentation and WeSpeaker embeddings (or an equivalent) as GGUF models on ggml, then remove ONNX Runtime.

## Why
Longhand uses ONNX Runtime only for speaker turns (`crates/onnx`, `crates/pyannote-local`, `crates/embedding`). It is a large held dependency: each update needs a signed build test, it ships DirectML on Windows, it has its own security items, and it caused build faults (stale clang_rt.osx). Handy removed it for one engine, a smaller app and fewer build faults.

## Notes / Current state
- Not checked: is there a GGUF port of pyannote or WeSpeaker, or can the diarizer in NeMo-Speech.cpp do the work?
- The replacement must keep the same quality and speed of speaker turns.
