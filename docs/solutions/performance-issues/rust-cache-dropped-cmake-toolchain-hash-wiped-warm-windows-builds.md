---
title: rust-cache dropped the CMake toolchain hash, so each warm hosted Windows run rebuilt the CMake crates
date: 2026-10-25
category: performance-issues
module: Windows CI compile cache (.github/actions/sccache-setup/action.yml, .github/actions/windows-build-setup/action.yml, scripts/build-nemo.sh)
problem_type: performance_issue
component: infrastructure
symptoms:
  - A warm 16-core hosted Windows check took 20.1 minutes; the cold run took 17.1 minutes
  - knf-rs-sys, opusic-sys and transcribe-cpp-sys showed "Compiling" on every warm run, with no change to Cargo.lock
  - The knf-rs-sys build script took 2 to 6.5 minutes on a warm run
  - "The step log printed \"Removing target/debug/build/knf-rs-sys-<hash>\" on every hosted run"
root_cause: config_error
resolution_type: config_change
severity: medium
framework_version: rust 1.95.0, Swatinem/rust-cache v2 (pin 6323deb), sccache 0.17.0
related_components:
  - tooling
tags:
  - rust-cache
  - windows-ci
  - cmake
  - cargo-fingerprint
  - github-hosted-runners
  - nemo-cache
  - ci-compile-time
---

# rust-cache dropped the CMake toolchain hash, so each warm hosted Windows run rebuilt the CMake crates

## Problem

On GitHub's hosted Windows runners, a warm run of the Windows check was slower than a cold run. A step of the sccache setup action deleted the CMake builds that rust-cache had restored, on every run. PR #71 fixed it, when the Windows checks moved from the self-hosted runner to hosted runners.

## Symptoms

- A warm run on the 16-core larger runner took 20.1 minutes. The cold run took 17.1 minutes.
- The warm test step compiled only 34 crates, but took 8 minutes. Most of the time was the `knf-rs-sys` build script: 6.5 minutes on one run, 1.6 minutes on the next. The same build script took 2.2 minutes on the cold run.
- `knf-rs-sys`, `opusic-sys` and `transcribe-cpp-sys` (the crates that build C or C++ with cmake-rs) showed `Compiling` on every warm run, with no change to `Cargo.lock`. Their dependents (`asr`, `embedding`, `diarize`, `pipeline`) waited for them.
- The step "Remove CMake build folders of another generator" printed `Removing target/debug/build/<crate>-<hash>` for each of these crates on every hosted run.

## What Didn't Work

- **sccache write errors.** The cold run had 562 failed cache writes of 937. The warm run had none, and 95% hits. The failures were a burst on the cold run, not the cause.
- **A slow link or build script as the cause.** The 5-minute gap after `Compiling asr` looked like a link or a slow build script. The gap was the `knf-rs-sys` build script, but its time changed from run to run (1.6 to 6.5 minutes). The real question was why it ran at all.
- **Registry source times.** rust-cache does not keep `~/.cargo/registry/src`, so cargo extracts the crate sources again in each job. New file times would make a `rerun-if-changed` build script run again. But cargo extracts each file with the fixed time from the `.crate` archive (2006 for these crates), so this was not the cause.

## Solution

**Diagnosis.** One run with `CARGO_LOG=cargo::core::compiler::fingerprint=info` on the test step showed why each unit was dirty. For each CMake crate, the run-build-script fingerprint was missing:

```text
fingerprint error for knf-rs-sys v0.3.2/RunCustomBuild/...
err: failed to read `D:\longhand\target\debug\.fingerprint\knf-rs-sys-<hash>\run-build-script-build-script-build`
```

The fingerprint folder had the same `<hash>` as the build folder in the `Removing` lines.

**Cause.** The step compared a hash of the CMake toolchain file with `target/cmake-toolchain.sha256`. When the hashes did not agree, it deleted each CMake build folder and its fingerprint, so that cargo ran the build scripts again with the new toolchain file. The self-hosted runner kept `target/` between jobs, so the file was there. rust-cache restores only `target/<profile>/build`, `deps` and `.fingerprint`, without loose files in `target/`. On a hosted runner the hash file was never there, and the step deleted the CMake builds on every run.

**Fix (PR #71).**

- The rust-cache key includes the hash of the toolchain file (`.github/actions/windows-build-setup/action.yml:81`): `key: cmake-${{ hashFiles('.github/actions/sccache-setup/msvc-toolchain.cmake') }}`. A new toolchain file gives a new cache, so the hosted runner needs no hash file.
- The step compares the hash only on a self-hosted runner (`.github/actions/sccache-setup/action.yml:90`).
- Reusing a cached CMake build on another machine has its own risks, so the step (now "Remove CMake build folders that cannot be used") removes a build folder and its fingerprint when the `CMakeCache.txt` names a missing compiler or a missing source folder (`action.yml:107-113`). An update of the runner image can replace the MSVC version, and an older linker cannot always link libraries from a newer compiler. The Dev Drive action does not fix the drive letter, so a cache from another job can name a source folder on another letter, and CMake refuses that build folder.
- `scripts/build-nemo.sh:19-28` has the same check for the NeMo build folders. It configures a build folder again when its compiler is missing or its `CMAKE_HOME_DIRECTORY` is not the current source folder. CI now restores these folders from the Actions cache on every hosted Windows run. A push to `main` saves them when the cache has no entry for the key yet.

The same PR made other warm-run changes. The table shows the Windows job times on 2026-10-25, before the NeMo cache:

| Runner | Warm Windows job |
|---|---|
| 16-core larger runner | 10.4 and 15.6 minutes |
| 8-core larger runner | 12.7 and 14.7 minutes |
| `windows-latest` (4 vCPU) | 24.9 and 24.4 minutes |

- SPIRV-Headers install from the commit of tag `vulkan-sdk-1.4.309.0` with CMake, not with vcpkg (about 40 s to about 20 s).
- On hosted runners, `CARGO_PROFILE_DEV_DEBUG=line-tables-only` makes the links of the test binaries and the app faster.
- The Windows tests run with cargo-nextest, which runs the test binaries side by side.

## Why This Works

The cache key now holds the only input that the hash file held. A change of the toolchain file gives a new key and a cold run, which is the result that the old step forced with the hash file. With the same key, the restored CMake builds and their fingerprints stay, so cargo marks the CMake crates fresh. A CMake build that came from another image or another drive letter can fail where cargo does not see a change. The guard removes it together with its fingerprint, so cargo runs that build script again on this machine.

`ort-sys` still runs its build script again on each warm run, because rust-cache removes `DirectML.dll` from `target/debug`. That costs about 12 s, and the workspace crates compile again on each run in any case, so the change left it.

## Prevention

- Do not keep state that a CI step depends on in `target/` (or in another folder that a cache restores) unless the cache keeps that exact file. Put the input into the cache key.
- To prove a warm run is warm, count the `Compiling` lines of non-workspace crates in the log. If a dependency compiles with no change to `Cargo.lock`, run once with `CARGO_LOG=cargo::core::compiler::fingerprint=info` and read the `dirty:` and `fingerprint error` lines. They name the missing file or the changed dependency.
- A cached CMake build folder can come from another machine. Before reuse, check that the compiler and the source folder in its `CMakeCache.txt` exist (`CMAKE_CXX_COMPILER`, `CMAKE_HOME_DIRECTORY`). When it is a cargo build script's folder, remove the fingerprint too, or cargo keeps the old libraries.
- Compare runners on warm time from two or more runs. Two runs on one runner differed by up to 5 minutes in this measurement.

## Related Issues

- [windows-cmake-debug-build-with-ninja-and-sccache.md](../build-errors/windows-cmake-debug-build-with-ninja-and-sccache.md): why the toolchain file and the CMake folder cleanup exist.
- [sccache-compile-cache-experiment.md](sccache-compile-cache-experiment.md): sccache and rust-cache on hosted and self-hosted runners.
- [cargo-feature-unification-ci-rebuilds.md](cargo-feature-unification-ci-rebuilds.md): earlier Windows CI compile time work, rust-cache keys and the Dev Drive.
- [nemo-helper-built-for-the-build-machine-cpu.md](../build-errors/nemo-helper-built-for-the-build-machine-cpu.md): a NeMo build folder that CI kept between jobs.
