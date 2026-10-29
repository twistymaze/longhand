---
title: "After an in-app update on macOS, Longhand opened in a second Dock tile, because tauri's restart runs open -n while the old process still runs"
date: 2026-10-29
category: integration-issues
module: "Update restart (src-tauri/src/relaunch.rs, src-tauri/src/managers/updates.rs)"
problem_type: integration_issue
component: background_job
severity: medium
symptoms:
  - "On macOS, after Install and Relaunch, the new build opened in a second, temporary Dock tile"
  - "The tile that the user keeps in the Dock showed no running app"
  - "After a quit and a new launch, the kept tile was active again"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "tauri 2.12.4 on macOS"
tags:
  - tauri
  - restart
  - updater
  - dock
  - keep-in-dock
  - open-n
  - launchservices
retire_when: "tauri's macOS restart (tauri::process::restart / restart_macos_app) waits for the old process to end before it opens the bundle, or no longer uses open -n. Read the tauri source of the version in Cargo.lock to check."
---

# Update relaunch opened a second Dock tile

## Problem

On macOS, Longhand restarts after an in-app update: at "Install and Relaunch", and when it installs a saved build at launch. After the restart, the new build showed in a second Dock tile. The tile that the user keeps in the Dock stayed empty. A quit and a new launch made the kept tile active again.

## Root cause

tauri 2.12.4 restarts a macOS app with `open -n <bundle>` (`restart_macos_app` in `tauri::process`). It runs `open -n` while the old process still runs. LaunchServices then starts a second instance, and the Dock gives that instance its own tile. The kept tile stays with the old process, which ends a moment later.

`app.request_restart()` does not prevent this: the restart occurs after `RunEvent::Exit` and `cleanup_before_exit`, but the process still runs at that time.

anarlog uses the same tauri restart API, but on tauri 2.11.5, which starts the binary directly and not with `open -n`. Thus a comparison with anarlog does not show the problem.

## Solution

[PR #78](https://github.com/twistymaze/longhand/pull/78) added `src-tauri/src/relaunch.rs`. On macOS, the two update restarts start a small helper and then quit. The helper starts the bundle only after the old process ends:

1. Longhand starts `/bin/sh -c HELPER_SCRIPT` in its own process group (`process_group(0)`), with a stdin pipe. Longhand keeps the write end of the pipe and does not close it (`std::mem::forget`). The kernel closes it when the process ends, and Rust pipes are close-on-exec, so no child keeps it open.
2. The helper does `read _`, which returns at EOF, when the old process ends.
3. A `kill -0 <pid>` loop, at most 5 s, waits until the old PID is gone.
4. The helper runs `/usr/bin/open -n <bundle> --args …`. If `open` fails, it does `exec` of the binary, as tauri's own restart does.

The callers:

- `relaunch::request_relaunch` (the `Effect::Restart` arm in `updates.rs`) calls `app.exit(0)` after the helper starts. Thus `RunEvent::Exit` still runs `install_at_exit`. If the helper does not start, it calls `app.request_restart()`.
- `relaunch::relaunch_now` (the install at launch) calls `cleanup_before_exit()` and `std::process::exit(0)`. If the helper does not start, it calls `app.restart()`.

On Windows, both functions use tauri's restart. The Windows installer starts the new build itself.

## Verification

- A notarized `LONGHAND_RELEASE=1` build updated itself to the published release with Install and Relaunch. The Dock showed one tile, and the kept tile was active. This test ran before the `open` fallback was added.
- Unit tests in `relaunch.rs` run the real helper script with a recording opener. They check the arguments, each wait by itself, and the fallback to the binary. A helper that does not wait is found only when it runs before the next step of the test, because no event can show that a launch did not occur too early.

## Prevention

Before you change the restart path or update tauri, read tauri's `restart_macos_app` in the version in `Cargo.lock`. If it still runs `open -n` before the process ends, keep `relaunch.rs`. A Dock test needs a signed, notarized update from a published release (see `docs/solutions/workflow-issues/manual-test-of-the-updater-needs-a-release-flag-build.md`). `tauri dev` cannot do an in-app update, so it cannot show the problem.
