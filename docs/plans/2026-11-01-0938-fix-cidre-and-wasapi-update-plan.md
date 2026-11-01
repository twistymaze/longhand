---
title: Update cidre and wasapi - Plan
type: fix
date: 2026-11-01
topic: cidre-and-wasapi-update
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Update cidre and wasapi - Plan

## Goal Capsule

- **Objective:** Users get app audio capture that does not link a wasapi version with RUSTSEC-2026-0332, and macOS capture that has the cidre memory-safety fixes, with no change to how app audio behaves.
- **Means:** One `fix(deps)` PR that updates cidre 0.15.2 to 0.29.0 and wasapi 0.23.0 to 0.26.0 in `crates/audio-system`.
- **Authority:** The 7-day age rule in `docs/development-workflow.md#dependency-age` wins over the version numbers in the upkeep items.
- **Stop conditions:** Stop and ask when a new version is younger than 7 days, when `cargo deny` reports a new advisory or licence, or when a crate update needs a change to Longhand source code.
- **Execution profile:** `ce-work` in a worktree from `origin/main`, one commit per unit, one PR titled as a `fix(deps)` Conventional Commit. Give a signed macOS build for the manual test.
- **Open blockers:** None.

---

## Product Contract

### Summary

Update the two held platform audio crates that `crates/audio-system` uses: cidre (macOS process tap) and wasapi (Windows loopback capture). The PR changes only `crates/audio-system/Cargo.toml`, `Cargo.lock`, and the licence list. The windows-sys and getrandom edge moves in U1 come with it.

### Problem Frame

The upkeep routine holds cidre and wasapi because they sit under audio capture, where a fault is hard to see in CI. wasapi 0.23.0 has RUSTSEC-2026-0332 (`WaveFormat::parse` reads past the end of a `WAVEFORMATEX`), fixed in 0.25.0. Longhand does not call `WaveFormat::parse`, but `AudioClient::get_mixformat` calls it, and `crates/audio-system/src/windows.rs` calls `get_mixformat`. cidre 0.16 to 0.29 fix memory-safety faults without an advisory (alloc/init ownership, release of CF types, FFI output buffers).

App audio (PR 63) ships enabled. On macOS it records apps through the cidre process tap (`src/macos.rs`), lists apps through `src/apps.rs` and checks permission through `src/permission.rs`. On Windows 11 it records apps through wasapi process loopback (`src/windows_app_audio.rs`) and the Teams output device (`src/windows.rs`). The update is therefore on a path that ships to users.

### Requirements

- R1. `Cargo.lock` has wasapi 0.26.0, so RUSTSEC-2026-0332 does not apply.
- R2. `Cargo.lock` has cidre 0.29.0 and cidre-macros 0.12.0.
- R3. `crates/audio-system/Cargo.toml` enables the cidre `dispatch` feature (KTD2).
- R4. Each version that the PR adds to `Cargo.lock` is at least 7 days old, and `min-age-exceptions.toml` gets no new entry.
- R5. `licences/generated/` matches the new dependencies.
- R6. App audio records an app on macOS in a signed build, also after the tap stops and starts again, and the App audio access permission check works.

### Scope Boundaries

- No change to Longhand source code. `PROVENANCE.md` stays the same, because no borrowed code changes.
- Not in this PR: the routine patch updates and sysinfo (next upkeep run).
- No Windows hardware test unless the user asks for one. The Windows CI checks and `scripts/check-windows.sh` compile, lint and test the Windows paths, but they do not run process loopback. The wasapi changes on that path are a block copy in place of a byte loop, a string helper that frees the same memory, and the parse bounds fix (KTD1). The residual risk is a Windows capture fault that only a user would see; the handoff offers the Windows test steps.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **No source change.** The workspace compiles on macOS and for `x86_64-pc-windows-msvc` with the new versions. The changes behind the APIs that `audio-system` calls keep the same behaviour:
  - cidre: `TapDesc`, `ns::Array` and `av::AudioFormat` initializers move from `arc::A<T>` `msg_send` to `objc::init`, which fixes alloc/init ownership; `arc::Release` takes a pointer; `os::Status` gets a checked path in debug builds. The `core_audio` hardware and tap API is unchanged apart from a new property constant.
  - wasapi: the `autoconvert` flags in `initialize_client` are the same; device strings use a helper that frees the same memory; deque reads and writes copy in blocks; `WaveFormat::parse` (used by `get_mixformat`) has the bounds fix; `to_waveformatex` refuses ambiguous formats, which only exclusive-mode format search uses.
- KTD2. **Enable the cidre `dispatch` feature.** cidre 0.29.0's `av` module imports `dispatch` but the `av` feature does not enable it, so the build fails without it. Adding the feature is the smallest fix.
- KTD3. **The manual test is on macOS only.** The macOS process tap and the permission check run in a signed bundle only (TCC). Windows has no local hardware test; CI is the check (Scope Boundaries).

### Risks

- A cidre ownership change could release an object one time too many or too few at run time, which the compile does not show. The manual test (R6) records a note with app audio, pauses the app until the tap stops, plays it again, and starts a second app in the app set. This runs the tap stop, start and process-list update paths in `crates/audio-system/src/macos.rs`, which create and release `TapDesc`, `ns::Array` and the aggregate device again.

---

## Implementation Units

### U1. Update cidre and wasapi

- **Goal:** cidre 0.29.0 and wasapi 0.26.0 in `Cargo.lock`.
- **Requirements:** R1, R2, R3, R4
- **Dependencies:** None
- **Files:** `crates/audio-system/Cargo.toml`, `Cargo.lock`
- **Approach:** Change the requirements to `cidre = "0.29"` with the `dispatch` feature added, and `wasapi = "0.26.0"`. Update only these two crates (and cidre-macros, which cidre requires) with `cargo update -p ... --precise`. Cargo then also moves nine Windows-only dependency edges of other crates (dirs-sys, errno, nu-ansi-term, rustix, rustls-platform-verifier, tempfile, winapi-util and others) from windows-sys 0.52/0.59 to 0.61.2, and tempfile from getrandom 0.3.4 to 0.4.3. Both target versions are already in `Cargo.lock`. A precise update gives the same result, so keep these moves; the Windows CI checks cover them.
- **Test expectation:** none -- dependency update with no behaviour change. The existing `audio-system` tests and the workspace tests are the regression check.
- **Verification:** `Cargo.lock` adds cidre 0.29.0, cidre-macros 0.12.0 and wasapi 0.26.0. Each other change moves an edge to a version that is already in the lock, and no package entry is added or removed. The age check passes.

### U2. Licence list

- **Goal:** `licences/generated/` matches the dependencies.
- **Requirements:** R5
- **Dependencies:** U1
- **Files:** `licences/generated/*`
- **Approach:** Run `scripts/check-licences.sh` and commit the files that it writes.
- **Test expectation:** none -- generated files. The script is the check.
- **Verification:** A second run of `scripts/check-licences.sh` passes with no change.

---

## Verification Contract

| Check | Proves |
|---|---|
| The changed versions in `Cargo.lock` and `crates/audio-system/Cargo.toml` | R1, R2, R3 |
| `node scripts/check-dependency-age.ts` (as in the `cargo deny` CI job) | R4 |
| `cargo deny check` | No new advisory or licence |
| `cargo test --workspace` and `cargo clippy --workspace --all-targets` | KTD1 on macOS |
| `cargo fmt --all --check`, `npm run typecheck`, `npm test` | No other change |
| `scripts/check-windows.sh` and the Windows CI checks | KTD1 on Windows |
| `scripts/check-licences.sh` | R5 |
| Signed macOS build: App audio access check; a note that records an app (for example a browser that plays speech), pauses it until the tap stops, plays it again, and starts a second listed app; the transcript has speech from after each change | R6 |

---

## Definition of Done

- U1 and U2 are committed on one branch, and the PR with a `fix(deps)` title is open and green.
- `min-age-exceptions.toml` has no new entry.
- The user has a signed macOS build and the manual test steps, and the PR is reported merge-ready only after the user reports that the macOS test passed.
