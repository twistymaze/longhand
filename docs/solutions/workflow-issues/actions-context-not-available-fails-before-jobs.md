---
title: A GitHub Actions context in the wrong key fails the workflow before any job starts
date: 2026-09-28
module: GitHub Actions CI
problem_type: workflow_issue
component: development_workflow
severity: medium
applies_when:
  - "A workflow change uses a context such as runner, env, secrets or steps in an expression"
  - "A PR shows required checks as Expected, or a run shows only a workflow file issue"
symptoms:
  - "The run failed with a workflow file issue and created no jobs."
  - "The PR showed the required checks as Expected, not as failed."
root_cause: config_error
resolution_type: config_change
tags:
  - github-actions
  - actionlint
  - runner-temp
  - context-availability
  - ci
---

# A GitHub Actions context in the wrong key fails the workflow before any job starts

## Context

GitHub Actions makes each context available only in some workflow keys. The
`runner` context is not available in a job-level `env` or in a step's `shell`.
When a workflow uses a context in a key that does not allow it, GitHub rejects
the whole file. No job starts, and the log shows no step output.

This occurred twice in Longhand:

- A `runner.os` expression in a step's `shell` field. The
  [Windows linker solution](../build-errors/windows-rust-selects-git-linker-after-msvc-setup.md)
  records it.
- `${{ runner.temp }}` in the preview job's job-level `env` (archived PR 22). A branch
  push showed the run as "workflow file issue".

A YAML parser accepts both files. `actionlint` finds both errors, because it
checks each context against the key that uses it.

## Guidance

Run `actionlint` on each workflow change before the push:

```sh
actionlint .github/workflows/ci.yml
```

When a job needs a path under the runner's temporary directory, set it in the
first step and write it to `$GITHUB_ENV`. Later steps read it as an ordinary
environment variable. Keep only constant values in the job-level `env`.

## Examples

Before (GitHub rejects the file):

```yaml
preview:
  env:
    KEYCHAIN: ${{ runner.temp }}/preview.keychain-db
```

After (`.github/workflows/ci.yml`, the preview job):

```yaml
preview:
  env:
    BUILDS_REPOSITORY: twistymaze/longhand-builds
  steps:
    - name: Record the job start and the temporary paths
      run: |
        {
          echo "KEYCHAIN=$RUNNER_TEMP/preview.keychain-db"
          echo "APPLE_API_KEY_PATH=$RUNNER_TEMP/AuthKey.p8"
        } >> "$GITHUB_ENV"
```

## Why This Matters

The failure gives no job log, so the run does not show the cause. On a PR, the
required checks stay "Expected" and do not fail, so the error is easy to miss.
`actionlint` finds it locally in seconds.
