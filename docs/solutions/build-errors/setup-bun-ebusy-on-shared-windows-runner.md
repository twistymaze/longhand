---
title: setup-bun fails with EBUSY when two Windows runners share one user
date: 2026-10-12
category: build-errors
module: CI Bun setup (.github/actions/windows-build-setup, .github/workflows/ci.yml, release-windows.yml)
problem_type: build_error
component: ci
related_components:
  - tooling
severity: medium
applies_when:
  - "Two or more self-hosted runners run as the same OS user on one machine"
  - "A workflow step installs a tool into the user's home folder"
symptoms:
  - "Checks (windows-latest) fails in the setup-bun step after about 30 seconds"
  - "Error: EBUSY: resource busy or locked, copyfile '...\\bun-windows-x64\\bun.exe' -> 'C:\\Users\\runner\\.bun\\bin\\bun.exe'"
  - "The other Windows runner was busy with a job at the same time"
root_cause: shared_mutable_state
resolution_type: config_change
tags: [ci, self-hosted-runner, windows, bun, setup-bun]
---

# setup-bun fails with EBUSY when two Windows runners share one user

## Status
Superseded on 2026-10-20 by PR #55. Longhand does not use Bun or `setup-bun` now. The `.github/actions/node-setup` action installs the npm version from `devEngines` into `$RUNNER_TEMP/npm-tool`, a folder of the job, not of the user. So two jobs do not write the same npm files. The Prevention rule about runners that share a user still applies to each tool that a step installs into the home folder.

## Problem
The Windows check failed twice in the `setup-bun` step with `EBUSY` on `C:\Users\runner\.bun\bin\bun.exe`. No build step ran. A rerun passed.

## Cause
`oven-sh/setup-bun` always installs into `~/.bun/bin` (`join(homedir(), ".bun", "bin")`). Both Windows runners run as the user `runner` on one VM, so they share that `bun.exe`.

With no `bun-version` input, the requested version is `latest`. The action never matches `latest` to an existing install, so each job downloads Bun and replaces `bun.exe`. Windows does not let a process replace an executable that another process runs. When a job on the other runner runs Bun (`bun install`, `bun test`, the Tauri build), or when an earlier job left a `bun` process alive, the replace fails with `EBUSY`.

## Fix
`package.json` has `"packageManager": "bun@<version>"`, and every `setup-bun` step has `bun-version-file: package.json`. When `bun.exe` already has the requested version, the action uses it and does not write the file. So a busy or leftover `bun.exe` no longer stops the job.

The pin did not stop all failures. Other files that the two runners shared also failed when two jobs ran at the same time, for example a rename with `EPERM` in the NeMo packaging step. On 2026-10-12 the second Windows runner was removed. The machine has one runner now, so Windows jobs run one at a time and a second job waits.

## Prevention
- Change the Bun version only in `packageManager`. The first job after a change, or after a reset of the runner machine, installs that version into `~/.bun/bin`. The machine image has no Bun, so it never holds an old version.
- Do not put two runners on one machine as the same user. They share every tool that a step installs into the home folder. To run two jobs at the same time, give each runner its own user or its own machine.
