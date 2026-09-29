---
title: The PR watcher wakes with base-ref-blocked after every push while CI runs
date: 2026-09-26
category: workflow-issues
module: PR babysit watcher (compound-engineering plugin, ce-babysit-pr scripts/pr-snapshot)
problem_type: workflow_issue
component: development_workflow
related_components:
  - tooling
severity: medium
applies_when:
  - "ce-babysit-pr watches an open Longhand PR and a push or a merge of main changes the PR head"
  - "A pr-snapshot watch run prints BABYSIT_WAKE with reason base-ref-blocked soon after it starts"
  - "A snapshot shows base_ref_blocker \"race\" and mergeability_certain false"
symptoms:
  - "pr-snapshot watch exits within one poll with reason base-ref-blocked while CI still runs"
  - "Each later push gives one more immediate base-ref-blocked wake"
  - "The snapshot shows base_ref_blocker \"race\", mergeability_certain false, merge_state_status BLOCKED or CLEAN"
root_cause: logic_error
resolution_type: workflow_improvement
tags:
  - ce-babysit-pr
  - pr-snapshot
  - base-ref-blocked
  - github-mergeability
  - compound-engineering-plugin
---

# The PR watcher wakes with base-ref-blocked after every push while CI runs

## Context

Longhand PRs use the compound-engineering plugin's `ce-babysit-pr` skill to watch CI. Its detector, `pr-snapshot watch`, polls the PR and prints one `BABYSIT_WAKE` line when the agent must act, then exits.

After a push to the PR branch, or a merge of `main` into it, GitHub keeps the PR's `potentialMergeCommit` on the old base until it rebuilds that commit in the background. During that time `pr-snapshot` reports `base.identity` `"race"`, `base_ref_blocker` `"race"` and `mergeability_certain` false.

The paths below are inside the plugin's `ce-babysit-pr` skill directory (`~/.claude/plugins/cache/compound-engineering-plugin/compound-engineering/3.27.0/skills/ce-babysit-pr/`), not this repo. The skill's own `references/tick.md` calls `race` and `mergeability-pending` transient states that "disable `mergeability_certain` and re-poll". The script does not follow that text (plugin version 3.27.0; line numbers change between versions):

- `_base_ref_blocker` (`scripts/pr-snapshot` near line 487) returns `race` and `mergeability-pending` as blockers.
- `_wake_reason` (near line 2527) returns `base-ref-blocked` for any blocker. It checks this before merge-ready and treats it like `stack-blocked` and `unrequested-base-merge`.
- The watch loop (near line 2945) suppresses a blocker only when its signature was already present when the watcher was armed: `_blocker_sig(actionable) <= armed`. A push gives a new head and so a new signature. The first watcher after each push therefore wakes at once.

A `race` does not last forever in 3.27.0. `_apply_stale_merge_computation` (near line 1118) records when a head and live base pair first shows the race. After `STALE_MERGE_COMPUTATION_SECONDS` (600 seconds, line 89) with no change, the identity becomes `stale-computation`, `_base_ref_blocker` returns no blocker, and a merge-ready wake can follow. Only `race` gets this bound. A `mergeability-pending` blocker has no time limit. An earlier note from PR #6 said the watcher "never" reached merge-ready (auto memory [claude]). That note predates this bound, or the watcher on PR #6 was not re-armed long enough to reach it.

On PR #11 (2026-09-26), two pushes gave two immediate `base-ref-blocked` wakes while CI ran. The Windows CI job takes about 25 minutes, so each wake arrived long before the checks finished.

## Guidance

After a `base-ref-blocked` wake, take one fresh snapshot and read these fields together:

| Field | Ready value |
|---|---|
| `all_checks_ok` | `true` |
| `mergeable` | `MERGEABLE` |
| `merge_state_status` | `CLEAN` |
| `counts` | all zero (empty attention set) |
| `branch_currency_blocker` | `null` |
| `base_ref_blocker` | `"race"` is the only blocker |

- **All fields match:** the PR is ready. Stop watching and report it ready. Do not wait for a merge-ready wake.
- **CI still runs:** re-arm the watcher with the same `--invocation-id`, `--session-started-at` and `--invocation-budget-seconds` values. The race signature is now in the armed set, so the watcher keeps polling. It wakes on a CI result, a comment, or merge-ready after the 600-second bound.
- **The watcher keeps waking on new heads,** because you push often: wait for CI with a bounded loop in a background shell, then take one snapshot. Keep the loop bounded, and do not hide `gh` errors (see the zsh watcher note in auto memory).

```sh
for i in $(seq 1 60); do
  out=$(gh pr checks 11 -R twistymaze/longhand 2>&1)
  if ! echo "$out" | grep -qE "pending|in_progress|queued"; then echo "$out"; exit 0; fi
  sleep 30
done
echo TIMEOUT; echo "$out"
```

Do not edit `scripts/pr-snapshot` in the plugin cache. A plugin update replaces it. The fix belongs upstream: `_wake_reason` should let `race` and `mergeability-pending` fall through and keep polling, and surface them only when all checks are terminal or the 600-second bound expires, as `tick.md` already describes.

## Why This Matters

If an agent reads the wake as a real blocker, it stops watching a PR that only waits for CI, or it reports a false "blocked" state. If it waits for merge-ready without re-arming, it waits on a watcher that already exited. Both waste time on every PR, because every push causes the wake.

## When to Apply

- A `base-ref-blocked` wake arrives within a poll or two after a push, or after a merge of `main` into the branch.
- A snapshot shows green checks and `MERGEABLE`/`CLEAN`, and the only blocker is `race`.
- Before you file or apply an upstream fix, check the plugin version. A later version can change this behavior.

## Examples

- **PR #11, first push:** the watcher woke with `base-ref-blocked` while CI ran. A re-arm with the same invocation values kept it polling. It later woke with `merge-ready`, and the snapshot showed `all_checks_ok` true, `MERGEABLE`, `CLEAN`.
- **PR #11, after merging `main`:** the watcher woke at once with `base-ref-blocked`, `all_checks_ok` false, `race`. The bounded `gh pr checks` loop waited for CI. All checks passed, and auto-merge merged the PR.

## Related

- [The PR watcher looks broken while it waits for its next poll after CI finishes](pr-watcher-seems-silent-after-ci.md): the other causes of a silent watcher, the settle window and the poll interval.
