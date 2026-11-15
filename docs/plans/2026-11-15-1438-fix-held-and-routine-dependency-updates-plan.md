---
title: Held and routine dependency updates - Plan
type: fix
date: 2026-11-15
topic: held-and-routine-dependency-updates
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Held and routine dependency updates - Plan

## Goal Capsule

- **Objective:** Users get an app whose Windows updater, Keychain, file dialogs, note database and Windows installer use dependency versions with the known security and memory-safety fixes, with no change to how the app behaves.
- **Means:** One `fix(deps)` PR that takes the held Tauri, updater, dialog, Keychain and database sets, three small manifest changes (sysinfo, base64, rfd), and the routine minor and patch updates (KTD1, KTD2).
- **Authority:** The 7-day age rule in `docs/development-workflow.md#dependency-age` wins over the version numbers in this plan. `CLAUDE.md` wins over this plan.
- **Stop conditions:** Stop and ask when an update needs a Longhand source change other than the sysinfo test helper (U2), when `cargo deny` reports a new advisory or licence, or when a migration test fails.
- **Execution profile:** `ce-work` in a worktree from `origin/main`, one commit per unit, one PR titled as a `fix(deps)` Conventional Commit. The maintainer runs the manual tests (macOS and Windows) and approves the merge.
- **Open blockers:** None. The PR opens on or after 2026-11-18 (KTD2).

---

## Product Contract

### Summary

Update the dependency sets that the weekly upkeep held for a manual test, and the routine minor and patch updates, in one PR. Each version is the newest version that is at least 7 days old on the day the PR opens. The PR changes manifests, `Cargo.lock`, `package-lock.json`, the licence list, and one test helper.

### Problem Frame

The upkeep routine takes only fix-only patches by itself. It holds updates under the updater, Keychain, dialog and Tauri crates because a fault there shows only in a signed build, and it holds minor versions. Several of the held updates fix real faults:

- zip 9.1.0, which `tauri-plugin-updater` uses to extract the Windows update archive, has three high advisories (GHSA-66qv-gmq4-qcqj, GHSA-h5xf-qc2v-94w6, GHSA-mpc8-q7jp-wjrf). zip 9.2.0 fixes all three. The archive signature check makes the risk low, but a fault here can stop later updates.
- inout 0.2.5, under the Keychain crates, has GHSA-2943-gp7x-qg39 (aliasing undefined behaviour).
- openssl 0.10.82 fixes two out-of-bounds writes and a use-after-free, with no advisory yet.
- vite 6.4.4 has GHSA-8g5g-mxqh-grgj (dev server only).
- tauri 2.12.5 fixes an NSIS downgrade check, and the Tauri core set has panic and bounds fixes (wry, brotli, crc32fast, flate2, jiff, serde_with, ico).
- rusqlite 0.41 fixes a use-after-free from safe code. Longhand does not reach it, but each later rusqlite step needs a matching rusqlite_migration release, so the pair must move together.

### Requirements

**Security fixes**

- R1. `Cargo.lock` has zip 9.2.0 or later (KTD1), so the three zip advisories do not apply.
- R2. `Cargo.lock` has inout 0.2.6 or later, so GHSA-2943-gp7x-qg39 does not apply.
- R3. `package-lock.json` has vite 6.4.5 or later, so GHSA-8g5g-mxqh-grgj does not apply.
- R4. `Cargo.lock` has openssl 0.10.82 or later and openssl-sys 0.9.118 or later.

**Held sets**

- R5. tauri 2.12.5 and the Tauri core crates move together (tauri-build, tauri-codegen, tauri-macros, tauri-runtime-wry, wry, brotli, jiff, serde_with, ico, crc32fast, flate2, plist), each at the newest version that KTD1 allows.
- R6. tauri-plugin-dialog is 2.8.4 and Longhand's own rfd requirement is 0.17, so `Cargo.lock` has one rfd version, 0.17.2.
- R7. The Keychain crates aes, const-oid and ctutils move with inout, each at the newest version that KTD1 allows.
- R8. xattr is 1.7.0 or later.
- R9. rusqlite is 0.41 and rusqlite_migration is `=2.7.0` in `crates/session-core`. rusqlite 0.42 and 0.43 stay out (KTD3).

**Manifest changes and routine updates**

- R10. The sysinfo dev-dependency of `crates/recorder` is 0.40. Its test helper compiles on macOS and Windows, and its test passes on macOS.
- R11. Longhand's own base64 requirement in `src-tauri/Cargo.toml` is 0.23.
- R12. The routine minor and patch updates move to the newest version that KTD1 allows (KTD4). Crates: mime, notify, notify-types, openssl, openssl-sys, opusic-sys, winnow, js-sys, web-sys, wasm-bindgen and its macro, futures and shared crates, open, rustix, tauri-plugin-single-instance, tinystr, yoke, zerovec, zerovec-derive, async-lock, cc, the icu_* crates, rustls-pki-types, tokio, toml, toml_writer, toml 0.8 with toml_datetime and toml_edit, trash, utf8_iter, uuid, zcheapstr. npm packages: @tailwindcss/vite, prosemirror-schema-list, prosemirror-inputrules, rollup with the @rollup/* binaries, enhanced-resolve, ws, vite, vitest.

**Process**

- R13. `min-age-exceptions.toml` gets no new entry, and the `cargo deny` job passes.
- R16. No lockfile keeps a version that R1 to R3 replace: no zip below 9.2.0, no inout below 0.2.6, and no vite 6.4.4.
- R14. `licences/generated/` matches the new dependencies.
- R15. Existing note databases open and migrate with no data loss, and a new database gets all migrations.

### Scope Boundaries

- No Longhand source change except the sysinfo test helper in `crates/recorder/src/storage.rs` (R10). `PROVENANCE.md` stays the same.
- No new migration and no change to a migration file (`docs/solutions/workflow-issues/edited-applied-migration-breaks-older-databases.md`).
- Not in this PR: rusqlite 0.42 and 0.43 (no rusqlite_migration release accepts them), rubato 5, and the other deferred upkeep items. The transcription engine update, the UI fixes and the WASAPI buffer change are later PRs.
- Versions younger than 7 days stay out, also when the upkeep named them (KTD1). They come in a later weekly run.
- rig-core keeps base64 0.22 in `Cargo.lock`. This PR does not try to remove it.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Newest version that is at least 7 days old on the day the PR opens.** No `min-age-exceptions.toml` entry. On 2026-11-15 this rule gives, for example: rollup 4.68.0 (4.68.1 and later are younger), zip 9.4.2 (9.5.x and 9.6.0 are younger, and 9.4.2 has all three advisory fixes), brotli 9.1.1, vite 6.4.5. On 2026-11-18 more versions pass: wry 0.57.2, const-oid 0.10.3, sysinfo 0.40.3, crc32fast 1.5.5, rustls-pki-types 1.16.0, uuid 1.29.2, rusqlite_migration 2.7.0. Resolve the exact versions again immediately before the push, with `.npmrc`'s `min-release-age=7` for npm and the publish dates on crates.io for crates. (session-settled: user-approved — chosen over an exception for each young version: the upkeep approval said to leave out versions that the age check rejects and take the newest version that passes.)
- KTD2. **Open the PR on or after 2026-11-18.** rusqlite_migration 2.7.0 (published 2026-11-10 22:09 UTC) is the only release that accepts rusqlite 0.41, and it passes the age check after 2026-11-17 22:09 UTC. Implementation runs before that date. The final version resolution and the U5 signed builds run on or after that date, so the manual tests cover the versions that the PR ships. The maintainer decides on the series timing (Assumptions).
- KTD3. **rusqlite 0.41, not 0.43.** rusqlite_migration 2.7.0 accepts only 0.41, and libsqlite3-sys is a `links` crate, so only one rusqlite minor can be in the tree. (session-settled: user-approved — chosen over a wait for 0.43 support: 0.41 now, 0.42 and 0.43 when rusqlite_migration accepts them.) Update the pin comment in `crates/session-core/Cargo.toml` with the new pair.
- KTD4. **Lockfile-only for routine updates.** The routine crates and npm packages move with `cargo update -p <crate> --precise <version>` and `npm update` (npm 12, see `docs/development-workflow.md#checks-and-ci`), not with a manifest change. A manifest requirement changes only for sysinfo, base64, rfd, rusqlite, rusqlite_migration and the tauri minimum (`src-tauri/Cargo.toml`). tauri-plugin-dialog keeps its `2.8` requirement. A precise update can also move other crates' edges to versions that are already in `Cargo.lock`: keep those moves, as the cidre and wasapi plan did.
- KTD5. **No JavaScript Tauri change.** `@tauri-apps/api` and `@tauri-apps/cli` 2.12.4 are the newest releases on npm, and `package-lock.json` already has them. tauri 2.12.5 needs no new JavaScript package.
- KTD6. **Windows NSIS test with a local installer.** CI makes no Windows installer for a PR. Build the NSIS installer of the branch on the Windows test machine with `LONGHAND_RELEASE=1`, as `docs/development-workflow.md#windows` describes for code that is not on `main`. The local build has the version 0.1.0 from `src-tauri/tauri.conf.json`, so its install over the latest release is a downgrade, which the NSIS installer allows by default (`allowDowngrades` is not set). The same version makes Check for Updates offer the latest release, so one build tests both the install and an update.

### Assumptions

- The maintainer accepts that the PR opens on or after 2026-11-18 (KTD2), which delays the next PRs of the series by about three days. The other choice is to leave rusqlite out of this PR and keep it deferred.
- The rusqlite 0.41 and rusqlite_migration 2.7.0 changes that Longhand uses (`M::up`, `M::up_with_hook`, `Migrations::new`, `to_version`, `rusqlite::Error`, `rusqlite_migration::Error` and the too-far-ahead error that the launch check matches) compile and behave the same. The migration tests check this (U3).

### Risks

- A Tauri or wry change can break a window workaround that only a signed build shows. The manual test retests Install and Relaunch for one Dock tile (`docs/solutions/integration-issues/update-relaunch-opened-a-second-dock-tile.md`) and the window frame and traffic lights (`docs/solutions/ui-bugs/window-showed-before-tao-applied-saved-frame.md`, `docs/solutions/ui-bugs/tao-traffic-light-y-not-top-inset.md`).
- rfd 0.17 can change the startup alert in `src-tauri/src/startup.rs`, which calls `MessageDialog` directly. A compile on macOS and Windows checks the API; the manual test checks one file dialog on each OS.
- zip: Longhand's Windows update artifact is the signed `-setup.exe`, so the update flow does not reach the zip code. The version in `Cargo.lock` and `cargo deny` prove R1 and R16; U5 does not.
- uuid v5 IDs in `crates/shared-settings/src/prompts.rs` must stay the same. The existing tests of the built-in prompt IDs check this.

---

## Implementation Units

### U1. Held sets and routine updates in the lockfiles

- **Goal:** `Cargo.lock` and `package-lock.json` have the versions of R1 to R8 and R12.
- **Requirements:** R1, R2, R3, R4, R5, R6, R7, R8, R11, R12, R13, R16
- **Dependencies:** None
- **Files:** `src-tauri/Cargo.toml`, `Cargo.lock`, `package-lock.json`
- **Approach:**
  1. Change `src-tauri/Cargo.toml`: tauri `2.12.5`, rfd `0.17` (and its comment), base64 `0.23`.
  2. Update the held crates and the routine crates with precise updates, at the versions that KTD1 gives (KTD4).
  3. Update the npm packages with npm 12, so `min-release-age=7` picks the versions.
  4. Check that `Cargo.lock` has one rfd version and that no new crate entry comes in except the planned versions.
- **Patterns to follow:** `docs/plans/2026-11-01-0938-fix-cidre-and-wasapi-update-plan.md` (U1), `docs/plans/2026-10-31-2230-fix-held-dependency-sets-plan.md`.
- **Test expectation:** none -- dependency update with no behaviour change. The workspace tests, the Windows CI checks and the manual tests (U5) are the regression check.
- **Verification:** The age check and `cargo deny check` pass. `cargo tree -d -i rfd` shows one rfd. `Cargo.lock` and `package-lock.json` have no version that R16 excludes. `cargo tree -e features -i reqwest` shows the same TLS backend as before (`docs/solutions/integration-issues/reqwest-tls-backend-with-unified-features.md`).

### U2. sysinfo 0.40

- **Goal:** The sysinfo dev-dependency is 0.40, and the disk test helper uses the 0.40 API.
- **Requirements:** R10
- **Dependencies:** None
- **Files:** `crates/recorder/Cargo.toml`, `crates/recorder/src/storage.rs`, `Cargo.lock`
- **Approach:** Change the requirement to `0.40`. 0.40 removes `Disks::list()`: change the helper that calls `Disks::new_with_refreshed_list().list()` to iterate the disks through the 0.40 API. `crates/recorder/tests/soak.rs` uses only the process API.
- **Test scenarios:**
  - The existing storage tests in `crates/recorder` that use the helper pass on macOS.
  - The Windows CI checks and `scripts/check-windows.sh` compile the helper for Windows.
- **Verification:** `cargo test -p recorder` passes.

### U3. rusqlite 0.41 with rusqlite_migration 2.7.0

- **Goal:** The note database uses rusqlite 0.41, and all migrations work on old and new databases.
- **Requirements:** R9, R15
- **Dependencies:** None
- **Files:** `crates/session-core/Cargo.toml`, `Cargo.lock`
- **Approach:** Change rusqlite to `0.41` and rusqlite_migration to `=2.7.0` (KTD3). Read the rusqlite 0.41 and rusqlite_migration 2.7.0 changelogs for changes to the APIs in Assumptions. No migration file changes.
- **Test scenarios:**
  - The old-database tests in `crates/session-core/tests/migrations.rs` pass: each builds a database at an older version from the real migration SQL, then migrates it to the latest version.
  - `migrations_one_to_twenty_one_keep_their_hashes` and the hash tests pass, so the recorded migration hashes do not change.
  - A new database gets all 25 migrations (`a_new_database_records_each_migration_with_the_build_hash`).
  - The too-far-ahead launch check still matches its error (`docs/solutions/runtime-errors/older-build-panicked-on-newer-database-at-launch.md`).
- **Verification:** `cargo test -p session-core` passes. The manual test (U5) opens the maintainer's real library, which a release build created.

### U4. Licence list

- **Goal:** `licences/generated/` matches the dependencies.
- **Requirements:** R14
- **Dependencies:** U1, U2, U3
- **Files:** `licences/generated/*`, possibly `licences/manifest.json`, `deny.toml`, `about.toml`
- **Approach:** Run `scripts/check-licences.sh` and commit the files that it writes. A new permissive licence goes in `deny.toml` and `about.toml` (`docs/solutions/workflow-issues/new-permissive-licence-goes-in-the-allow-list.md`).
- **Test expectation:** none -- generated files. The script is the check.
- **Verification:** A second run of the script passes with no change.

### U5. Signed builds and manual tests

- **Goal:** The maintainer confirms the paths that CI cannot test.
- **Requirements:** R5, R6, R7, R15
- **Dependencies:** U1, U2, U3, U4, and the final KTD1 resolution on or after 2026-11-18 (KTD2)
- **Files:** none
- **Approach:**
  1. Make both builds from the commit that the PR pushes. If the final resolution changes a version after a test, make the builds again and repeat the tests.
  2. macOS: a signed build with `LONGHAND_RELEASE=1` (`docs/solutions/workflow-issues/manual-test-of-the-updater-needs-a-release-flag-build.md`), notarized when the test runs on another Mac.
  3. Windows: the NSIS installer of the branch, built with `LONGHAND_RELEASE=1` on the Windows test machine (KTD6).
- **Test scenarios:**
  - macOS: the app opens the existing library and its notes. Keychain: an LLM API key that the release saved still reads, and a new key saves. A file dialog opens (folder choice, file import). Check for Updates offers the latest release, and Install and Relaunch gives one Dock tile and the release.
  - Windows: the installer installs over the latest release, the app starts with the existing library, a file dialog opens, and Check for Updates installs the latest release.
- **Verification:** The maintainer reports that each scenario passes.

---

## Verification Contract

| Check | Proves |
|---|---|
| The changed versions in the manifests and lockfiles | R1 to R12, R16 |
| `node scripts/check-dependency-age.ts` and `cargo deny check` | R13, no new advisory or licence |
| `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo fmt --all --check` | KTD4 on macOS, U2, U3 |
| `npm run typecheck`, `npm test`, `npm run build` | The npm updates (vite, vitest, rollup, tailwind) |
| `scripts/check-windows.sh` and the Windows CI checks | U1 and U2 on Windows |
| `scripts/check-licences.sh` | R14 |
| U5 manual tests on macOS and Windows, on the commit that the PR pushes | R5, R6, R7, R15 in a signed build |

---

## Definition of Done

- U1 to U4 are committed on one branch, and the PR has a `fix(deps)` title. It opens on or after 2026-11-18 with the versions that KTD1 gives on that day, and CI is green.
- `min-age-exceptions.toml` has no new entry.
- The maintainer reports that the U5 tests passed on builds of the pushed commit, and approves the merge.
- No test code or helper from an abandoned attempt stays in the diff.
