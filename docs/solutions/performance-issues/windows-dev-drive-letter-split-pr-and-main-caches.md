---
title: "The Dev Drive letter differed between runner types, so pull requests never restored main's Windows caches"
date: 2026-10-29
category: performance-issues
module: "Windows CI caches (.github/actions/windows-build-setup/action.yml, .github/workflows/ci.yml NeMo cache steps)"
problem_type: performance_issue
component: infrastructure
symptoms:
  - "The Windows check of PR #81 took 38 minutes"
  - "rust-cache printed \"No cache found\" on pull requests, although main had a cache with the same prefix"
  - "The Rust sccache hit rate on pull requests was 0.34 % (2 of 593)"
  - "\"Cache not found\" for the NeMo cache, although main had an entry with the same key"
root_cause: config_error
resolution_type: config_change
severity: medium
related_components:
  - ci
tags: [windows-ci, rust-cache, sccache, nemo-cache, dev-drive, github-hosted-runners, larger-runners]
---

# The Dev Drive letter differed between runner types, so pull requests never restored main's Windows caches

## Problem

Pull requests run the Windows checks on `windows-latest`. `main` runs them on the 8-core larger runner (`WINDOWS_RUNNER`). A pull request can restore only caches that `main` saved, but every Windows cache from `main` missed, so each PR built Windows from zero. The cache storage was not full (22.2 of 25 GB).

## Root cause

The two runner types put the build at different absolute paths:

| | Larger runner (`main`) | `windows-latest` (PRs) |
|---|---|---|
| Dev Drive (`setup-dev-drive` takes the next free letter) | `D:` | `E:`, because `D:` is the temp disk |
| `runner.temp` | `C:\a\_temp` | `D:\a\_temp` |

Each Windows cache depends on one of these paths:

- **rust-cache** puts the values of `CARGO_HOME` and `RUSTUP_HOME` into its environment hash, which is part of the key. `main` saved under `…-Windows_NT-x64-9a9abb26-…`; PRs looked for `…-x64-70cf4d4e-…`.
- **sccache** keys of Rust crates include the source paths (`D:\longhand` against `E:\longhand`). C/C++ objects matched only in part.
- **actions/cache** puts the `path` input into the cache version. The NeMo cache had the same key on both, but `C:\a\_temp/longhand-nemo` against `D:\a\_temp/longhand-nemo`.

Both runner types used the same image (`windows-2025-vs2026` 20261021.278.1), so the paths were the only difference.

## Solution

PR #82 mounts the Dev Drive at a fixed folder with the `mount-path` input of `samypr100/setup-dev-drive` (`mount-path: C:/devdrive`). `DEV_DRIVE` is then `C:\devdrive` on every runner type, and the workspace, `CARGO_HOME` and `RUSTUP_HOME` follow it. The NeMo cache moved from `runner.temp` to `${{ env.DEV_DRIVE }}/longhand-nemo`. The rust-cache key changed by itself (new `CARGO_HOME`), so no prefix bump was needed. A folder mount has no drive letter, so the "Disk use" step prints the Dev Drive's free space with `fsutil volume diskfree`.

The first `main` run after the merge saved the caches under the new key (`…-x64-b4b67ae8-…`) and the new NeMo path. The proof that PRs now restore them is the next PR's Windows job: "Restored from" for rust-cache, a NeMo cache hit, and a high Rust sccache hit rate.

## Prevention

When a cache must be shared between jobs on different runner types, give every cached path, and every environment value that a cache key includes, a fixed value. Do not depend on drive letters, `runner.temp` or `github.workspace` on Windows. To find a mismatch, compare the cache keys that `main` saved (`gh api repos/<repo>/actions/caches`) with the restore key in the PR's log, and compare the `env:` values that the two logs print.
