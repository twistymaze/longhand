---
title: "macOS accepts a semver pre-release version in both bundle version fields"
date: 2026-09-28
category: integration-issues
module: "Preview builds (src-tauri/tauri.conf.json, scripts/publish-preview.ts)"
problem_type: integration_issue
component: tauri_backend
severity: medium
symptoms:
  - "Tauri writes the config version into CFBundleShortVersionString and CFBundleVersion"
  - "Apple documents both fields as numbers separated by periods, so a version such as 0.1.0-preview.20260929011320 could fail"
root_cause: unverified_assumption
resolution_type: verification
framework_version: "tauri 2 on macOS 26"
related_components:
  - updates
tags: [macos, version, notarization, gatekeeper, tcc, keychain, preview-build, signed-bundle]
---

# macOS accepts a semver pre-release version in both bundle version fields

## Problem
A preview build has the version `<config version>-preview.<UTC commit time>`, for example `0.1.0-preview.20260929011320`. Tauri writes the full version into `CFBundleShortVersionString` and `CFBundleVersion` (`tauri-codegen-2.6.3/src/context.rs:324`). Apple documents both fields as integers separated by periods. The preview track needs a proof that notarization, Gatekeeper, TCC and the Keychain accept this form, and that an update between two such versions keeps the grants.

## Test
Two local release builds (`bun tauri build --bundles app --config <file>`) with the versions `0.1.0-preview.20260929011320` (A) and `0.1.0-preview.20260929012613` (B), signed with the Developer ID certificate and notarized with the App Store Connect API key.

For each build:

- Info.plist holds the full version in both fields.
- Apple's notary service accepts the app, and `xcrun stapler validate` passes.
- `codesign --verify --deep --strict` passes.
- `spctl -a -vv` reports `accepted`, `source=Notarized Developer ID`.

In the installed build A, the user allowed the microphone, recorded a note and made a summary (a Keychain read). A quit with Cmd+Q and a quit from the tray each wrote the `app exit` log line from `RunEvent::Exit`. Build B, installed over A, showed its new version and recorded and summarised with no new microphone or Keychain prompt.

## Result
No change is needed. The version form works in both bundle fields, so the fallback in KTD5 of `docs/plans/2026-09-28-1240-feat-preview-builds-and-updates-plan.md` (a plain `X.Y.Z` config version with the timestamp in `bundle.macOS.bundleVersion`) is not used. TCC grants and Keychain access stay after an update, because the bundle identifier and the Developer ID team stay the same. `RunEvent::Exit` runs for Cmd+Q and for the tray's Quit, so an install at quit can use it. The test did not cover logout; the next-launch install covers that path.

## Prevention
- Keep the bundle identifier `net.ewal.longhand` and the Developer ID team. A change to either resets the TCC grants and the Keychain access of all installed builds.
- If a later macOS rejects the version form, use the KTD5 fallback.
