---
title: sccache and the release profile in CI, measured on hosted and self-hosted runners
date: 2026-10-12
last_updated: 2026-10-25
category: performance-issues
module: CI workflow (.github/workflows/ci.yml, release-windows.yml, .github/actions/sccache-setup)
problem_type: performance_issue
component: infrastructure
severity: medium
symptoms:
  - Release builds took 24 min (macOS) and 35 min (Windows) on GitHub's runners
  - The last step of each release build (full LTO, one codegen unit) took 8 to 11 min with no log output
  - The build log showed "Compiling trash" for minutes; trash was only the last crate that cargo started
root_cause: config_error
resolution_type: config_change
framework_version: rust 1.94.0, sccache 0.17.0
related_components:
  - tooling
tags:
  - sccache
  - rust-cache
  - lto
  - codegen-units
  - github-actions
  - self-hosted-runner
  - ci-compile-time
---

# sccache and the release profile in CI, measured on hosted and self-hosted runners

## Problem

CI took too long. The plan `docs/plans/2026-10-11-1635-ci-sccache-compile-cache-plan.md` measured sccache on the two hosted release builds and on the self-hosted checks. The first warm runs showed that the release profile's last step took more time than any cache could save, so the release profile was measured too.

## Decision (2026-10-12, PR twistymaze/longhand#11)

- sccache on every CI build of the app: the checks (hosted and self-hosted) and both release builds. rust-cache stays.
- The release profile uses no LTO and 16 codegen units.
- Rust, C and C++ compiles go through sccache. On Windows, CMake uses the Ninja generator.

The user wants the same setup on every runner, and accepts a few percent of Rust runtime speed for faster builds.

## Measurements

One run per cell, on GitHub's runners unless the row says otherwise. "Warm" means the next commit after a Cargo.lock change (hyper-util 0.1.20 to 0.1.21) and one workspace source change, built on the cache that the cold run saved. Before the experiment, the release builds took 23.7 min (macOS, run 38155311321) and 34.8 min (Windows, run 38178105848, nearly cold).

**sccache setup, today's release profile (full LTO, one codegen unit), job time:**

| | rust-cache + sccache, cold | rust-cache + sccache, warm | sccache only, cold | sccache only, warm |
|---|---|---|---|---|
| macOS | 23.2 min | 9.9 min | 20.5 min | 14.8 min |
| Windows | 32.2 min | 21.0 min | 39.1 min | 26.2 min |

- Cold runs gained nothing and lost nothing within normal variation.
- "sccache only" (rust-cache keeps only the crate downloads) was slower when warm. The restored target/ folder does more than sccache alone.
- Warm runs had 92 to 95% cache hits. One Windows cold run had 11 write errors in 1,467 writes (GitHub's limit of 200 cache uploads a minute); the others had 0.
- Each setup used about 3.2 GB of the 10 GB repository cache.

**Release profile, with rust-cache + sccache:**

| | Full LTO, 1 unit | Thin LTO, 16 units | No LTO, 16 units |
|---|---|---|---|
| Last step, warm, macOS / Windows | 6.0 / 11.1 min | 5.1 / 5.9 min | 2.4 / 2.6 min |
| Job, warm, macOS / Windows | 9.9 / 21.0 min | 8.5 / 14.6 min | 7.2 / 10.4 min |
| Job, cold, macOS / Windows | 23.2 / 32.2 min | 19.6 / 35.0 min | 15.7 / 25.6 min |
| DMG / Windows installer | 25.3 / 32.6 MB | 26.9 / 32.9 MB | 26.7 / 32.6 MB |

The last step is the time from the last `Compiling` line to `Finished`: the app crate and the LTO link. Runtime speed was not measured.

**Self-hosted checks, a switch between two Cargo.lock states:**

- macOS: 5.6 min with the local sccache cache, 6.4 to 6.7 min without. Cache hits 97 to 99%. The folder held about 700 MB (20 GB limit).
- Windows: no measurable difference (4.8 min with, 4.2 to 6.4 min without). The switch rebuilds only 6 crates there.
- No run with sccache was slower than the matching run without it.

**Hosted macOS checks (added 2026-10-20, PR twistymaze/longhand#57):**

| Runner | Warm job | Rust tests | Licences |
|---|---|---|---|
| Self-hosted Mac Studio (`longhand-macos`) | about 6 min | 4.4 min | 0.2 min |
| `macos-latest-xlarge` (M2 Pro, 5 cores) | 11.1 to 13.4 min | 5.0 to 5.4 min | 1.0 min (0.6 min after the fix below) |
| `macos-latest` (M1, 3 cores) | 22.1 min | 10.5 min | 2.5 min |

- The CodeQL Rust analysis took about 13 min. It is the longest check of a PR, so a macOS checks job under 13 min does not make a PR finish later.
- The first `macos-latest` run was cold (40.5 min, 0.5% Rust cache hits). The self-hosted runner had kept its sccache store in a local folder, so GitHub's cache had no macOS entries for the PR to read. A PR reads only its own cache entries and those of `main`.
- `macos-latest` and `macos-latest-xlarge` share the rust-cache key (`v4-rust-checks-Darwin-arm64-…`) and the sccache entries. The first xlarge run used the cache of the `macos-latest` runs (98.9% hits). A change between the two runners stays warm.
- One xlarge job waited 10 min in the queue for a machine. Another started after 5 s.
- The licence step was slow because cargo-about reads the licence files of each crate, once for each target. PR #57 runs cargo-about only when a licence input changes (`scripts/licence-inputs-changed.sh`), runs the two targets at the same time, and does not install and build the frontend again in CI.

## What to know

- **The log is misleading during the last step.** Cargo prints a line when a crate starts. With full LTO, the app crate and the link took minutes with no output, so the last crate that started (trash) seemed slow.
- **A changed RUSTC_WRAPPER rebuilt everything once on the Windows runner** (12 to 14 min). macOS did not rebuild.
- **A VM reset empties the local cache.** Two resets during the experiment emptied the Windows cache, and its first "on" runs had 0 hits. Check the cache folder size in the job summary before you trust a self-hosted measurement.
- **CMake refuses a build folder that another generator made.** The self-hosted Windows runner kept Visual Studio build folders in target/. The sccache setup action removes each CMake build folder that is not a Ninja build. Since PR #71 it also removes a cached CMake build whose compiler or source folder is missing (another runner image or Dev Drive letter).
- **The cmake-rs crate does not pass RUSTC_WRAPPER to CMake.** The C/C++ compiles of ggml need `CMAKE_C_COMPILER_LAUNCHER` and `CMAKE_CXX_COMPILER_LAUNCHER`, and the Visual Studio generator ignores both.
- **Use sccache v0.17.0.** v0.18.0 can panic at startup with "no CA certificates loaded" (mozilla/sccache#2869).
- **Not measured:** rust-cache alone after a Cargo.lock change, so the share of the warm gain that comes from sccache is not known. The hosted Windows checks were measured on 2026-10-25, when PR #71 moved them to GitHub's runners: see [rust-cache dropped the CMake toolchain hash](rust-cache-dropped-cmake-toolchain-hash-wiped-warm-windows-builds.md).
- **A dependency update changes the licence lists.** The macOS licence check fails until `scripts/check-licences.sh` writes them again.

## Method notes for a later experiment

- `gh workflow run` starts only a workflow that the default branch has, so the experiment jobs were inputs of `ci.yml` on the branch.
- A driver script moved the branch between two pushed commits with `gh api -X PATCH repos/<repo>/git/refs/heads/<branch> -F force=true`, so that runs continued without local git steps. GitHub scopes cache entries by branch, so a warm run must run on the same branch as its cold run.
- `gh cache delete --all --ref <ref>` deletes thousands of small sccache entries one by one (about 20 min for 4,600).
- Job summaries are not in the API. The runs page shows them.
