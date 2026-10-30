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

The first `main` run after the merge saved the caches under the new key (`…-x64-b4b67ae8-…`) and the new NeMo path.

## Second cause: the restore wrote main's caches to the workspace drive

After PR #82, the PR #83 Windows job printed "Restored from cache key … full match: true" and "Cache restored from key: nemo-Windows-…". But it still took 33 minutes; `main` took 12 minutes for the same code. Cargo compiled 558 units against 48 on `main`, and CMake configured and built NeMo again. With `CARGO_LOG=cargo::core::compiler::fingerprint=info`, cargo printed `failed to read …\.fingerprint\serde-d7b465a333563a5c\lib-serde … cannot find the file` for each dependency, although `main`'s cache holds that folder.

`actions/cache` stores a path outside the workspace as a path relative to the workspace (`../../../devdrive/…`), and it extracts with `tar -C <workspace>`. The logs show `-C C:/a/longhand/longhand` on the larger runner and `-C D:/a/longhand/longhand` on `windows-latest`. So on a PR, the Rust cache and the NeMo cache went to `D:\devdrive`, and the build on `C:\devdrive` found nothing. A fixed mount path is not sufficient: the path relative to the workspace must also point to it.

A path on another drive than the workspace is stored as a full path. So PR #84 replaces the `C:\devdrive` folder mount with a fixed drive letter: the action mounts the Dev Drive at the next free letter, and the next step changes that letter to `Z:` (`Set-Partition -NewDriveLetter Z`) and sets `DEV_DRIVE`, `DEV_DRIVE_WORKSPACE`, `CARGO_HOME` and `RUSTUP_HOME` to `Z:`. `Z:` is never the workspace drive, so each runner type saves and restores full `Z:\` paths. The action's cleanup dismounts the VHDX by its file path, so the changed letter does not affect it. (A first version of PR #84 made `D:\devdrive` a junction to `C:\devdrive`. A test run proved that it worked, but the fixed letter is simpler.)

The same PR also puts the remaining CMake inputs on the Dev Drive (the toolchain file, `VULKAN_SDK`, SPIRV-Headers), because CMake records their paths. Moving them made the cached CMake folders name a Vulkan SDK that was gone, and the CMake generate step failed ("Imported target Vulkan::Vulkan includes non-existent path"). So `build-nemo.sh` and the sccache setup now remove a cached CMake build folder that names a missing `Vulkan_*` path or toolchain file. The Windows rust-cache prefixes changed (`v5-rust`, `v4-release-windows`), so that `main` saves caches with the new paths. With the junction version, the PR #84 Windows job restored `main`'s fingerprints, compiled 54 units, and had a Rust sccache hit rate of 100 %.

## Prevention

When a cache must be shared between jobs on different runner types, give every cached path, and every environment value that a cache key includes, a fixed value. Do not depend on drive letters, `runner.temp` or `github.workspace` on Windows. To find a mismatch, compare the cache keys that `main` saved (`gh api repos/<repo>/actions/caches`) with the restore key in the PR's log, and compare the `env:` values that the two logs print.

A key hit does not prove that the build used the cache. Also compare the number of `Compiling` lines and the sccache hit rate with a warm `main` run, and compare the `tar … -C <dir>` folders of the restores. For a cached path outside the workspace, keep it on a drive that is never the workspace drive, such as a fixed `Z:`, so that `actions/cache` stores a full path.
