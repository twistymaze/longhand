---
title: The PR watcher looks broken while it waits for its next poll after CI finishes
date: 2026-09-29
category: workflow-issues
module: PR babysit watcher (compound-engineering plugin, ce-babysit-pr scripts/pr-snapshot)
problem_type: workflow_issue
component: development_workflow
related_components:
  - tooling
severity: medium
applies_when:
  - "ce-babysit-pr watches an open Longhand PR and CI still runs"
  - "The user asks for status while pr-snapshot watch runs and shows no output"
  - "gh pr checks shows all checks green but no BABYSIT_WAKE line has arrived"
symptoms:
  - "No BABYSIT_WAKE line arrives for minutes after all checks are green"
  - "The user asks \"still waiting?\" or says the watcher is broken"
  - "The agent stops a healthy watcher shortly before its next poll"
root_cause: config_error
resolution_type: workflow_improvement
tags:
  - ce-babysit-pr
  - pr-snapshot
  - merge-ready
  - settle-seconds
  - poll-interval
  - compound-engineering-plugin
---

# The PR watcher looks broken while it waits for its next poll after CI finishes

## Problem

Longhand PRs use the `ce-babysit-pr` skill to wait for CI. Its detector, `pr-snapshot watch`, polls the PR and prints one `BABYSIT_WAKE` line only when there is work or a stop condition. Then it exits. It shows nothing while it waits. Longhand PRs have no reviewers, so the only wait is CI.

A `merge-ready` wake can arrive later than CI completion by up to one poll interval, plus the settle window. While the watcher is silent, the user and the agent can think that it is broken.

The paths below are in the plugin's `ce-babysit-pr` skill directory (`~/.claude/plugins/cache/compound-engineering-plugin/compound-engineering/3.27.0/skills/ce-babysit-pr/`), not in this repo. Line numbers are for plugin version 3.27.0 and can change between versions.

- `cmd_watch` (`scripts/pr-snapshot` line 2829) runs a loop. Each pass calls `_wake_reason` (line 2907), then waits `min(args.interval, remaining)` seconds (lines 2960-2962) before the next snapshot (line 2967). The watcher does not see a CI result until the next poll.
- `--interval` has a default of 150 seconds (line 3517). The next poll after arm time is at about arm time + k × interval. Each fetch adds a small drift.
- `_wake_reason` (line 2527) returns `merge-ready` only when all of these conditions are true (lines 2586-2591): `mergeability_certain`, `mergeable` is `MERGEABLE`, `merge_state_status` is `CLEAN`, `checks_terminal`, no failing checks, no checks that wait for approval, no `branch_currency_blocker`, and `quiet_seconds >= settle_seconds`.
- `quiet_seconds` is the time since `last_change_at` (line 2160). A check completion is a change, so the settle window starts again when CI finishes.
- `--settle-seconds` has a default of `DEFAULT_SETTLE_SECONDS = 300.0` (line 96, option at line 3521).

With the defaults, the worst-case delay from the last check completion to `merge-ready` is about 300 + 150 seconds.

A different false wake, `base-ref-blocked` after each push, is in [The PR watcher wakes with base-ref-blocked after every push while CI runs](pr-snapshot-base-ref-race-false-wake.md).

## Symptoms

The same question came from the user four times in six days (session history):

- **2026-09-24, archived PR 2.** The user said: "Your babysitter is broken; the CI is already done." The watcher was in the default 300-second settle window.
- **2026-09-25/26, archived PR 6.** The user asked why the watcher took so long. `main` had moved during CI, and the watcher reported a base-ref race. That cause is in the race doc linked above.
- **2026-09-28, archived PR 20.** The user asked: "CI is done. is your watcher broken?" The settle window again. A memory to skip the settle wait existed, but the agent did not apply it when it armed the watcher.
- **2026-09-29, archived PR 25.** The watcher had `--settle-seconds 0`. After a second push, the agent armed it again at 16:22:52Z. The Windows CI job takes about 16-17 minutes and completed at 16:39:03Z. The user asked "still waiting?". The agent ran `gh pr checks`, saw green checks, told the user that the watcher had failed, and stopped it at 16:39:54Z.

## What Didn't Work

- **The default settle window.** On a PR with no reviewers, the 300-second quiet window has no purpose. It only adds 5 minutes after CI.
- **A memory that says "skip the settle wait" (session history).** On 2026-09-28 the memory existed, but nothing made the agent apply it at arm time. The memory now names the flag, `--settle-seconds 0`. On archived PR 25 the agent passed the flag.
- **A diagnosis from `gh pr checks` alone.** On archived PR 25 the watcher was healthy. Polls after 16:22:52Z fell near 16:25:22, 16:27:52, and so on to 16:37:52 and 16:40:22Z. The checks completed at 16:39:03Z, after the 16:37:52Z poll. The next poll, about 30 seconds after the agent stopped the watcher, would have emitted `merge-ready`. In `state.json`, `last_change_at` was 16:39:54Z. That value came from the agent's own manual snapshot, not from the watcher. All the merge-ready conditions held. The agent compared the check result with "no wake yet" and did not compare it with the poll schedule.
- **Silence during a long wait.** The user saw no output for about 17 minutes. Without an expected finish time, the silent wait looked like a hang.

## Solution

1. Arm the watcher with `--settle-seconds 0`. Longhand PRs have no reviewers, so no quiet window is necessary.
2. When you arm the watcher, tell the user the expected CI finish time. The Windows job takes the longest: expect about 17 minutes after the push.
3. When the user asks for status, run `gh pr checks`. Report which checks still run and when you expect each one to finish.
4. Before you say that the watcher is broken, compare three times: the last check completion, the watcher arm time, and the interval. The next poll is at about arm time + k × interval. If that poll has not occurred yet, the watcher is not broken.
5. If all checks are green and the next poll is far away, take one `pr-snapshot snapshot`. If it shows the merge-ready fields, report the PR ready yourself, then stop the watcher.

## Why This Works

The watcher is a poller. Its latency is one interval plus the settle window, and it prints nothing until it wakes. A settle value of 0 removes the second term, so the delay after CI is at most one interval (150 seconds). A stated CI finish time tells the user what the silence means. A check of the poll schedule separates "not polled yet" from "broken". A manual snapshot gives the same merge-ready answer as the watcher without the wait for the next poll.

## Prevention

- Always pass `--settle-seconds 0` when you arm `pr-snapshot watch` for a Longhand PR.
- When you arm or re-arm the watcher, say when CI will probably finish. Use the Windows job time (about 17 minutes after the push).
- Do not call the watcher broken until the time since the last check completion is more than one interval plus the settle value.
- If you stop the watcher early, first take a snapshot and report the result. Do not report a failure that you did not confirm.
- Do not edit `scripts/pr-snapshot` in the plugin cache. A plugin update replaces it.
