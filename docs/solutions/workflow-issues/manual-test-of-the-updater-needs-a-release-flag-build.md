---
title: A manual test of the updater needs a test build with LONGHAND_RELEASE=1
date: 2026-10-15
category: workflow-issues
module: Updater manual tests (src-tauri/src/commands/build_info.rs, src-tauri/src/managers/updates.rs, docs/development-workflow.md manual tests)
problem_type: workflow_issue
component: development_workflow
related_components:
  - tauri_backend
  - testing_framework
severity: medium
applies_when:
  - "A change touches the update notice, the tray update item, Check for Updates, or Install and Relaunch, and needs a manual test"
  - "You make the signed or notarized test build from docs/development-workflow.md#manual-tests for an updater change"
tags:
  - updater
  - manual-test
  - release-build
  - notarization
  - test-build
---

# A manual test of the updater needs a test build with LONGHAND_RELEASE=1

## Context

The manual test steps in `docs/development-workflow.md#manual-tests` make a test build with `bun tauri build --bundles app`. That build is not a release build, so it gets no updates. For the update notice timing change (PR twistymaze/longhand#21), the first signed build and the first notarized build of this session were both of this kind. In those builds, "Check for Updates…" shows "This build gets no updates", and no update notice or tray update item can show. So these builds cannot test an updater change.

## Guidance

For a manual test of an updater change, set `LONGHAND_RELEASE=1` at build time. Add the notarization variables when the test runs on another Mac:

```sh
LONGHAND_RELEASE=1 \
APPLE_API_KEY=<key id> APPLE_API_ISSUER=<issuer id> APPLE_API_KEY_PATH=<path to .p8> \
bun tauri build --bundles app
```

- `is_release_build()` reads `option_env!("LONGHAND_RELEASE")` at compile time, and only the value `1` gives a release build (`src-tauri/src/commands/build_info.rs:18-19`). Because the value is read at compile time, cargo builds the `longhand` crate again when the value changes. A build that you made without the flag does not change at run time.
- The version in `src-tauri/tauri.conf.json` is `0.1.0`. CI sets the real version only for a release (`.github/workflows/ci.yml:486-487`, the `release.json` overlay). The updater check offers a release whose version is higher than the version of the app, so the test build sees the latest published release as an update, and the full flow runs: the notice, the tray item, Install and Relaunch.
- The updater manifest is `latest.json` of the latest release (`src-tauri/src/managers/updates.rs:47-50`). The test needs network access and a published release.
- The local build needs no `TAURI_SIGNING_PRIVATE_KEY`. Only the CI `release.json` overlay sets `createUpdaterArtifacts`, and only an updater archive needs that key.

## Why This Matters

Without the flag, the test build hides the full update flow. A test of that build can pass for the wrong reason (no notice shows, so "no notice over another app" looks correct). It can also stop the test, and the build must be made again. In this session, two builds (one signed, one notarized) were made before the gap was found.

## When to Apply

- Before you make the test build for a change to `src-tauri/src/managers/updates.rs`, `src-tauri/src/managers/updates/flow.rs`, the update window, or the tray update item.
- Not for other manual tests: a test build without the flag makes no update checks, so the installed release and its update state stay out of the test.

## Examples

Effects to know before the test:

- Install and Relaunch downloads the published release and installs it over the running app, which is the test build (per this session's test steps). After the relaunch, the app is the published release, not the change under test. Test the steps before the install first.
- The test build uses the same app data as the installed release (`docs/development-workflow.md#manual-tests`), so update delays (Remind Me Later, Later) that the test sets also apply to the installed release.
- Quick check of the bundle: `spctl -a -vv Longhand.app` must say `source=Notarized Developer ID` for a remote Mac. "Check for Updates…" in the test build must show the offer for the latest release, not "This build gets no updates".

## Related

- `docs/solutions/ui-bugs/window-showed-before-tao-applied-saved-frame.md`: how the update window shows, and when it takes the keyboard.
- `docs/solutions/integration-issues/semver-prerelease-version-in-macos-bundle.md`: the bundle version that CI sets.
