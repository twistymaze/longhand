---
title: A dispatched release build could write the main branch caches from any commit
date: 2026-10-19
category: security-issues
module: Windows release workflow (.github/workflows/release-windows.yml find job, .github/actions/windows-build-setup)
problem_type: security_issue
component: development_workflow
severity: medium
symptoms:
  - "Code scanning alerts #1, #3, #4, #5 (`actions/cache-poisoning/poisonable-step`, high) on release-windows.yml and windows-build-setup/action.yml"
  - "The alert text: potential cache poisoning in the default branch context, because a workflow_dispatch run checks out untrusted code from `needs.find.outputs.commit`"
  - "After the fix merged, the CodeQL `actions` analysis of the merge commit on main still showed all four alerts open"
root_cause: missing_validation
resolution_type: config_change
related_components:
  - infrastructure
tags:
  - github-actions
  - codeql
  - code-scanning
  - cache-poisoning
  - workflow-dispatch
  - release-build
  - false-positive
---

# A dispatched release build could write the main branch caches from any commit

## Problem

`release-windows.yml` can run by `workflow_dispatch` with a release tag. The find job read the release's `target_commitish`, and the build job checked out that commit and ran steps that save caches (rust-cache, sccache, bun) in the scope of the default branch. Pull request runs restore the sccache and bun entries, and later release runs restore the rust-cache entry, which has its own release key. So a release whose target was a commit that is not on `main` could put untrusted build output into caches that later runs trust. Only a user with write access can make a release or dispatch the workflow, so the risk was limited.

## Symptoms

- CodeQL default setup, which scans GitHub Actions workflows too, was turned on on 2026-10-19 (session history). Its first analyses reported five high alerts of the rule `actions/cache-poisoning/poisonable-step`. Alert #2 was on `upkeep-licences.yml`. The other four concern this build: one on the `bun install` step in `.github/actions/windows-build-setup/action.yml`, and three on the setup, sccache and NeMo build steps of `release-windows.yml`.
- After PR #49 merged, the CodeQL `actions` analyses of the merge commit and of the next commit on `main` still reported all four alerts open.

## What Didn't Work

- A fix in the workflow did not close the alerts. CodeQL flags the pattern (a dispatch input flows into `actions/checkout` `ref:`, then cache steps run). It does not model a shell step that stops the job for a commit that is not on `main`. The alerts stayed open on the first analysis after the merge.

## Solution

The find job resolves the release target to a SHA and checks with the GitHub compare API that the SHA is on `main`. If not, the job fails, and the build job does not run (PR #49). The guard is at `.github/workflows/release-windows.yml:76` to `:81`:

```bash
commit=$(gh api "repos/$REPOSITORY/commits/$commit" --jq .sha)
status=$(gh api "repos/$REPOSITORY/compare/$commit...main" --jq .status)
if [ "$status" != "ahead" ] && [ "$status" != "identical" ]; then
  echo "::error::Commit $commit of release $TAG is not on main ($status). Nothing to build."
  exit 1
fi
```

Then the alerts were dismissed as false positives, with a comment that names PR #49:

```bash
gh api -X PATCH repos/twistymaze/longhand/code-scanning/alerts/<n> \
  -f state=dismissed -f dismissed_reason='false positive' \
  -f dismissed_comment='PR #49: the find job fails unless the release commit is on main ...'
```

## Why This Works

- `compare/<commit>...main` gives `ahead` when `main` contains the commit and has more commits, and `identical` when they are the same commit. `behind` and `diverged` mean that the commit is not on `main`. A manual check on 2026-10-19 gave `ahead` for an older `main` commit and `identical` for the tip of `main`.
- The step resolves the target to a SHA first, because `target_commitish` can be a branch name. The build job checks out the same SHA from the job output, so a branch that moves between the two jobs cannot change what is built.
- The find step runs under `shell: bash` (`-eo pipefail`, see [default-run-shell-hides-pipe-failures](../workflow-issues/default-run-shell-hides-pipe-failures.md)). A failed `gh api` call in a `$( )` assignment fails the step, so an API error does not let the build run.
- The `workflow_run` path builds `head_sha` of a CI run for a push to `main`, so it always passes the check.

## Prevention

- A workflow that takes a ref as input and then saves caches must check that the ref is on the default branch before the checkout, or it must not save caches.
- For an `actions/cache-poisoning` alert, expect CodeQL to keep the alert open after a guard of this kind. Confirm on the first analysis of `main` after the merge, then dismiss the alert as a false positive and name the PR that adds the guard. Do not dismiss an alert before the guard is on `main`.
- A dispatch from another branch uses that branch's copy of the workflow, which may not have the guard. Its caches have the scope of that branch, so they do not reach the default branch caches.

## Related

- [pr-bunfig-controls-base-check-scripts](pr-bunfig-controls-base-check-scripts.md): another case where an untrusted checkout could change what a trusted CI step does.
- [sccache-compile-cache-experiment](../performance-issues/sccache-compile-cache-experiment.md): the cache setup of the release build.
