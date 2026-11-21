---
title: "A Windows cache from an older runner image called a linker that the new image does not have"
date: 2026-11-21
category: build-errors
module: "Windows CI caches (.github/actions/windows-build-setup/action.yml, .github/actions/sccache-setup/action.yml, .github/workflows/ci.yml NeMo cache steps, scripts/build-nemo.sh)"
problem_type: build_error
component: infrastructure
symptoms:
  - "\"Build and stage NeMo helper\" failed on a PR that changed only dependency versions"
  - "CMake: the C compiler MSVC 19.52 \"is not able to compile a simple test program\""
  - "LINK Pass 1 called ...\\MSVC\\14.51.36231\\bin\\Hostx64\\x64\\link.exe and failed with \"no such file or directory\""
  - "A PR run on the same image one day before passed, because it had a cache miss"
root_cause: config_error
resolution_type: config_change
severity: high
related_components:
  - ci
tags: [windows-ci, nemo-cache, rust-cache, cmake, msvc, runner-image, github-hosted-runners]
---

# A Windows cache from an older runner image called a linker that the new image does not have

## Problem

On 2026-11-21 the Windows check of PR #104 (dependency updates only) failed in "Build and stage NeMo helper" (Actions run 44922525635). The job ran on image win25-vs2026/20261116.315, which has MSVC 14.52.37025. It restored the NeMo cache that a push to `main` saved on 2026-11-20 (run 44666459894) on image 20261103.298, which had MSVC 14.51.36231. The cached SentencePiece build folder named the 14.51 linker, so CMake's compiler test called a `link.exe` that the new image does not have.

## What Didn't Work

- **A re-run.** The routine re-ran the failed job, and it failed again in the same way. The cache stays the same until a key input changes.
- **The missing-path guard alone.** PR #103 added `CMAKE_LINKER`, `CMAKE_AR` and `CMAKE_MT` to the guard in `scripts/build-nemo.sh`, which configures a build folder again when a tool path is missing. PR #104's branch did not have it, so a rebase fixes this one failure. But the guard accepts a folder whose old tools are still installed next to new ones, and an older linker cannot always link the objects of a newer compiler. Also, the key still lets each later image restore the old cache.

## Solution

- `windows-build-setup` reads the default MSVC toolset version from the Visual Studio install (`VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt`) before rust-cache runs, and exports it as `LONGHAND_MSVC_TOOLSET`. `msvc-dev-cmd` sets `VCToolsVersion` only later.
- The rust-cache key and the NeMo cache key contain `LONGHAND_MSVC_TOOLSET`. An exact hit then comes only from a job with the same toolset.
- The guards in `scripts/build-nemo.sh` and `.github/actions/sccache-setup/action.yml` also configure a build folder again when a tool path in its `CMakeCache.txt` is under `VC/Tools/MSVC/<version>/` and `<version>` is not `$VCToolsVersion`. This covers what the key cannot: a rust-cache fallback restore, a build folder that stays between jobs, and a job whose actual toolset differs from the default version that the key read. `scripts/cmake-cache-guards.test.ts` tests both copies.

## Why This Works

A cached CMake build folder holds absolute paths to the tools of one toolset. The toolset version is an input of the cache, so it belongs in the key. A new toolset gives a new key and one cold build on `main`. An image update that keeps the MSVC version keeps the warm cache. The guard is the second check, for the restores that the key does not control.

## Prevention

- Put each input that a cached build depends on in the cache key, also an input that the runner image supplies (a compiler or linker version), not only the repository files.
- When a cache-related failure shows on a PR, check whether the run had a cache hit, which run saved that cache, and which runner image both runs used (the "Runner Image" group at the top of each job log).
- Related: [rust-cache dropped the CMake toolchain hash](../performance-issues/rust-cache-dropped-cmake-toolchain-hash-wiped-warm-windows-builds.md), [the Dev Drive letter split PR and main caches](../performance-issues/windows-dev-drive-letter-split-pr-and-main-caches.md).
