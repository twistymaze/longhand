---
title: Update the Held Dependency Sets - Plan
type: fix
date: 2026-10-31
topic: held-dependency-sets
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Update the Held Dependency Sets - Plan

## Goal Capsule

- **Objective:** Users get an app that does not link a zip version with the two known zip advisories, and whose Keychain and window crates have the current fixes, with no change to how Longhand behaves.
- **Means:** One `fix(deps)` PR that updates three held crate sets in `Cargo.lock` and the `dirs` requirement in `src-tauri/Cargo.toml`, with only versions that pass the 7-day age rule (KTD1).
- **Authority:** The 7-day age rule in `docs/development-workflow.md#dependency-age` wins over the version numbers in the upkeep items.
- **Stop conditions:** Stop and ask when a set cannot update without a version younger than 7 days, when `cargo deny` reports a new advisory or licence, or when a crate update needs a code change outside `src-tauri/Cargo.toml`.
- **Execution profile:** `ce-work` in a worktree from `origin/main`, one commit per unit, one PR titled as a `fix(deps)` Conventional Commit. Give a signed macOS build for the manual test.
- **Open blockers:** None.

---

## Product Contract

### Summary

Update the tauri-plugin-updater set (zip, typed-path), the keyring set (inout), the tauri-runtime-wry set (tao, wry, tauri-runtime-wry, siphasher) and `dirs` 6 to 7. Versions younger than 7 days on 2026-11-01 (UTC) stay at their current version until a later upkeep run.

### Problem Frame

The upkeep routine holds these crates because they sit under the updater, the Keychain and the window runtime, where a fault is hard to see in CI. zip 9.0.5 under tauri-plugin-updater has GHSA-wjr6-mhvj-cjvm and GHSA-gmfj-vffv-32pw, both fixed in zip 9.1.0. inout 0.2.5 fixes a soundness fault. tao and wry fix drag-and-drop and focus faults on macOS. wry already uses dirs 7, and Longhand still requires dirs 6. auto-launch 0.6.0 (the newest version, under tauri-plugin-autostart) also requires dirs 6, so dirs 6 stays in `Cargo.lock` for it.

### Requirements

**Security fixes**

- R1. `Cargo.lock` has zip 9.1.0 or later, so neither zip advisory applies.
- R2. `Cargo.lock` has inout 0.2.5.

**Runtime and paths**

- R3. `Cargo.lock` has tao 0.37.2, wry 0.57.1, tauri-runtime-wry 2.12.3 and siphasher 1.0.4.
- R4. `src-tauri/Cargo.toml` requires `dirs = "7"`, and the `longhand` entry in `Cargo.lock` depends on dirs 7.0.0. Only auto-launch keeps dirs 6.0.0.

**Rules**

- R5. Each version that the PR adds to `Cargo.lock` is at least 7 days old, and `min-age-exceptions.toml` gets no new entry. The tauri-plugin-updater 2.14.1 entry stays until 2026-11-04.
- R6. `licences/generated/` matches the new dependencies.
- R7. The update install, Keychain store/read/delete, window drag-and-drop and window focus work in a signed macOS build.

### Scope Boundaries

- No change to Longhand code other than the `dirs` requirement.
- Not in this PR: cidre 0.29 and wasapi 0.26 (a separate PR after this one merges), the routine patch updates and sysinfo (too young this week).
- No Windows installer test. The Windows CI checks compile and test the Windows paths. Longhand calls only `dirs::home_dir` and `dirs::data_local_dir`, which dirs 7 does not change.

#### Deferred to Follow-Up Work

- zip 9.1.3 or 9.2.0, xattr 1.7.0 and ctutils 0.4.7: a later upkeep run takes them when they are 7 days old (KTD1).

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Take the newest version that is 7 days old, with no new age exception.** The age rule allows an exception only when the fix needs the younger version. zip 9.1.0 (2026-10-24) has both advisory fixes, so zip 9.1.3 (2026-10-27) and 9.2.0 (2026-10-31) do not qualify. xattr 1.7.0 (2026-10-28) fixes a build with rustix 1.1.6, and `Cargo.lock` has rustix 1.1.5. ctutils 0.4.6 and 0.4.7 harden only Thumb-1 and RISCV32, which Longhand does not build. These three stay at their current versions. If inout 0.2.5 or zip 9.1.0 requires a younger crate, stop (Goal Capsule).
- KTD2. **Use `cargo update --precise` for each crate.** A precise update changes only the named crate and what it requires, so other crates in `Cargo.lock` do not move.
- KTD3. **The manual test uses a build with `LONGHAND_RELEASE=1`.** Only a release build checks for updates (`docs/solutions/workflow-issues/manual-test-of-the-updater-needs-a-release-flag-build.md`). The build then offers the latest published release. The update install is a regression check of the updater and of the window runtime only. zip and typed-path are Windows-only dependencies of tauri-plugin-updater, macOS updates are tar.gz, and Longhand's Windows updates are NSIS `.exe` files, which the updater installs without zip extraction.

### Risks

- tauri-runtime-wry 2.12.3 may require a newer tauri. If it does, stop and ask, because that is a larger change.
- typed-path 0.13 changes absolute-path rules for UNC paths on Windows. Only the updater's zip extraction uses it, and Longhand's Windows updates do not use zip extraction, so the Windows compile (`scripts/check-windows.sh` and the Windows CI checks) is the check.

---

## Implementation Units

### U1. Updater set

- **Goal:** zip 9.1.0 and typed-path 0.13.0 in `Cargo.lock`.
- **Requirements:** R1, R5
- **Dependencies:** None
- **Files:** `Cargo.lock`
- **Approach:** Precise update of zip to 9.1.0 (KTD1, KTD2). zip 9.1.0 requires typed-path ^0.13, so typed-path 0.13.0 comes with it. Keep xattr at 1.6.1.
- **Test expectation:** none -- lockfile only. The `Cargo.lock` versions and the age check prove R1 and R5.
- **Verification:** `Cargo.lock` has zip 9.1.0 and typed-path 0.13.0, and xattr stays at 1.6.1.

### U2. Keyring set

- **Goal:** inout 0.2.5 in `Cargo.lock`.
- **Requirements:** R2, R5
- **Dependencies:** None
- **Files:** `Cargo.lock`
- **Approach:** Precise update of inout to 0.2.5. Keep ctutils at 0.4.5 (KTD1).
- **Test expectation:** none -- lockfile only. The Keychain round trip is a manual test (R7).
- **Verification:** `Cargo.lock` has inout 0.2.5 and ctutils 0.4.5.

### U3. Window runtime set

- **Goal:** tao 0.37.2, wry 0.57.1, tauri-runtime-wry 2.12.3 and siphasher 1.0.4 in `Cargo.lock`.
- **Requirements:** R3, R5
- **Dependencies:** None
- **Files:** `Cargo.lock`
- **Approach:** Precise updates of the four crates. tauri stays at its current version.
- **Test expectation:** none -- lockfile only. Drag-and-drop and focus are manual tests (R7).
- **Verification:** The workspace builds and its tests pass.

### U4. dirs 7

- **Goal:** Longhand uses dirs 7, the same version as wry.
- **Requirements:** R4
- **Dependencies:** None
- **Files:** `src-tauri/Cargo.toml`, `Cargo.lock`
- **Approach:** Change the requirement to `"7"`. The comment above it stays true.
- **Patterns to follow:** The existing tests in `src-tauri/src/startup.rs`.
- **Test scenarios:**
  - The existing `startup.rs` tests pass, so the startup alert log path on macOS stays `~/Library/Logs/<id>`.
- **Verification:** The `longhand` entry in `Cargo.lock` lists dirs 7.0.0, and only `auto-launch` lists dirs 6.0.0.

### U5. Licence list

- **Goal:** `licences/generated/` matches the dependencies.
- **Requirements:** R6
- **Dependencies:** U1, U2, U3, U4
- **Files:** `licences/generated/*`
- **Approach:** Run `scripts/check-licences.sh` and commit the files that it writes.
- **Test expectation:** none -- generated files. The script is the check.
- **Verification:** A second run of `scripts/check-licences.sh` passes with no change.

---

## Verification Contract

| Check | Proves |
|---|---|
| The changed versions in `Cargo.lock` and `src-tauri/Cargo.toml` | R1, R2, R3, R4 |
| `node scripts/check-dependency-age.ts` (as in the `cargo deny` CI job) | R5 |
| `cargo deny check` | No new advisory or licence |
| `cargo test --workspace` and `cargo clippy --workspace --all-targets` | R3, R4 |
| `cargo fmt --all --check`, `npm run typecheck`, `npm test` | No other change |
| `scripts/check-windows.sh` | Windows compile of the changed crates |
| `scripts/check-licences.sh` | R6 |
| Signed macOS build with `LONGHAND_RELEASE=1` (KTD3): update install, Keychain store/read/delete, drag-and-drop, focus | R7 |

---

## Definition of Done

- U1 to U5 are committed on one branch, and the PR with a `fix(deps)` title is open and green.
- `min-age-exceptions.toml` has no new entry.
- The user has a signed macOS build and the manual test steps.
