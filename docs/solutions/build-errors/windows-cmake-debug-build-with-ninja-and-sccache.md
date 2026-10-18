---
title: A cold Windows CMake Debug build with Ninja and sccache failed with C1041, then with debug C runtime link errors
date: 2026-10-18
category: build-errors
module: Windows CI compile cache (.github/actions/sccache-setup/action.yml, msvc-toolchain.cmake) and the cmake-rs crates (knf-rs-sys)
problem_type: build_error
component: infrastructure
symptoms:
  - "Workspace Rust tests (Windows) failed in the knf-rs-sys build script: fatal error C1041: cannot open program database '...kaldi-native-fbank-core.pdb'; if multiple CL.EXE write to the same .PDB file, please use /FS"
  - "The failing cl command line already had /FS"
  - "After /Z7 in _CL_, sccache stopped every compile: failed to zip up compiler outputs ... failed to open file ...kaldi-native-fbank-core.pdb"
  - "With /MD /Z7 in _CL_, the compile passed and the test executables failed to link: LNK2001 unresolved external symbol __imp__CrtDbgReport, __imp__malloc_dbg, __imp__free_dbg (msvcprtd.lib)"
  - "main passed on the same runner, because target/ kept a knf-rs-sys library that was built before Ninja"
root_cause: config_error
resolution_type: config_change
severity: high
framework_version: rust 1.95.0, cmake-rs 0.1.58, sccache 0.17.0, MSVC 14.51
tags: [windows, ci, sccache, ninja, cmake, msvc, c-runtime, self-hosted-runner]
related_components: [ci, build]
---

# A cold Windows CMake Debug build with Ninja and sccache failed with C1041, then with debug C runtime link errors

## Problem

The Windows CI job compiles the CMake crates with Ninja and with sccache as the CMake compiler launcher (PR #11). A CMake Debug build uses `/MDd /Zi` by default. Two faults stayed hidden while `target/` on the self-hosted runner kept libraries built before PR #11. A cold build of knf-rs-sys during PR #39 showed both faults (that PR changed the Rust toolchain to 1.95.0, and the runner VM may also have had its weekly reset).

## Symptoms

- C1041 on the target PDB file, with `/FS` on the command line (CMake adds `/FS` itself).
- After the first workaround, `sccache: error: failed to zip up compiler outputs`.
- After the second workaround, LNK2001 for `__imp__CrtDbgReport`, `__imp__malloc_dbg` and `__imp__free_dbg` from the knf-rs-sys objects and `msvcprtd.lib`.

## What Didn't Work

- **`CFLAGS=/FS` and `CXXFLAGS=/FS`.** The log showed `/FS` already on the failing command line. The C1041 comes from parallel compiles through sccache on one `/Zi` PDB file, and `/FS` does not stop it.
- **`_CL_=/Z7`.** cl adds `_CL_` after its command line, so the compile used `/Z7`. But sccache reads only the command line. It saw `/Zi`, expected a PDB output, and failed when no PDB came.
- **`_CL_=/MD /Z7`.** The compile passed in one run, but cargo did not run the knf-rs-sys build script again: `_CL_` is not a variable that cmake-rs or the build script tells cargo to watch, so cargo kept the earlier `/MDd` library. The link failed again, and the rlib hash was the same.
- **`CMAKE_POLICY_DEFAULT_CMP0091` and `CMP0141` in a toolchain file**, with `CMAKE_MSVC_RUNTIME_LIBRARY` and `CMAKE_MSVC_DEBUG_INFORMATION_FORMAT`. CMake reads the policy defaults at `cmake_minimum_required`, before it reads the toolchain file in `project()`. knf-rs-sys requires CMake 3.3 and 3.10, so the old policies stayed and the flags were still `/MDd /Zi`.

## Solution

A toolchain file gives the Debug flags as cache values. CMake reads the toolchain file before its MSVC platform module, and that module keeps a cache value that exists. The settings, abridged from `.github/actions/sccache-setup/msvc-toolchain.cmake:11-26`:

```cmake
set(CMAKE_C_COMPILER cl)
set(CMAKE_CXX_COMPILER cl)
foreach(lang C CXX)
  set(CMAKE_${lang}_FLAGS_DEBUG "/MD /Z7 /Ob0 /Od /RTC1" CACHE STRING "")
  set(CMAKE_${lang}_FLAGS_RELWITHDEBINFO "/MD /Z7 /O2 /Ob1 /DNDEBUG" CACHE STRING "")
endforeach()
set(CMAKE_MSVC_RUNTIME_LIBRARY MultiThreadedDLL)
set(CMAKE_MSVC_DEBUG_INFORMATION_FORMAT "$<$<CONFIG:Debug,RelWithDebInfo>:Embedded>")
```

The action sets `CMAKE_TOOLCHAIN_FILE` on Windows (`action.yml:56`). cmake-rs then passes it as `-DCMAKE_TOOLCHAIN_FILE`, and it no longer gives `-DCMAKE_C_COMPILER`, so the file names `cl`.

cargo does not run a CMake build script again when the toolchain file changes. So the cleanup step keeps a hash of the file in `target/cmake-toolchain.sha256`, a file that the step writes on the runner (`action.yml:79-86`). When the hash changes, it removes each CMake build folder and its fingerprint (`target/<profile>/build/<crate>-<hash>` and `target/<profile>/.fingerprint/<crate>-<hash>`), and cargo runs those build scripts again. The first run after a change builds the CMake crates cold.

## Why This Works

- `/MD` matches the release C runtime that Rust links. With the Visual Studio generator, cmake-rs replaces the Debug flags with its own `/MD` flags. With Ninja it does not, so `/MDd` stayed (the `generator.is_none() && msvc` branch of `src/lib.rs` in the cmake 0.1.58 crate, in the cargo registry).
- `/Z7` keeps the debug information in each object file, so no compiles share a PDB file. sccache sees `/Z7` on the command line and can cache the compile.

## Prevention

- A green Windows job on the self-hosted runner does not prove a cold build works: `target/` keeps earlier build outputs. After a change to the compile setup (generator, launcher, flags), remove the CMake build folders once, or run on a clean `target/`.
- Give compiler options to sccache on the command line, never through `_CL_` or `CL`.
- An option that a CMake project's old policies decide goes in the toolchain file as a cache value, not as a policy default.
- See [the compile cache experiment](../performance-issues/sccache-compile-cache-experiment.md) for the sccache and Ninja setup, and [the NeMo helper doc](nemo-helper-built-for-the-build-machine-cpu.md) for another case where a kept CMake build folder kept old settings.
