---
title: A PR's bunfig.toml controlled the base check scripts that judge the PR
date: 2026-10-18
category: security-issues
module: GitHub Actions CI (cargo deny job, .github/workflows/ci.yml; scripts/check-dependency-age.ts, scripts/check-new-advisories.ts)
problem_type: security_issue
component: development_workflow
severity: high
symptoms:
  - "Code review found that the required `cargo deny` job ran `bun base/scripts/<check>.ts` with the PR checkout as the working directory"
  - "A local test with a head bunfig.toml `preload` that replaced process.exit made a base script that exits 1 exit 0"
  - "`bun audit` for the head ran in the head checkout, so the head bunfig.toml or .env could change how the audit runs"
root_cause: config_error
resolution_type: config_change
related_components:
  - tooling
tags:
  - github-actions
  - cargo-deny
  - bun
  - bunfig
  - npm
  - base-checkout
  - pull-request
  - required-check
  - supply-chain
---

# A PR's bunfig.toml controlled the base check scripts that judge the PR

## Problem

The required `cargo deny` job checks the dependency versions that a PR adds. It runs the base branch's copy of each check script, so that a PR cannot change the code that judges it. The first version ran the base script from the PR checkout. bun reads `bunfig.toml` and `.env` from its working directory, so the PR could still run its own code inside the base script.

## Symptoms

- Before the review fix (an earlier commit of PR #44; the squash merge keeps only the fixed form), the step was `bun base/scripts/check-dependency-age.ts base .`, with no `working-directory`. The working directory was the PR checkout.
- A test reproduced the bypass on bun 1.4.0. The test put a `bunfig.toml` with `preload = ["./evil.ts"]` in the head checkout, and `evil.ts` replaced `process.exit`. The base script then called `process.exit(1)`, and bun exited 0.
- `scripts/check-new-advisories.ts` ran `bun audit --json` in the head checkout. The head `bunfig.toml` or `.env` could change how the audit runs, for example which registry answers it (per this session's conclusion; not tested).

## What Didn't Work

- A run of the base copy of the script is not enough by itself. The script source came from the base, but bun took its settings from the working directory, which was the PR checkout.

## Solution

Run each check step in the base checkout, and give the audit tool the base settings when it reads the head lockfile. PR #44 did this for Bun. PR #55 moved Longhand to npm and Node and kept the same rule.

The age step and the advisory step in `.github/workflows/ci.yml` run in the base checkout:

```yaml
- name: Check the age of added dependency versions
  id: age
  if: github.event_name == 'pull_request'
  working-directory: base
  env:
    SCRIPTS: ${{ steps.base.outputs.scripts }}
  run: |
    script=$SCRIPTS/check-dependency-age.ts
    [[ -f "$script" ]] || script=../scripts/check-dependency-age.ts
    node "$script" . ..
```

npm has no option that stops it from reading the `.npmrc` of its project folder. So `auditCheckout` in `scripts/check-new-advisories.ts` runs `npm audit` in a new temporary folder. The folder holds only copies of the head `package.json` (without `devEngines`) and `package-lock.json`, and a copy of the base `.npmrc`. The command also gives an empty `--userconfig`, `--ignore-scripts`, `--package-lock-only` and `--registry=https://registry.npmjs.org/`.

The first checkout of the job also sets `persist-credentials: false`.

## Why This Works

Node reads no configuration file from its working directory, unless a flag such as `--env-file` names one. npm reads `.npmrc` from the project folder. In the temporary audit folder, the only `.npmrc` is the base copy, so the PR cannot change the registry or another setting of the audit. The PR's lockfiles stay data that the scripts read by path (`..`).

Until PR #55, the same rule applied to Bun: Bun read `bunfig.toml` from the working directory (or from `--config`) and loaded `.env` unless `--no-env-file` was given. The check ran `bun --no-env-file` in the base checkout and `bun audit` with `--config=<base bunfig.toml>`.

## Prevention

- When a CI step runs base code against PR files, make the base checkout the working directory, and pass the base configuration explicitly to each tool that reads configuration from its working directory (npm, cargo). When a tool cannot ignore its project configuration, as npm cannot ignore `.npmrc`, run it in a new folder that holds only the PR data and the base configuration.
- Know the remaining limit. On a `pull_request` event, GitHub runs the workflow file from the PR merge ref. The PR can change `ci.yml` itself, for example the `if:` of a check step, and a `.cargo/config.toml` that the PR adds (the repository has none today) still applies to `cargo deny` in the head checkout. Review of those files is the control, and the comment above the `deny` job says so.
- For a new check of this kind, test the bypass: put a head configuration file that changes the tool (for example a head `.npmrc` with another `registry`), and confirm that the check does not use it.

## Related

- [The default run shell hides a failure in a pipe](../workflow-issues/default-run-shell-hides-pipe-failures.md): another way that a required step passes while its check does nothing.
- [Full licence list blind spots](../workflow-issues/full-licence-list-blind-spots.md): what a dependency gate cannot see.
