---
title: A store lock in an if-let condition stayed held in its block, so a title job waited for ever and held the summary queue
date: 2026-10-18
category: runtime-errors
module: Title job of a saved note (crates/pipeline/src/runner.rs send_title)
problem_type: runtime_error
component: background_job
symptoms:
  - "After a quit during a summary, the next launch sent the saved title request, and Regenerate Summary cancelled it"
  - "The note then showed 'Waiting to summarize' for ever, and no request went to the endpoint"
  - "The app log ended at 'request end: ... title ... cancelled' with nothing after it"
root_cause: logic_error
resolution_type: code_fix
severity: high
framework_version: rust 1.95.0
last_updated: 2026-10-18
tags: [deadlock, mutex, store-lock, if-let, temporary, edition-2024, title-job, summary-queue, regeneration, hang, test-hang, abort]
related_components: [pipeline, background_job]
---

# A store lock in an if-let condition stayed held in its block, so a title job waited for ever and held the summary queue

## Problem

When a regeneration stopped a title job that had sent its request, the job ran this code:

```rust
if let TitleDue::Dropped = title_due(&lock(&env.store), claims, session_id) {
    drop_title_request(&lock(&env.store), session_id);
}
```

The guard from the first `lock` is a temporary of the `if let` condition. Rust keeps such a temporary until the end of the `then` block (edition 2024 drops it before the `else` block only). The second `lock` in the block then waits for the first guard in the same thread, and a `std::sync::Mutex` waits for ever. The job never ended, so it kept its turn in the summary queue, and the regeneration waited for that turn.

## Symptoms

- The user quit the app during a summary. At the next launch, the title job sent the saved title request.
- Regenerate Summary cancelled the title request (`request end: ... title ... cancelled` in the app log), and the note showed "Waiting to summarize" with no more log lines.

## Solution

Take one guard for the check and the drop:

```rust
let store = lock(&env.store);
if let TitleDue::Dropped = title_due(&store, claims, session_id) {
    drop_title_request(&store, session_id);
}
```

Before you write `lock(...)` inside an `if let` or `match` condition, look at the block: if the block takes the same lock, bind the guard to a variable first, and use it in the block.

## A test for a deadlock must not hang

The test `a_regeneration_during_the_title_turn_stops_the_title_job_and_gets_the_turn` waits 30 s for the title job. With the fault, a panic at the timeout does not end the test: the store lock stays held, and the Tokio runtime waits for the blocked `spawn_blocking` thread when it is dropped. So the test calls `std::process::abort()` at the timeout, with a message. The test binary then fails at once with SIGABRT, and CI does not wait for its job timeout.

## Prevention

- Search for this pattern with `grep -rn -A3 "if let .*(&lock(\|match .*(&lock(" crates src-tauri/src` and look for a second `lock` in the block.
- A test that can deadlock on a lock that the runtime waits for aborts the process at its timeout; it does not panic.
