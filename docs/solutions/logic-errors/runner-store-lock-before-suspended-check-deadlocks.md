---
title: A runner that locked the store before it read capture state could deadlock with a model delete
date: 2026-09-28
category: logic-errors
module: Lock order between the capture manager and the processing runner (src-tauri/src/managers/capture.rs, crates/pipeline/src/runner.rs, src-tauri/src/managers/processing.rs)
problem_type: logic_error
component: service_layer
severity: high
symptoms:
  - Code review found that a model delete during a "replace and reprocess" of a note could freeze the app
  - The delete command runs on the main thread, so the whole app would stop responding
root_cause: concurrency
resolution_type: code_fix
framework_version: rust 1.94.0
related_components:
  - background_job
tags:
  - lock-order
  - deadlock
  - mutex
  - capture-manager
  - runner
  - suspended
---

# A runner that locked the store before it read capture state could deadlock with a model delete

## Problem

The capture manager and the processing runner share one store mutex. The capture manager takes its own state lock (`inner`) and then the store lock. `Runner::reprocess` took the store lock and then called `env.suspended`, which takes the capture state lock. When the two paths ran at the same time, each held the lock that the other needed.

## Symptoms

- No user saw the freeze. The code review of archived PR 19 found it before release.
- The failure path: the user deletes a model in Settings while a note is being reprocessed. `delete_model` is a sync command, so the main thread waits forever, and the app stops responding.

## What Didn't Work

- Reading `reprocess` alone does not show the lock. `(self.env.suspended)()` is a boxed closure. The pipeline crate cannot see that the app implements it with `CaptureManager::is_capturing()`, and that function locks `inner` (`src-tauri/src/managers/processing.rs:522`, `src-tauri/src/managers/capture.rs:1003`).
- Before archived PR 19, `delete_model` took only the store lock, so the cycle did not exist. The cycle started when `delete_model` began to use `CaptureManager::with_recording_held`. That function holds `inner` and then the store through the deletes, so a recording cannot start between the delete checks and the deletes (`src-tauri/src/managers/capture.rs:995`).

## Solution

Read capture state before the store lock, as `transcribe_again` already did (`crates/pipeline/src/runner.rs:426`).

Before:

```rust
let mut store = lock(&self.env.store);
// ...
let capture_active = (self.env.suspended)(); // locks capture `inner` while the store is held
```

After (`crates/pipeline/src/runner.rs:495`):

```rust
// Read the capture state before the store lock, as
// `transcribe_again` does: the capture manager locks its state and
// then the store, and a model delete holds both in that order.
let capture_active = (self.env.suspended)();
let mut store = lock(&self.env.store);
```

## Why This Works

The app has one lock order: the capture state lock (`inner`), then the store lock. `with_recording_held` states it in its doc comment (`src-tauri/src/managers/capture.rs:992`). A deadlock needs one path that takes the two locks in the opposite order. When the runner reads `suspended` before it takes the store lock, it never holds the store lock while it waits for `inner`, so no cycle can form.

The early read can become stale before the store lock is taken. `reprocess` only records the value in the `UserTranscribeAgain` event that it gives to `begin_reprocess` (`crates/pipeline/src/runner.rs:518`). A stale value is also possible in `transcribe_again`, which reads the value the same way. The runner stages call `suspended` again before they run inference, so a recording that starts later still pauses processing (`crates/pipeline/src/runner.rs:242`).

## Prevention

- In the runner, do not call `env.suspended` (or any `Environment` closure that can reach app state) while you hold `lock(&self.env.store)`. Call it first and keep the result.
- In app code that needs capture state and the store together, take `inner` first, then the store. Use `CaptureManager::with_recording_held` and do not write a new pair of lock calls.
- When a change makes a command hold a second lock, find each other path that takes the same two locks. Search for the store lock (`lock(&self.env.store)`, `self.store()`) and for each closure that can lock `inner` (`suspended`, `is_capturing`).
- A unit test with a stub `suspended` (`Box::new(|| false)`, `crates/pipeline/src/runner.rs:1331`) cannot find this deadlock, because the stub takes no lock. Only a review of the lock order finds it.
