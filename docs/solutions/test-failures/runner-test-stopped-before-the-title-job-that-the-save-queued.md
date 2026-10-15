---
title: A runner test stopped the runner at a save and raced the title job that the save queued
date: 2026-10-15
category: test-failures
module: Processing runner tests (crates/pipeline/tests/runner.rs, crates/pipeline/src/runner.rs)
problem_type: test_failure
component: testing_framework
severity: medium
symptoms:
  - "Test a_title_job_does_not_wait_for_the_audio_stages_of_another_note failed on Windows CI with left [(\"s\", \"Weekly sync\"), (\"b\", \"Weekly sync\")] and right [(\"s\", \"Weekly sync\")]"
  - The test failed twice in a row on Windows CI on a PR that changed no file in crates/pipeline
  - The test passed 15 of 15 local runs on macOS, and it passed on Windows CI on main
root_cause: async_timing
resolution_type: test_fix
tags:
  - windows-ci
  - race
  - runner
  - title-job
  - shutdown
  - test-timing
---

# A runner test stopped the runner at a save and raced the title job that the save queued

## Problem

The test waited for the `Saved` event of note B, then stopped the runner and asserted that only note "s" had a proposed title. But the save of B also queues the title job of B. On a slow runner, that title job sent its proposal before the stop, so the assertion failed. The test depended on the order of two threads, not on an event.

## Symptoms

- Windows CI failed in `crates/pipeline/tests/runner.rs` with one extra `("b", "Weekly sync")` entry in the proposed titles.
- The PR changed no code on this path. The test passed on macOS and on `main`.

## What Didn't Work

- **A rerun of the failed CI job.** The rerun failed in the same way. A rerun does not fix a race; it only hides it when the order of threads is lucky.
- **Calling the failure flaky and unrelated to the PR.** The first diagnosis said this and asked for a rerun. The failure was real: the test was wrong. `CLAUDE.md` now forbids this response to a timing failure.
- **Looking for the cause in the PR's changes.** The PR changed the VAD crate, and `pipeline` depends on `vad` through `asr`. But this path uses `EarshotVad`, not the changed `SmoothedVad`, so the PR did not change the timing of note B.

## Solution

Wait for the event that the assertion checks, then stop the runner, and assert the full expected result:

```rust
// Before: the test stopped the runner at the save of B.
until("B is saved", || event_at(&r, saved_event("b")).is_some()).await;
r.runner.shutdown();
assert_eq!(proposed_titles(&r), [("s".to_string(), "Weekly sync".to_string())]);

// After: the test waits for the title job that the save queued.
until("the title of B", || proposed_titles(&r).len() == 2).await;
r.runner.shutdown();
assert_eq!(
    proposed_titles(&r),
    [
        ("s".to_string(), "Weekly sync".to_string()),
        ("b".to_string(), "Weekly sync".to_string())
    ]
);
```

The check of KTD13 does not change: the title call of "s" comes while B is held in its speaker stage. The new assertion is stricter, because it also checks the order of the proposals and that B gets its own title.

## Why This Works

When a note finishes, `run_claimed` emits `Finished` and then calls `queue_title_if_pending` (`crates/pipeline/src/runner.rs:1506-1511`). The save emits `Saved` before that (`crates/pipeline/src/runner.rs:1892`). So when the test sees `Saved`, the title job of B is queued or about to be queued. The runner and the mock endpoint run in the same process, so the whole title job can finish in the time between the `until` poll and `shutdown()`. A slower CI runner makes this more likely. The fixed test waits for the last event of the work. After that event, the stop cannot cancel any work that the assertion depends on.

## Prevention

- In a runner test, find all work that the waited-for event queues. `Saved` and `Finished` queue the title job. If the test stops the runner after such an event, the test must wait for that queued work, or the note must queue no such work. For example, a note with a user title stores no title request.
- To prove a suspected race on macOS, add a short sleep, such as 300 ms, between the wait and the stop or the assertion. This test then failed locally in the same way as on Windows CI. Remove the sleep after the fix, and check that the fixed test also passes with it.
- Do not run CI again to get a pass for a timing failure. Fix the test in the same PR, also when the PR did not cause the race (`CLAUDE.md`, "Working in this repo").
