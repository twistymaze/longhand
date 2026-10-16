---
title: A microphone start that ended with no result freed its permit after the wait returned
date: 2026-10-15
category: logic-errors
module: Microphone start permit (crates/audio-cpal/src/startup.rs)
problem_type: logic_error
component: service_layer
severity: medium
symptoms:
  - "The test a_report_in_time_returns_its_result failed on the Windows CI runner: assertion failed: !FLAG.load(Ordering::SeqCst)"
  - A start directly after a start that ended with no result could fail with "an earlier microphone start has not returned"
root_cause: concurrency
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - drop-order
  - raii
  - permit
  - mpsc
  - timing-race
---

# A microphone start that ended with no result freed its permit after the wait returned

## Problem

`start` in `crates/audio-cpal/src/startup.rs` runs the start body on a capture thread and waits on a channel. The body holds a `Starting` value with two fields: the channel sender and the permit. When the body ends with no report, `Starting` is dropped. Rust drops struct fields in declaration order, and the sender was declared first. So the wait saw the closed channel and returned an error before the permit was free. The test checked the permit after `start` returned, so it passed or failed by thread timing. It failed on Windows CI in PR #23.

## Symptoms

- A test that checks the permit after a start with no report fails only on some runs.
- In the app, a retry at once after such a start could fail because the permit was still taken.

## Solution

Declare the permit before the sender, with a comment that names the reason:

```rust
pub(crate) struct Starting<T> {
    // Rust drops fields in declaration order. The permit is first, so it
    // is free before the closed channel ends the wait.
    permit: Permit,
    report: SyncSender<Result<T, Error>>,
}
```

`Starting::report` already freed the permit before it sent. The drop path now has the same order.

## Why This Works

The closed channel is the event that ends the wait. When the permit is freed before that event, each caller that sees the wait end also sees a free permit. The test now checks a state that always holds.

## Prevention

- When a struct holds a guard (a permit, a lock) and a channel end, and a drop of the channel signals another thread, declare the guard first. A field reorder can then break the order, so write a comment next to the fields.
- Use the same order in the explicit path and the drop path.
- A test that passes or fails on thread timing shows a missing happens-before relation. Find the event that the test must wait for. Do not run CI again.

## Related Issues

- [runner-store-lock-before-suspended-check-deadlocks.md](runner-store-lock-before-suspended-check-deadlocks.md): another order rule between two resources.
