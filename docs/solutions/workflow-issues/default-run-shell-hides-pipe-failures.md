---
title: The default run shell hides a failure in a pipe, so a step passes and does nothing
date: 2026-10-11
module: GitHub Actions CI
problem_type: workflow_issue
component: development_workflow
severity: high
applies_when:
  - "A workflow step on a Linux or macOS runner pipes a command into another command, such as tee or head"
  - "A step writes a decision to GITHUB_OUTPUT, and a later job or step runs only when that output has a value"
symptoms:
  - "The step passes although its first command failed."
  - "The output is empty, so later jobs read it as nothing to do, and the run is green."
root_cause: config_error
resolution_type: config_change
tags:
  - github-actions
  - pipefail
  - shell
  - github-output
  - silent-pass
---

# The default run shell hides a failure in a pipe, so a step passes and does nothing

## Context

PR twistymaze/longhand#2 added the release jobs. Two steps read a decision from a pipe:

- The "Release version" step in the `release-build` job: `node scripts/release-version.ts "$RELEASE_FILES/notes.md" | tee -a "$GITHUB_OUTPUT"` (Bun ran the script until PR #55). `release-publish` runs only when the `due` output is `true`.
- The "Find the release" step in `release-windows.yml`: `TAG=$(gh api ... --jq ... | head -n 1)`. The build runs only when `TAG` is not empty.

A step with no `shell:` key on a Linux or macOS runner runs as `bash -e {0}`, with no `pipefail`. The exit status of a pipe is the status of its last command. `tee` and `head` succeed, so the step passes when `release-version.ts` or `gh api` fails. The output is then empty. An empty `due` means that no release is due, and an empty `TAG` means that no release targets the commit. The run is green, and nothing is published or attached.

The code review of PR #2 found this. Three reviewers found it independently: correctness, reliability, and the cross-model reviewer. The local tests could not show it, because the scripts behave correctly. Only the shell that runs them hides the failure.

## Guidance

Set `shell: bash` explicitly on every job or step that pipes a command into another command. An explicit `bash` runs as `bash --noprofile --norc -eo pipefail {0}`, so a failed command in a pipe fails the step.

For a job, use the job defaults (`release-build` in `.github/workflows/ci.yml`):

```yaml
release-build:
  # An explicit bash sets pipefail, so a failed version script fails its
  # step and does not become "no release due".
  defaults:
    run:
      shell: bash
```

For one step, set `shell: bash` on the step (the "Find the release" step in `.github/workflows/release-windows.yml`).

With `pipefail`, `VAR=$(a | head -n 1)` also fails under `-e`: the status of the assignment is the status of the command substitution, which is now the status of `a`.

## Why This Matters

A failure here does not show as a red run. It shows as a missing release, or a release without its Windows installer, that nobody notices. A guard that turns an error into "nothing to do" is the worst kind of CI fault, because the result looks like correct behaviour.

The checks job already had `defaults.run.shell: bash`, so its steps were safe. New jobs and new workflows do not get that default, so check each new job.

## When to Apply

- Every new job or workflow on a Linux or macOS runner whose steps use `|`.
- Most of all, steps that write to `GITHUB_OUTPUT` or `GITHUB_ENV`, or set a variable that a later `if:` reads.
- Windows `pwsh` steps are a separate case. This note does not cover them.

## Examples

Before (a failed `gh api` gives an empty tag, and the job ends green):

```yaml
- name: Find the release
  run: |
    TAG=$(gh api "repos/$REPOSITORY/releases" --paginate --jq "..." | head -n 1)
```

After:

```yaml
- name: Find the release
  shell: bash
  run: |
    TAG=$(gh api "repos/$REPOSITORY/releases" --paginate --jq "..." | head -n 1)
```

Related: [A GitHub Actions context in the wrong key fails the workflow before any job starts](actions-context-not-available-fails-before-jobs.md) is the opposite failure in the same files: it is loud, and this one is silent.
