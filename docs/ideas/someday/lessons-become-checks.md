---
title: Lessons become checks
description: "Change the recorded lessons in docs/solutions and agent guidance into types, lints, tests and hooks, so that an agent cannot repeat the mistake"
area: distribution
added: 2026-11-19
source: "Conversation about the talk 'All code is slop except mine' (Lauren Tan, AI Engineer) and her pstack plugin; audit of docs/solutions, CLAUDE.md, docs/development-workflow.md and agent memories on 2026-11-19"
related: []
---

# Lessons become checks

## Idea

Most lessons in `docs/solutions/` and in the agent guidance are prose. Nothing fails when an agent does not read them. Change each lesson into the highest level of enforcement that works, in this order:

1. **A** Design change: a type, an API or a structure that makes the mistake impossible.
2. **B** Lint or static scan, with an error message that names the fix.
3. **C** Test.
4. **D** Runtime verification of the real app.
5. **E** Formal model.
6. **F** Process gate: CI step, git hook or Claude Code hook.
7. **G** Prose, only for a real judgment.

For each check, prove that it fails on the original mistake: check out the commit before the fix, or put the mistake back. Then make the doc one line that names the check, or delete the doc.

## Why

The talk argues that code is the largest input to an agent, and that agents copy the patterns they find. A check gives a signal each time. A doc gives no signal when an agent does not read it, and gives no signal when the doc becomes wrong.

The audit found these problems in Longhand:

- Only 6 of 115 solution docs need prose. The other 109 can become a check or a design change, or are obsolete.
- Almost no lesson has a project-specific check. The repo has no `clippy.toml`, no `[workspace.lints]` and no ESLint. Some source-scan tests exist, for example `src/lib/keyboard.test.ts` for the IME rule. These tests are the model to copy.
- Prose is stale. 15 solution docs show `bun` commands. Some docs cite a deleted workflow file, a deleted crate or old line numbers (185 `file:line` citations). One doc contradicts `src-tauri/capabilities/default.json`. Some agent memories are no longer true.

## Notes / Current state

### Audit counts (2026-11-19)

| Level | Solutions (115) | Memories and rules (148) |
|---|---|---|
| A design change | 43 | 26 |
| B lint / scan | 14 | 15 |
| C test | 23 | 2 |
| D runtime | 6 | 0 |
| E formal | 4 | 0 |
| F process gate | 13 | 41 |
| G judgment | 6 | 31 |
| H obsolete or environment fact | 6 | 33 |

Design change (A) is the largest level, because Rust types can hold most rules. In a TypeScript codebase, lint rules usually do more of the work.

### Families and the mechanism for each

| Family | Lessons | Mechanism |
|---|---|---|
| Two code paths drifted. The fix made one shared function, but new code can still go around it. | about 9 | `clippy.toml` `disallowed-methods` with a reason that names the shared function; a frontend source-scan test |
| Absence read as evidence; act only on this computer's own work; crash between a step and its record | about 14 | Signatures that require proof (`ProvenGone`, `OwnRuns`, `write_record(&Store, …)`); one test harness that injects faults and partial syncs |
| Lock discipline (comments only) | 4 | `clippy::significant_drop_in_scrutinee`; lock-holding types |
| Tests that depend on timing | 5 | One script that finds short real-time limits, bare sleeps and closed "reserved" ports |
| Synced names reach a file path | 4 | `NoteId` and `AudioName` newtypes that parse at the boundary |
| Dependency defaults and joined features (serde_json, reqwest, ort) | 4 | `disallowed-methods` that send each crate through one helper |
| Two lists kept the same by hand | 4 | One equality test for each pair |
| WebKit behaviour that happy-dom does not show (focus, selection, IME, hover) | about 11 | A vitest setup in which a click does not focus a button, plus a runtime harness |
| CI workflow faults | 3 | actionlint and zizmor |
| Agent process mistakes (PR, push, commit, shell commands) | about 35 | One PreToolUse hook script with a table of rules |

About 60 % of all items fall into five low-cost mechanisms: `clippy.toml` with workspace lints, a frontend source-scan test, a script for timing-dependent tests, a hook rule table, and newtypes at the boundaries.

### Known gaps found during the audit

- 63 `map_err(|e| e.to_string())` calls in `src-tauri/src/commands` send internal errors to the UI. A solution doc says not to do this.
- Pinned migration hashes cover migrations 1 to 21. Migrations 22 to 26 have no hash (`crates/session-core/tests/migrations.rs`).
- `crates/pipeline/src/type_move.rs` calls `rename_new` directly. The folder-in-use doc says that all rename sites use `rename_new_waiting`. Not yet verified whether this is deliberate.
- `scripts/check-dependency-age.ts` tells the agent to add an exception. The bundled-entries lesson says never to do that.

### Experiments, in order

1. **Low-cost checks.** Build the five mechanisms. Measure: lessons retired per hour, each rule fails on its original faulty commit, no false positives on `main`.
2. **Blind replay.** For 5 past bugs, give a new agent the original task at the commit before the fix. One agent gets only the doc, and the other gets only the check. Count how often each repeats the mistake. The task must look normal: no words such as "eval".
3. **Error-message quality.** Same replay. Compare a bare lint message with a message that names the function to use.
4. **One design-change pilot**, for example `ProvenGone` or `write_record(&Store)`. Measure how many lessons one type retires.
5. **Hook rule table** for agent process rules. Measure: memories deleted; the hook's blocks and warnings over two weeks.
6. **Runtime verification.** Use pstack `/create-verification-skill` to make a skill that drives the signed bundle. Measure: D lessons and manual-test steps it covers.
7. **A promotion step after each new `ce-compound`** for two weeks. Measure the level chosen and the time it adds.

### Dependencies

How to package this method for all projects comes first. It is a separate decision: combine compound engineering with pstack (`/correct`, `/create-verification-skill`, `/create-formal-verification-skill`). The per-row audit tables are local working files and are not in git.
