---
title: Live readers missed the last chunk, and a dropped live worker left its readers waiting
date: 2026-10-17
category: logic-errors
module: Live workers (crates/pipeline/src/live.rs, crates/pipeline/src/live/features.rs, crates/pipeline/src/recognition/nemo.rs)
problem_type: logic_error
component: service_layer
severity: medium
symptoms:
  - "The live speaker features or the live NeMo text did not include the last audio of a segment"
  - "After capture closed a segment with an error, the live speaker and NeMo workers did not stop"
root_cause: concurrency
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - live-worker
  - capture-end
  - race
  - ordering
  - nemo
  - diarization
---

# Live readers missed the last chunk, and a dropped live worker left its readers waiting

## Problem

The live speaker worker (`extract` in `crates/pipeline/src/live/features.rs`) and the live NeMo worker (`run_live` in `crates/pipeline/src/recognition/nemo.rs`) read the archive of the segment while capture writes it. Capture closes the last chunk, and then it sets the capture end. The readers read the next frame first and then the end. This order has a race:

1. The reader reads and finds no frame. Capture has not closed the last chunk yet.
2. Capture closes the last chunk and sets the end.
3. The reader reads the end, finds it set, and finishes. It does not read the last chunk.

Capture drops a `LiveWorker` with no `end` call when it closes a segment with an error. Nothing then set the capture end, so the readers waited for an end that never came (PR #30).

## Solution

One helper decides the next step of each reader. It reads the end **before** the frame:

```rust
pub(crate) fn next_step<T, E>(
    end: &CaptureEnd,
    read: impl FnOnce() -> Result<Option<T>, E>,
) -> Result<Step<T>, E> {
    let ended = end.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    Ok(match read()? {
        Some(frame) => Step::Frame(frame),
        None if ended => Step::Finish,
        None => Step::Wait,
    })
}
```

If the end was set before the read, the last chunk was closed before the read, so a read that finds no frame means that no frame is left. If the end is set during the read, the reader waits and reads again.

`impl Drop for LiveWorker` sets the capture end to `LiveEnd::Keep` when it is `None`, as a closed channel does. The readers then finish.

## Prevention

When a writer writes data and then sets a "done" flag, a reader must read the flag first and the data second. Finish only when the flag was set before a read that found nothing. Put the order in one shared helper, so each reader uses the same order. A worker that other threads wait on must set its end in `Drop` as well as on the normal path.
