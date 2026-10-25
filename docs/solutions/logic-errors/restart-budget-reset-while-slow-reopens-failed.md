---
title: The microphone restart budget reset while slow reopen attempts failed, so the note was never interrupted
date: 2026-10-24
category: logic-errors
module: Capture source restart loop (crates/supervisor/src/recover.rs, crates/supervisor/src/restart.rs, src-tauri/src/managers/capture.rs)
problem_type: logic_error
component: service_layer
symptoms:
  - "A note stays in recording with no audio after the microphone fails, and no alert appears"
  - "The log shows restart after restart of the microphone, with no end"
root_cause: logic_error
resolution_type: code_fix
severity: high
framework_version: rust 1.95.0
tags: [capture, microphone, restart, budget, recover, supervisor, anarlog, fake-clock, interrupt, alert, fallback]
related_components: [supervisor, audio]
---

# The microphone restart budget reset while slow reopen attempts failed, so the note was never interrupted

## Problem

When the microphone fails during a note, the capture manager reopens it at most three times, then interrupts the note and shows the alert. The budget counted restarts in a time window. When each reopen attempt was slow, the failures spread past the window, the count started again, and the three attempts never ran out.

## Symptoms

- The note stays in recording, but no audio arrives.
- No alert appears, because the note is never interrupted.
- The restarts continue with the 1, 2 and 4 s backoff.

## What Didn't Work

- **The window-based budget.** `RestartTracker::record_restart` starts a new window when the last window is older than `max_window`. The source window is 22 s: 15 s for failures plus 7 s for the backoff sleeps (`crates/supervisor/src/lib.rs:20`, `:21`). The allowance for the sleeps assumes that each attempt fails fast. A reopen can take the 5 s start limit of `audio_cpal` (`crates/audio-cpal/src/startup.rs:13`) or a little more. With 5.5 s attempts, the attempts end at about 6.5 s, 14 s and 23.5 s, so the third failure falls in a new window and the count is 1 again.
- **The quiet-period reset.** The old loop also called `maybe_reset_at` before each restart. That clears the count 30 s after the window start, also while failures continue. It was meant for "the source was quiet", but it measured time since the first failure, not time since capture recorded.
- **A larger window.** Any fixed window has the same fault for slower attempts. The budget must not depend on how long an attempt takes.

## Solution

PR #69 ports the consecutive counter from anarlog commit `693500220d8e253b175e7a5b88fd831c48b4583b`, where upstream also counts consecutive failures and resets after 30 s of healthy capture:

- `RestartTracker::record_consecutive_restart` adds 1 to the count with no window, and `reset` clears it (`crates/supervisor/src/restart.rs:75`, `:80`).
- `SourceRestarts` holds the tracker and the time that capture last came back (`crates/supervisor/src/recover.rs:33`). `recover` counts each restart with `record_consecutive_restart` (`recover.rs:69`). It records the comeback time when an attempt succeeds (`recover.rs:77`). At entry, it resets the count only when capture came back 30 s (`SOURCE_RESTART_BUDGET.reset_after`) or more before now (`recover.rs:61`).
- The capture manager holds `SourceRestarts` in place of the bare tracker. When the chosen microphone goes away and the system default records in its place, no restart runs, so the manager calls `came_back_at` to start the 30 s period again (`src-tauri/src/managers/capture.rs:3605`).

anarlog decides that capture is healthy after 30 s of continuous frames. In Longhand a segment ends on its own only on a loss, so 30 s from a successful reopen to the next loss is the same signal. Per this session's reading of the capture code, a stalled input that delivers no frames does not end a segment, so it starts no restart and does not touch this budget. That gap is separate and exists before this fix.

## Why This Works

The budget now counts what it must stop: failures in a row. Time passes only between losses, and only time with capture recording can clear the count. A microphone that keeps failing runs out three attempts however long each attempt takes. A microphone that comes back and drops again 4 s later still spends the same budget. A microphone that recorded for 30 s gets a full budget at its next loss.

## Prevention

- **Test a restart budget with attempts that take time.** A fake clock that moves only in the backoff sleeps hides this fault, because every failure is then instant. `slow_failed_attempts_still_run_out_the_budget` (`recover.rs:154`) moves the clock 5.5 s in each failed attempt.
- **Make a broken loop fail, not hang.** In that test the source opens from the tenth attempt. The old loop then returns `Resumed` and the test fails. Without that exit, the broken loop would never return and the test would hang.
- **Reset a failure budget on evidence of health, not on elapsed time.** A reset that measures time since the first failure clears the count during the failures that it must count.
- **Every way that capture comes back starts the health period.** A path that reopens capture outside `recover` (such as the fallback to the system default) must call `came_back_at`, or a later loss resets the count from an older comeback time. `a_comeback_on_another_input_starts_the_health_period_again` (`recover.rs:229`) covers this.
- **Known limit.** The health check runs when `recover` starts, after the lost segment is closed. A loss at about 29.8 s after a comeback can cross the 30 s boundary and get a full budget. The source recorded for almost the full period in that case.

## Related

- `docs/solutions/ui-bugs/one-capture-failure-gave-two-messages.md`: the interrupt and alert that this fault kept from appearing.
- `docs/solutions/test-failures/tests-passed-while-the-behaviour-in-their-names-was-broken.md`: tests must fail when the behaviour in their names breaks.
