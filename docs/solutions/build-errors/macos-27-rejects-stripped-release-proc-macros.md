---
title: macOS 27 rejects stripped release proc macros, and rustc reports E0463
date: 2026-10-19
module: Release build of the app (Cargo release profile, bun tauri build)
problem_type: build_error
component: infrastructure
severity: high
symptoms:
  - "`bun tauri build` fails with E0463 \"can't find crate for `ctor_proc_macro`\", then for phf_macros, tokio_macros and other proc macros."
  - "The dylib of the proc macro is in target/release/deps, and rustc gives no other note."
  - "Debug builds and release builds without MACOSX_DEPLOYMENT_TARGET work."
root_cause: config_error
resolution_type: config_change
framework_version: rustc 1.95.0, Xcode 27.1, macOS 27.0.1
tags:
  - macos
  - xcode
  - proc-macro
  - strip
  - deployment-target
  - e0463
  - release-profile
retire_when: "A rustc release loads a stripped release proc macro that was linked with MACOSX_DEPLOYMENT_TARGET=15.0 on macOS 27. Test it with a minimal proc-macro crate before you remove the override."
---

# macOS 27 rejects stripped release proc macros, and rustc reports E0463

## Problem

After the upgrade of the Mac Studio to macOS 27.0.1 and Xcode 27.1, the signed release build (`bun tauri build --bundles app`) failed. rustc could not load any proc macro that it had built in the same run.

## Symptoms

- `error[E0463]: can't find crate for 'ctor_proc_macro'` in the `ctor` crate. After a fix of that one crate, the same error came for `phf_macros`, `futures_macro` and `tokio_macros`.
- The proc-macro dylib was present in `target/release/deps`, and `--extern` named its correct path.
- rustc gave only E0463. `RUSTC_LOG=rustc_metadata::locator=debug` showed that rustc read the file, but gave no reason for the rejection.

## What Didn't Work

- **`cargo clean -p ctor-proc-macro -p ctor`, then a rebuild.** The full build made the same broken dylib again. A rebuild of only `ctor` with `cargo build --release -p ctor` worked, because that command does not set `MACOSX_DEPLOYMENT_TARGET`. This difference pointed to the build environment.
- **A comparison of the `.rustc` metadata sections.** The start of the metadata was the same in a good dylib and a bad dylib. The fault is not in the metadata.

## Solution

Do not strip build scripts and proc macros in the release profile (PR #52, `Cargo.toml:43`):

```toml
[profile.release]
strip = true

[profile.release.build-override]
strip = false
```

For a one-time local build without the change, the environment form is `CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=false`.

Proc macros that a failed run built stay broken in `target/release/deps`. The changed profile gives them a new hash, so Cargo builds them again.

## Why This Works

Tauri sets `MACOSX_DEPLOYMENT_TARGET` from `minimumSystemVersion` (`src-tauri/tauri.conf.json:36`, now `15.0`). rustc applies the variable to host artifacts too, and that includes proc macros. The release profile sets `strip = true` (`Cargo.toml:34`). In this session, a minimal proc-macro crate gave these results on macOS 27 with Xcode 27.1 and rustc 1.95.0:

| `MACOSX_DEPLOYMENT_TARGET` | Release, `strip = true` |
|---|---|
| not set (rustc default 11.0) | works |
| 11.0 | works |
| 12.0, 13.0, 14.0, 15.0, 26.0, 27.0 | E0463 |

A debug build with 15.0 worked. With `[profile.release.build-override] strip = false`, the release build with 15.0 worked.

The `LC_SYMTAB` string table offset (`stroff`) of the bad dylib was 327564, which is not a multiple of 8 (327564 % 8 = 4). The good dylib had 327896. rustc loads a proc macro with `dlopen`. Other projects report that the macOS 27 loader refuses a dylib with this misaligned string table, and that rustc then reports only E0463 (for example privacykey/privacytracker#382 and manaflow-ai/cmux#17311). This session did not test the loader directly.

`build-override` applies only to build scripts, proc macros and their dependencies. These run only on the build machine. The app binary is still stripped, so the change does not change the shipped app.

## Prevention

- When a release build on macOS fails with E0463 for a proc macro that is present in `target/release/deps`, first test the same crate without `MACOSX_DEPLOYMENT_TARGET` or in a debug build. If those work, suspect the strip of host artifacts, not the dependency.
- To test a new macOS or Xcode release, use a minimal proc-macro crate with `strip = true` and `MACOSX_DEPLOYMENT_TARGET=15.0`. It shows the fault in seconds; a full app build takes many minutes.
- Do not remove the `build-override` section until the check in `retire_when` passes.

## Related: the Xcode 26.3 linker and the macOS 27 SDK

Before the Xcode upgrade, the same build failed earlier, in the CMake configure step of the NeMo helper (sentencepiece). The macOS 27 upgrade installed Command Line Tools whose `MacOSX.sdk` link points to `MacOSX27.0.sdk`. The Xcode 26.3 linker cannot read its `libSystem.tbd`:

```text
ld: tapi error: malformed file
.../MacOSX27.0.sdk/usr/lib/libSystem.B.tbd:4:20: error: unknown architecture
                   arm64e.x1-macos, arm64e.x1-maccatalyst ]
```

CMake then reports that `/usr/bin/cc` cannot compile a simple test program. The fix is to upgrade Xcode (27.1 worked) and open it once. Then delete `target/nemo/sentencepiece-build`, because its CMake cache keeps the result of the failed configure step.

The self-hosted macOS CI runner is a tart VM with its own macOS and Xcode. The upgrade of the host did not change it, and CI passed on it after the upgrade.
