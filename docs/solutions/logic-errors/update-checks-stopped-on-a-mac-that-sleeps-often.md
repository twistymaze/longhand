---
title: Automatic update checks stopped for days on a Mac that sleeps often
date: 2026-10-25
category: logic-errors
module: update flow (src-tauri/src/managers/updates/flow.rs, src-tauri/src/managers/updates.rs)
problem_type: logic_error
component: background_job
severity: medium
symptoms:
  - "One Mac did not find a new release for days, until the user clicked \"Check for Updates…\""
  - "The same release showed at once after a manual check or a restart"
  - "The log had no successful automatic check between launches on that Mac"
root_cause: wrong_api
resolution_type: code_fix
tags:
  - updates
  - sleep
  - wake
  - tokio-timer
  - wall-clock
  - retry
  - network
---

# Automatic update checks stopped for days on a Mac that sleeps often

## Problem

The update manager checked 30 seconds after launch, then on a one-hour tokio interval, and once more at each wake. On a laptop that sleeps often, no automatic check succeeded for days. A manual check found the release at once.

## Symptoms

- One of the user's Macs did not show a new release until the user clicked "Check for Updates…". Other Macs found it.
- A restart also found the release, because the check 30 seconds after launch ran with the network up.

## What Didn't Work

- **The interval alone.** `tokio::time::interval` measures time with tokio's `Instant`. On macOS that clock does not advance while the Mac sleeps (the same fact that `summary-in-a-wake-with-no-network-kept-the-default-budget.md` records for the summary retry). "Each hour" meant "after each hour of awake time". A Mac that is awake for 40 minutes and then sleeps never reaches the tick.
- **The check at wake.** The `DidWake` handler sent a check when `NSWorkspaceDidWakeNotification` arrived. That is usually before Wi-Fi reconnects, so the request failed.
- **No retry.** An automatic check that fails shows nothing, on purpose (R12 of the updates plan). The flow also did not schedule another try, so the next attempt was the next interval tick, which the paused interval clock could delay for days.

## Solution

PR #70 moves the schedule into the pure flow and uses UTC time:

- The flow keeps `next_check`, a UTC time in seconds (`flow.rs:194`). It is `None` at launch, so the first tick checks.
- `checked` sets the next due time after each result (`flow.rs:492-502`): one hour after a successful check (`CHECK_INTERVAL_SECONDS`, `flow.rs:15`), or 2, 5, then 15 minutes after failures in a row (`RETRY_SECONDS`, `flow.rs:20`).
- The manager ticks each minute (`TICK`, `updates.rs:39`) and sends `Input::Timer`. The flow starts a check only when it is due (`flow.rs:430-435`).
- A wait longer than the interval means that the clock moved back, so the check is due (`flow.rs:430-435`). Without this rule, a clock that a user or NTP sets back by days would stop the checks again.
- The update call in the `DidWake` handler is gone (`lib.rs:337-341`). The first tick after a wake covers it, and the retry covers a network that is not back yet.

## Why This Works

The schedule now depends on UTC time, which counts sleep, not on a timer that stops during sleep. The tick only gives the flow a chance to decide. Its own clock can pause, but after a wake the next tick comes within one minute of awake time, and the decision uses `Facts.now`. A failure no longer waits for the next hour: it waits 2 minutes, which is long enough for Wi-Fi to reconnect after a wake.

The flow stays pure (it reads no clock), so the unit tests drive the schedule with `facts(now)`: the one-hour wait, a long sleep, the retry steps, and a clock that moves back.

## Prevention

- Do not use a tokio interval or sleep for a schedule that must count real time on macOS. Keep the due time as UTC in pure logic, and tick often.
- Do not make a network request at `NSWorkspaceDidWakeNotification` and stop on failure. Retry with short waits, or let a later tick do the work.
- A schedule that keeps a future UTC time must treat a wait longer than its interval as due, so that a clock set back does not stop it.
- A manual test of this schedule needs a release-flag build, because test builds make no update checks (`manual-test-of-the-updater-needs-a-release-flag-build.md`).

## Related Issues

- `docs/solutions/logic-errors/summary-in-a-wake-with-no-network-kept-the-default-budget.md`: the same tokio clock fact, used there to make a retry wait continue after the next full wake.
- `docs/solutions/logic-errors/update-flow-states-made-while-busy-missed-the-busy-rule.md`: the earlier fix to the same pure update flow.
- `docs/solutions/workflow-issues/manual-test-of-the-updater-needs-a-release-flag-build.md`
