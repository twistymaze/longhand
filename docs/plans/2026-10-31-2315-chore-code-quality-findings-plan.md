---
title: Code Quality findings in the skill and the bindings - Plan
type: chore
date: 2026-10-31
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Code Quality findings in the skill and the bindings - Plan

## Goal Capsule

- **Objective:** The GitHub Code Quality page of the repository shows no open finding for the two empty `except` clauses in the notes skill and for the unused import in `src/bindings.ts`, and the same findings do not come back when the bindings are generated again.
- **Means:** Reason comments on the empty `except` clauses (KTD1), and a step in the bindings export that removes the unused import (KTD2).
- **Authority:** This plan, then `CLAUDE.md`.
- **Stop conditions:** Stop and ask if the Code Quality run reports a new finding for the changed code, or if a later tauri-specta version uses `TAURI_CHANNEL` in the generated code.
- **Execution profile:** One `chore` PR. No change to how the app operates.

## Product Contract

### Summary

GitHub Code Quality reports three findings. Findings #1 and #6 (`py/empty-except`) are on two `except ...: pass` clauses in `plugins/longhand/skills/longhand-notes/scripts/_longhand.py`. Finding #7 (`js/unused-local-variable`) is on the import `Channel as TAURI_CHANNEL` in `src/bindings.ts`. tauri-specta writes that file.

### Problem Frame

Each open finding is noise in the weekly review and hides new findings. Code scanning uses CodeQL default setup. Code Quality is a separate run (actor `github-code-quality`), and it has no setting that excludes a path or generated files. Thus a CodeQL configuration file or advanced setup with `paths-ignore` does not remove finding #7. tauri-specta 2.0.0-rc.21 writes the import from a fixed template (`src/lang/globals.ts` in the crate) and has no option that removes it.

### Requirements

- R1. Each empty `except` clause in `_longhand.py` has a comment that says why the error is ignored. The behaviour does not change.
- R2. `src/bindings.ts` does not import `Channel as TAURI_CHANNEL` while no generated code uses `TAURI_CHANNEL`.
- R3. The shared `export_bindings` function removes the import, so `cargo test -p longhand export_bindings` and a debug run of the app write the same file, and the CI step "Bindings are current" keeps the result.
- R4. If generated code uses `TAURI_CHANNEL` (a command with a channel argument), the export keeps the import. `npm run typecheck` does not find a missing import here, because the file starts with `// @ts-nocheck`, so a unit test must prove R4.

### Scope Boundaries

- No switch from CodeQL default setup to advanced setup.
- No change to the tauri-specta version or to other generated code.

## Planning Contract

### Key Technical Decisions

- KTD1. **Comment, not new handling.** The two ignored errors are expected and correct to ignore: the first is a stream without `reconfigure` (for example a replaced `sys.stdout`); the second is a front-matter value in quotes that is not valid JSON, which the function returns as text.
- KTD2. **Remove the import after export.** `export_bindings` in `src-tauri/src/lib.rs` reads the written file, removes the line `\tChannel as TAURI_CHANNEL,` when the text `TAURI_CHANNEL` occurs nowhere else, and writes the file again. (session-settled: user-directed — chosen over a dismissal on GitHub and over advanced setup with `paths-ignore`: Code Quality does not read a path exclusion, and a dismissal does not stop a repeat.)

## Implementation Units

### U1. Reason comments on the empty except clauses

- **Goal:** R1.
- **Files:** `plugins/longhand/skills/longhand-notes/scripts/_longhand.py`.
- **Approach:** Add one ASD-STE100 comment line in each `except` body, before `pass`, per KTD1.
- **Test expectation:** none -- comments only; the existing skill script tests show that the behaviour did not change.

### U2. Remove the unused import from the generated bindings

- **Goal:** R2, R3, R4.
- **Files:** `src-tauri/src/lib.rs`, `src/bindings.ts` (generated again).
- **Approach:** Per KTD2. Keep the change in `export_bindings`, so that the test and the app use the same path. Use a small pure function on the text, so that a unit test can call it.
- **Test scenarios:**
  - Text with the import line and no other `TAURI_CHANNEL`: the result has no import line, and the other lines are the same.
  - Text with the import line and a use of `TAURI_CHANNEL` in a function: the result is the same as the input.
  - Text with no import line: the result is the same as the input.
- **Verification:** `cargo test -p longhand export_bindings` changes `src/bindings.ts` only on the import line.

## Verification Contract

| Check | Command |
| --- | --- |
| Skill script tests | `uv run --no-project python -m unittest discover -s plugins/longhand/skills/longhand-notes/scripts/tests -t plugins/longhand/skills/longhand-notes/scripts` |
| Bindings and new tests | `cargo test -p longhand` (scoped to the new tests and `export_bindings`) |
| Frontend | `npm run typecheck` |
| Format and lint | `cargo fmt --all --check`, `cargo clippy -p longhand` |
| After merge | Code Quality findings #1, #6 and #7 are closed on the next run on `main` |

## Definition of Done

- U1 and U2 are complete, and all checks in the Verification Contract before the "After merge" row pass.
- The PR is a `chore` PR and is merge-ready.
- After merge, the next Code Quality run on `main` shows findings #1, #6 and #7 as closed.
- The diff has no code from approaches that were not used.
