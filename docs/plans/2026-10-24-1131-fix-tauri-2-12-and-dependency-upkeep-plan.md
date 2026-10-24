---
title: Tauri 2.12 and dependency upkeep 2026-10-24 - Plan
type: fix
date: 2026-10-24
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Tauri 2.12 and dependency upkeep 2026-10-24 - Plan

## Goal Capsule

- **Objective:** Longhand users get a release that has the security fixes of Tauri 2.12, its updater, objc2 and the Keychain crypto crates. Updates, the Keychain, windows, menus, tray and recording work as before.
- **Means:** One `fix(deps)` PR that updates the approved dependencies together (KTD1).
- **Authority:** This plan, then `CLAUDE.md` and `docs/development-workflow.md`.
- **Stop conditions:** Stop and report when an update needs a code change that alters behaviour that users see, or when a version fails the `cargo deny` checks and an exception has no security reason.
- **Execution profile:** Mostly manifest and lock file changes. Proof is the full local checks, CI on macOS and Windows, and a manual test of a signed macOS build.
- **Who finishes:** The implementing session opens the PR, takes it to green, and gives the user the signed build for the manual test.

## Product Contract

### Summary

Update Tauri to 2.12 with its CLI, API package, runtime, updater and all plugins. In the same PR, take the security patches (objc2, keyring crypto crates, happy-dom) and the approved routine updates. Regenerate the licence list.

### Problem Frame

Tauri core, its CLI and the updater were held while 2.12 matured. Six held sets now carry security fixes: tauri (channel data exposure, a muda use-after-free on macOS), the CLI (dev-server and NSIS fixes), tauri-codegen (brotli memory faults), tauri-runtime-wry (tao/wry undefined behaviour), tauri-utils (Permissions-Policy header name) and the updater (signature checks). Other fixes are also held: objc2 0.6.5 fixes a miscompiled retain, ctutils fixes RUSTSEC-2026-0385, and happy-dom 21 fixes GHSA-834v-8h66-3h79 in the test environment. Each merged PR makes a release, and these updates all change `Cargo.lock`, `package-lock.json` and `licences/generated/`. Thus one PR is better than many.

### Requirements

**Tauri 2.12**

- R1. `tauri` 2.12 with `tauri-build`, `tauri-codegen`, `tauri-runtime-wry` and `tauri-utils` at the latest versions that pass the age check (KTD4); `@tauri-apps/cli` 2.12.4 or later (zip 9 fix); `@tauri-apps/api` 2.12.
- R2. Every `tauri-plugin-*` at the version that needs tauri 2.12: dialog, fs, log, opener, single-instance, store, window-state, updater 2.14. The exact pins move: autostart `=2.7.0`, global-shortcut `=2.4.1`, notification `=2.7.0`.
- R3. `windows` 0.62 in `src-tauri`, so `Cargo.lock` keeps one `windows` copy for the app.

**Security patches**

- R4. objc2 0.6.5; ctutils 0.4.5, inout 0.2.5 and sha2 0.11.1 under keyring; `@happy-dom/global-registrator` ^21.

**Routine updates**

- R5. The approved routine updates: proptest 1.12, sysinfo 0.39, notify-debouncer-full 0.7, notify-rust 4.18.2 with tauri-winrt-notification 0.8, and these minor and patch versions within their ranges:
  - Crates: deranged, lazy_static, libc, num_threads, schannel, schemars, schemars_derive, serde_json, syn, zeroize, toml_edit, zerocopy, zerocopy-derive, zbus_names, zvariant, zvariant_derive, hyper, hyper-util, want, js-sys, web-sys, the wasm-bindgen crates, rustls-native-certs, tokio, tokio-util, async-recursion, either, uuid, generic-array, cc, and the icu_* crates.
  - npm: `@types/node`, enhanced-resolve, ws, rollup with its `@rollup/rollup-*` platform packages.
  - A version that the age check refuses is left at an older version, not given an exception (KTD4).

**Release safety**

- R6. The new updater installs the latest release, and an installed copy of the previous release updates to this PR's release. The Keychain reads and writes API keys. Windows, menus and tray behave as before. Recording a note works.

### Scope Boundaries

- ort rc.15, ndarray 0.17, transcribe-cpp 0.4.1, cidre 0.29 and wasapi 0.26 are not in this PR. They follow after this PR merges.
- rubato 5, sha2 0.11 for the app's own hashing, rusqlite 0.41 and the Handy catalog schema 2 stay deferred.
- No Windows installer test from this branch: CI builds and tests Windows, and a Mac cannot build the installer.

## Planning Contract

### Key Technical Decisions

- KTD1. **One PR for all approved updates.** Each one rewrites the same lock files and licence list, and each merge makes a release. (session-settled: user-approved — chosen over one PR per group: fewer releases and no lock file conflicts between PRs.)
- KTD2. **Keep `specta` `=2.0.0-rc.22` and `tauri-specta` `=2.0.0-rc.21`.** tauri-specta rc.21 needs `tauri ^2.1.1`, so it works with 2.12. rc.26 needs specta rc.26, which changes the generated bindings API and the pins of both reference clones. That move is not an upkeep patch.
- KTD3. **notify-debouncer-full 0.7 departs from anarlog.** The `src-tauri/Cargo.toml` comment says the version follows anarlog's notify plugin. Change the comment to say that 0.7 is newer than anarlog's version for its deduplication fix. 0.6 drops Modify events right after a Create. The library watcher reacts to both kinds with the same rescan, so it sees no change in behaviour.
- KTD4. **Young versions only with a security reason.** The age check (`scripts/check-dependency-age.ts`, run in the `cargo deny` CI job) fails a version in `Cargo.lock` or `package-lock.json` that is less than 7 days old. Only an exact entry in `min-age-exceptions.toml` lets one through. Install a young npm version with the steps in `docs/development-workflow.md#dependency-age`. The security versions that need entries on 2026-10-24:
  - `@tauri-apps/cli` 2.12.4 (2026-10-21) and each of its 11 `@tauri-apps/cli-*` platform packages at 2.12.4: RUSTSEC-2026-0367/0368/0369 (zip 9).
  - `happy-dom` and `@happy-dom/global-registrator` 21.0.4 (2026-10-20): GHSA-834v-8h66-3h79 affects up to 21.0.3.
  - ctutils 0.4.5 (2026-10-23): RUSTSEC-2026-0385.
  - Any other young version only when a security fix needs it. For a routine version, such as tauri 2.12.5 and its 2026-10-21 build crates (bundler fixes only), use the older version.
- KTD5. **Two update tests.** Before the merge, a signed build with `LONGHAND_RELEASE=1` runs the updater. Its version is 0.1.0, so it offers the latest release and runs the full download, signature check and install with updater 2.14 (`docs/solutions/workflow-issues/manual-test-of-the-updater-needs-a-release-flag-build.md`). That test cannot show that installed releases accept the new updater artifacts, because only a release makes them. So before the merge, also read the CLI, bundler and updater changes for a change to `latest.json`, the archive or the signature format that the installed updater 2.12 refuses. Stop and report if one exists. After the merge, the release in `/Applications` updates to the new release through Check for Updates.

### Sources

- Checks to do for each update: the usage of `AsyncFd::new` (tokio 1.53.3 deprecates it), schemars property order, and the `Debug` output of zeroize's `Zeroizing`. A search of `crates/` and `src-tauri/` finds none of the three in Longhand code. If a test fails, compare `testdata/storage-folder/`: a JSON key order change from a feature is a known cause (`docs/solutions/integration-issues/dependency-feature-changed-json-key-order.md`).
- `src-tauri/src/managers/library.rs` uses `new_debouncer`, `Debouncer<RecommendedWatcher, RecommendedCache>` and `DebouncedEvent::new` in its tests.

## Implementation Units

### U1. Tauri 2.12 stack

- **Goal:** R1, R2, R3.
- **Dependencies:** none.
- **Files:** `src-tauri/Cargo.toml`, `package.json`, `Cargo.lock`, `package-lock.json`, `min-age-exceptions.toml`, the Rust and TypeScript source that a breaking API change touches (muda menus, `WindowEffect`, menu commands that now return errors, opener `Error`).
- **Approach:** Raise the manifest floors to the 2.12 versions, move the three exact pins, and move `windows` to 0.62 (R2, R3). Keep the specta pins (KTD2). Add the age exceptions (KTD4). Fix compile errors with the smallest change that keeps behaviour. Regenerate the TypeScript bindings only if the build changes them.
- **Test scenarios:** Test expectation: none -- the existing workspace and frontend tests cover the IPC and window code; U4 covers the behaviour.
- **Verification:** The workspace builds and its tests pass on macOS. `scripts/check-windows.sh` passes. `cargo tree -d` shows no second `windows` 0.61 copy from `src-tauri`.

### U2. Security patches

- **Goal:** R4.
- **Dependencies:** U1.
- **Files:** `Cargo.lock`, `package.json`, `package-lock.json`.
- **Approach:** Use `cargo update -p` for objc2, ctutils, inout and sha2@0.11. Move `@happy-dom/global-registrator` to ^21.
- **Test scenarios:** Test expectation: none -- `crates/secrets` tests and `npm test` prove the updates.
- **Verification:** `cargo deny check advisories` shows no RUSTSEC-2026-0385, and `npm audit` shows no GHSA-834v-8h66-3h79.

### U3. Routine updates

- **Goal:** R5.
- **Dependencies:** U2.
- **Files:** `crates/session-core/Cargo.toml` (proptest), `crates/recorder/Cargo.toml` (sysinfo), `src-tauri/Cargo.toml` (notify-debouncer-full and its comment, KTD3), `src-tauri/src/managers/library.rs` if the debouncer API changed, `Cargo.lock`, `package-lock.json`.
- **Approach:** Update the routine crates and npm packages within their ranges. Then raise the three manifest versions: proptest, sysinfo and notify-debouncer-full. notify-rust and tauri-winrt-notification come with the notification plugin.
- **Test scenarios:**
  - The library watcher tests in `src-tauri/src/managers/library.rs` still pass: a rescan event and a file change event each give one library refresh.
  - The session-core proptest suite runs with proptest 1.12.
- **Verification:** The full local checks pass.

### U4. Licences and the signed manual test

- **Goal:** R6, and a licence list that matches the dependencies.
- **Dependencies:** U1, U2, U3.
- **Files:** `licences/generated/*`.
- **Approach:** Run `scripts/check-licences.sh` and commit what it writes. Do the changelog check of KTD5. Make a signed macOS build with `LONGHAND_RELEASE=1` (KTD5), notarized when the user tests on another Mac.
- **Test scenarios,** in this order, because the install replaces the build under test:
  - Settings saves an API key to the Keychain and reads it back after a restart.
  - The main window, Settings, the app menu, the tray menu and the recording widget open and work. The Settings frame is the same as before.
  - A short recording makes a note with a transcript and a summary.
  - Check for Updates offers the latest release. Install and Relaunch downloads it, verifies its signature, and relaunches into the release. About shows the release version.
  - After the merge: the previous release in `/Applications` updates to the new release.
- **Verification:** The user reports that each scenario before the merge passed. The post-merge update is reported after the release.

## Verification Contract

| Gate | Command or check |
|---|---|
| Rust | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` |
| Frontend | `npm run typecheck`, `npm test`, `npm run test:scripts` |
| Windows lint | `scripts/check-windows.sh` |
| Licences | `scripts/check-licences.sh` writes no change after the commit |
| Dependency rules | `cargo deny check`; `scripts/check-dependency-age.ts` against `origin/main` passes with only KTD4 exceptions |
| CI | All PR checks green on macOS and Windows, including CodeQL |
| Manual | U4 scenarios pass on the signed macOS build |

## Definition of Done

- R1 to R6 are true, and every gate in the Verification Contract passes.
- The PR title is `fix(deps): …` and names Tauri 2.12 and the security fixes.
- Each `min-age-exceptions.toml` entry has a reason.
- The diff has no unused code from attempts that the work did not keep.
