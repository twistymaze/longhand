---
title: Tests with speed limits failed on a loaded CI runner
date: 2026-11-16
category: test-failures
module: Frontend tests (Vitest, Testing Library) and Rust integration tests
problem_type: test_failure
component: testing_framework
severity: high
symptoms:
  - "Main CI failed after PR #95 in two tests that the PR did not change"
  - "A NotesTab test slept 2 s of real time and then expected a reload"
  - "crates/pipeline/tests/in_use.rs gave up at a short deadline while the runner was busy"
root_cause: async_timing
resolution_type: test_fix
tags:
  - test-timing
  - fake-timers
  - vitest
  - testing-library
  - hang-limit
  - ci
---

# Tests with speed limits failed on a loaded CI runner

## Problem

Main CI failed after PR #95 in two tests that the PR did not change. Each test expected an event within a fixed real time: a 2 s sleep, the 1 s default limit of `waitFor`, or a short Rust deadline. A loaded runner can make correct code many times slower, so these tests failed. A sweep found the same pattern in about 30 other test files.

## Root cause

A time limit in a test did two jobs: it stopped a hung test, and it was part of the check ("the reload occurs within 2 s"). The second job makes the result depend on the speed of the machine.

## Solution

PR #96 fixed the two tests. PR #98 applied the same rules to the full suite.

1. **A limit only stops a hung test.** `vite.config.ts` sets `testTimeout` and `hookTimeout` to 60 s. Rust tests use a `HANG_LIMIT` of 60 s. Keep a hang limit below each periodic fallback of the code under test, so that a lost event still fails the test and a timer does not hide it:

   ```rust
   const HANG_LIMIT: Duration = Duration::from_secs(60);
   const { assert!(HANG_LIMIT.as_secs() < REFRESH_EVERY.as_secs()) };
   ```

2. **A test waits for the event that it checks, not for real time.**
   - Frontend: `vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"], shouldAdvanceTime: true })`, then `await vi.advanceTimersByTimeAsync(ms)` to move past the component timer. The Tauri IPC mock resolves on microtasks only, so fake timers do not stop it.
   - Rust: inject the interval or the clock. For example, `Runner::with_intervals(env, power, Intervals { poll, heartbeat })` in `crates/pipeline/src/runner.rs` lets a test use short intervals and wait on a channel or a condition variable.

3. **Testing Library and fake timers.** `src/test/setup.ts` defines a `jest` object with `advanceTimersByTime`, because Testing Library moves fake timers in `waitFor` only when a `jest` global exists. It also sets `asyncUtilTimeout` to 1 s under fake timers (a long limit would let `waitFor` move the fake clock past a timer that the test must not reach) and 30 s under real timers. Set it on the config of `@testing-library/dom`: `getConfig` of `@testing-library/react` returns a copy, and a change to the copy has no effect.

4. **Confirm each changed test with a mutation check.** Break the timer, the wake, or the deadline in the code, and make sure the test fails. A weak mutation can pass: a wake flag that did nothing still let a later notify wake the thread. Only when the wake was removed fully did the test fail, at the hang limit.

## Prevention

When a test needs "after N ms", use a fake clock or an injected interval. When a test needs "eventually", wait for the event with a hang limit of 60 s. Never put a real-time limit in the check itself.

Related: [watcher-test-gave-up-while-fseventsd-was-busy](watcher-test-gave-up-while-fseventsd-was-busy.md), [runner-test-stopped-before-the-title-job-that-the-save-queued](runner-test-stopped-before-the-title-job-that-the-save-queued.md), [failed-expect-on-happy-dom-node-blocks-waitfor](failed-expect-on-happy-dom-node-blocks-waitfor.md).
