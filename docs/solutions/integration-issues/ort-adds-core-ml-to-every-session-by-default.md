---
title: ort 2.0.0-rc.15 adds Core ML to every session unless the builder sets PreferCPU
date: 2026-10-24
category: integration-issues
module: ONNX sessions (crates/onnx/src/lib.rs)
problem_type: integration_issue
component: service_layer
severity: high
symptoms:
  - "A verbose ONNX Runtime session log shows 'Node(s) placed on [CoreMLExecutionProvider]' for a session that did not add Core ML"
  - The CPU fallback after a Core ML failure can fail again on Core ML
  - Speaker segmentation and the speech detector run partly on Core ML, also in a build without the onnx coreml feature
root_cause: wrong_api
resolution_type: code_fix
related_components:
  - speaker recognition (crates/pyannote-local, crates/embedding)
tags:
  - ort
  - onnx-runtime
  - coreml
  - execution-provider
  - cpu-fallback
  - dependency-update
---

# ort 2.0.0-rc.15 adds Core ML to every session unless the builder sets PreferCPU

## Problem

PR twistymaze/longhand#67 updated ort from 2.0.0-rc.10 to 2.0.0-rc.15 (ONNX Runtime 1.31). In rc.15, each new `SessionBuilder` sets the automatic device policy `AutoDevicePolicy::MaxEfficiency`. On macOS, this policy adds the Core ML execution provider to the session. Longhand's plain sessions and its CPU fallback did not stay on CPU.

## Symptoms

- A session that Longhand builds with no execution provider has some nodes on Core ML. The segmentation model had 5 Core ML partitions (9 of 50 nodes).
- The cargo feature `onnx/coreml` does not change this. The pyke ONNX Runtime binaries for macOS contain Core ML, and the policy uses it.
- `BackendSession::load_with` and `run_with_cpu_fallback` load a "CPU" session after a Core ML failure. That session also gets Core ML, so a Core ML fault can occur again in the fallback.
- No test fails, and the results do not change in a visible way. In this session, diarization output with the partial Core ML sessions was byte-identical to the CPU output.

## What Didn't Work

- The release notes and the API changes (`Session::builder(&env)`, `ort::ep`, the generic builder error) did not mention the new default. The compile errors showed only the API changes.
- Workspace tests, clippy and a diarization comparison on 4 files all passed. The comparison could not find the problem, because Core ML output matched CPU output for these models.
- The "CPU" comparison run in this session used a build without the `coreml` feature. Its log still had Core ML warnings, but they were not checked at the time.
- The cross-model code review (Codex) found the problem. It read the ort source, not the Longhand diff only.

## Solution

`session_builder()` in `crates/onnx/src/lib.rs` sets the CPU policy. `commit_accelerated()` still adds Core ML explicitly, and an explicit provider has priority over the policy.

```rust
// Before (ort rc.15 default: MaxEfficiency, which adds Core ML on macOS)
Ok(Session::builder(environment()?)?
    .with_intra_threads(1)?

// After
Ok(Session::builder(environment()?)?
    .with_auto_device(AutoDevicePolicy::PreferCPU)?
    .with_intra_threads(1)?
```

## Why This Works

`SessionBuilder::new` in ort rc.15 calls `SessionOptionsSetEpSelectionPolicy(..., AutoDevicePolicy::MaxEfficiency)` when the `api-22` feature is on. The ort default features turn on `api-22` and later. The ort documentation of `with_auto_device` says that `PreferCPU` "will never automatically register any EPs", and that providers added with `with_execution_providers` always take precedence. So `PreferCPU` keeps plain sessions on CPU, and the accelerated path keeps Core ML.

## Prevention

- After each ort or ONNX Runtime update, read `SessionBuilder::new` and `with_auto_device` in the new ort source. Check the default policy.
- Check where a session runs with a verbose session logger, not with timings. A small program that builds a session with `.with_log_level(LogLevel::Verbose)` and `.with_logger(...)` prints "Node(s) placed on [CoreMLExecutionProvider]" when Core ML has nodes. With `PreferCPU`, the log has only "All nodes placed on [CPUExecutionProvider]".
- Every new session in Longhand goes through `session_builder()`. Do not call `Session::builder` directly in another crate, or the session gets the ort default.
- On Windows, the same default can select DirectML or an NPU. `PreferCPU` prevents that too.
