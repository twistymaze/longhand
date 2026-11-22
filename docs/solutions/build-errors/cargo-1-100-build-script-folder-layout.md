---
title: Rust 1.100 moved the build script folders, and the release check found no CMake cache
date: 2026-11-21
category: build-errors
module: Release build (.github/workflows/ci.yml), CMake cache cleanup (.github/actions/sccache-setup/action.yml)
problem_type: build_error
component: tooling
severity: high
symptoms:
  - "Release build failed at \"Check the signatures and the notarization\" after notarization passed"
  - "ls: target/release/build/transcribe-cpp-sys-*/out/build/CMakeCache.txt: No such file or directory"
root_cause: config_error
resolution_type: config_change
tags:
  - cargo
  - build-dir
  - rust-toolchain
  - release
  - cmake
---

# Rust 1.100 moved the build script folders, and the release check found no CMake cache

## Problem

After PR #107 moved the toolchain to Rust 1.100.0, the release build of 1.2.32 failed on `main`. Signing and notarization passed. The last step of "Check the signatures and the notarization" could not find the transcribe-cpp CMake cache, so it could not confirm `GGML_NATIVE=OFF`. Nothing published. The PR checks do not run the release build, so the PR did not show the failure.

## Cause

Cargo in Rust 1.100 uses a new layout for build script output:

| Rust 1.95 | Rust 1.100 |
|---|---|
| `target/<profile>/build/<package>-<hash>/out` | `target/<profile>/build/<package>/<hash>/out` |
| `target/<profile>/.fingerprint/<package>-<hash>` | `target/<profile>/build/<package>/<hash>/fingerprint` |

The release check used the glob `build/transcribe-cpp-sys-*/`, which matches nothing in the new layout.

## Fix

PR #109:

- The release check uses `target/release/build/transcribe-cpp-sys/*/out/build/CMakeCache.txt`.
- The sccache setup step removes only the folder of the build script run (`build/<package>/<hash>`). That folder holds the fingerprint, so cargo runs the build script again. The old `.fingerprint/<package>-<hash>` path does not exist in the new layout.

A merged `ci:` PR publishes the release that the failed run did not publish (see [Releases](../../releases.md)).

## Prevention

- When the Rust toolchain changes, search CI and scripts for paths inside `target/` (`build/`, `.fingerprint/`, `deps/`), and check each one against a local build on the new toolchain.
- A path into `target/` is Cargo's internal layout, not a stable interface. Where possible, get the path from the build (for example, a build script can print it) instead of a glob.
- The release build runs only on `main`. A toolchain PR that changes a release step cannot show it in PR checks, so look at the first release run after the merge.
