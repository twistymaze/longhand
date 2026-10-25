---
title: Capture restart budget counts consecutive failures - Plan
type: fix
date: 2026-10-24
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Capture restart budget counts consecutive failures - Plan

## Goal Capsule

- **Objective:** When the microphone fails during a note and does not come back, Longhand interrupts the note and tells the user, also when each reopen attempt is slow.
- **Means:** Port anarlog's consecutive restart counter, and reset it only after capture was healthy again (KTD1, KTD2).
- **Authority:** This plan, then `CLAUDE.md` and `docs/development-workflow.md`.
- **Stop conditions:** Stop and report when the fix needs a change to the restart backoff, the number of attempts, or the interrupt and alert path.
- **Execution profile:** Small Rust change in `crates/supervisor` and `src-tauri/src/managers/capture.rs`. Proof is unit tests on the fake clock. Write the spaced-failure test first and see it fail.
- **Who finishes:** The implementing session opens one `fix(capture): …` PR and takes it to green and CLEAN.

## Product Contract

### Summary

Count capture restarts as consecutive failures, with no time window. Reset the count only when capture recorded for 30 s after it last came back. A microphone that keeps failing then always runs out the three attempts and interrupts the note.

### Problem Frame

`RestartTracker::record_restart` counts restarts in a 22 s window, and `recover` also calls `maybe_reset_at`, which clears the count 30 s after the window start. Both clear the count while failures continue. Each reopen can take up to the 5 s start limit of `audio_cpal`. Then the attempts end at about 6.5 s, 14 s and 23.5 s, the third failure is outside the window, the count starts again at 1, and the loop never runs out. The note stays Capturing with no audio, and the user gets no alert. anarlog fixed the same fault in commit `693500220d8e253b175e7a5b88fd831c48b4583b`.

### Requirements

- R1. Consecutive failed restarts run out the source budget after three attempts, however long each attempt takes.
- R2. A microphone that comes back and fails again before it was healthy spends the same budget (AE15, the flapping microphone in `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`, stays true).
- R3. After capture recorded for 30 s since it last came back, the next loss gets a full budget again.
- R4. The backoff (1, 2, 4 s), the number of attempts, and the interrupt and alert at the end do not change. A wake and a returned device start a new budget, as now.

### Scope Boundaries

- No frame-level health signal like anarlog's `SourceHealthy` message. A stalled input does not end a segment in Longhand, so it starts no restart and never interrupts the note; that is a separate, existing gap. A reopened stream that stalls and reports a loss more than 30 s later gets a new budget. That case is no worse than a plain stall, and a fix needs a stall detector, not this budget.
- The recorder budget, `retry.rs` and the window-based methods of `restart.rs` stay. The methods are upstream code; the provenance record keeps them.
- No manual test. The fault needs reopen attempts that each take about 5 s, which a person cannot make on purpose. The fake-clock tests prove R1 to R3, and the existing tests prove R4.

## Planning Contract

### Key Technical Decisions

- KTD1. **Port `record_consecutive_restart` and `reset` from anarlog 693500220d unchanged,** with their test. `restart.rs` keeps its other methods and tests.
- KTD2. **Healthy means 30 s of recording since capture last came back.** anarlog resets after 30 s of continuous frames. In Longhand a segment ends on its own only on a loss, so 30 s from a successful reopen to the next loss is the same signal. The period is `SOURCE_RESTART_BUDGET.reset_after` (30 s), so no new constant.
- KTD3. **The supervisor crate owns the reset, so the fake clock tests R1 to R3 end to end.** A Longhand-owned `SourceRestarts` in `recover.rs` holds the ported `RestartTracker` and the time capture came back. At entry, `recover` resets the tracker when that time is 30 s or more before now. On `Resumed`, it records the time. `recover` counts each restart with `record_consecutive_restart` and no longer calls `maybe_reset_at` or `record_restart_at`.

### Sources

- anarlog: `crates/supervisor/src/restart.rs` (the two methods and the test `consecutive_failures_stay_bounded_until_capture_is_healthy`), `crates/listener-core/src/actors/source/mod.rs` (30 s health check), `crates/listener-core/src/actors/session/supervisor.rs` (`SourceHealthy` resets the tracker).
- Longhand: the `Clock` struct, `begin_restart`, `CaptureManager::restart`, `did_wake` and `watch_for_device` in `src-tauri/src/managers/capture.rs`. Wake and device return already replace the tracker with a new one.
- Upstream commit 8860cd65 removed some `restart.rs` tests that Longhand keeps.

## Implementation Units

### U1. Port the consecutive counter

- **Goal:** KTD1.
- **Dependencies:** none.
- **Files:** `crates/supervisor/src/restart.rs`, `crates/supervisor/PROVENANCE.md`.
- **Approach:** Add the two methods and the anarlog test. In the `restart.rs` record, set Revision to the full SHA `693500220d8e253b175e7a5b88fd831c48b4583b`, and make Status true: the Longhand additions, and the tests that upstream removed in 8860cd65 and Longhand keeps.
- **Test scenarios:** The anarlog test: three restarts are within budget, the fourth is not, and after `reset` one is within budget again.
- **Verification:** `cargo test -p supervisor` and `scripts/check-provenance.sh` pass.

### U2. Recover loop with consecutive counting and the health reset

- **Goal:** R1, R2, R3.
- **Dependencies:** U1.
- **Files:** `crates/supervisor/src/recover.rs`, `crates/supervisor/src/lib.rs`.
- **Approach:** Add `SourceRestarts` and change `recover` to take it (KTD3). Export it. Replace the test `a_source_quiet_for_the_reset_period_gets_a_full_budget_again` with the 30 s scenario below. Add a doc comment on `SOURCE_RESTART_BUDGET`: the source uses only `max_restarts` and `reset_after`.
- **Execution note:** Write the spaced-failure test first and see it fail on the current loop.
- **Test scenarios:**
  - Spaced failures: each attempt fails and moves the fake clock by 5.5 s. `recover` returns `Exhausted` after three attempts with sleeps 1, 2 and 4 s. The attempt closure succeeds from the tenth attempt, so the old loop gives `Resumed` and the test fails instead of looping without end.
  - Healthy again: three losses that each come back spend the budget; the clock then moves 30 s; at the next loss, a source that stays gone gets three attempts again.
  - Not yet healthy: the same, but the clock moves 4 s; the next loss is `Exhausted` with no attempt (AE15).
  - The existing tests still pass: one failure comes back after 1 s, three failures run out the budget, a flapping source spends one budget, and a stop during the backoff calls the attempt off.
- **Verification:** `cargo test -p supervisor` passes.

### U3. Capture manager holds `SourceRestarts`

- **Goal:** R3, R4.
- **Dependencies:** U2.
- **Files:** `src-tauri/src/managers/capture.rs`.
- **Approach:** Change the `Clock` field, `begin_restart` and `restart` from `RestartTracker` to `SourceRestarts`. Wake and device return keep making a new one.
- **Test scenarios:** Test expectation: none -- a type change; U2 tests the reset on the fake clock.
- **Verification:** `cargo test -p longhand` and clippy pass.

## Verification Contract

| Gate | Command or check |
|---|---|
| Rust | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` |
| Provenance | `scripts/check-provenance.sh` |
| CI | All PR checks green on macOS and Windows, including CodeQL |

## Definition of Done

- R1 to R4 are true, and every gate in the Verification Contract passes.
- The spaced-failure test failed before U2 and passes after it.
- The PR title is `fix(capture): …`.
- The diff has no unused code from attempts that the work did not keep.
