---
title: Lessons become checks
description: "Change the recorded lessons in docs/solutions and agent guidance into types, lints, tests and hooks, so that an agent cannot repeat the mistake"
area: distribution
added: 2026-11-19
source: "Conversation about the talk 'All code is slop except mine' (Lauren Tan, AI Engineer) and her pstack plugin; audit of docs/solutions, CLAUDE.md, docs/development-workflow.md and agent memories on 2026-11-19"
related: []
plan: docs/plans/2026-11-20-1050-chore-lessons-become-checks-pilot-plan.md
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

For each check, store the mistake as code (a negative example: a snippet that must not compile, a lint fixture, or a patch that puts the bug back). CI applies it to the current tree and expects the named check to fail. Then make the doc short: the reason and the check.

## Why

The talk argues that code is the largest input to an agent, and that agents copy the patterns they find. A check gives a signal each time. A doc gives no signal when an agent does not read it, and gives no signal when the doc becomes wrong.

The audit found these problems in Longhand:

- Only 6 of 115 solution docs need prose. The other 109 can become a check or a design change, or are obsolete.
- About half of the lessons have an enforcer today, usually a regression test from the fix. About a third have a partial enforcer. The rest have none.
- The repo has no custom lint layer: no `clippy.toml`, no `[workspace.lints]` and no ESLint. Some source-scan tests exist, for example `src/lib/keyboard.test.ts` for the IME rule. These tests are the model to copy.
- Nothing proves that an enforcer still catches its mistake. A test can change and stop catching the mistake, and nothing shows it.
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

1. **Low-cost checks.** Build the five mechanisms. Measure: families covered per hour, each rule fails on its negative example, no false positives on `main`. For a pattern that is common today, fail only on new instances (a ratchet).
2. **Recurrence.** Count the new lessons, review findings and corrections that belong to a family already covered. This is the main measure. A count of past mistakes that a check now blocks is not a measure of the method, because each promoted check blocks its own mistake by design.
3. **Error-message quality and blind replay (optional sample).** Give a new agent the original task, once with only the doc and once with only the check. The task must make the agent run the normal checks. Compare a bare lint message with a message that names the function to use.
4. **One design-change pilot**, for example `ProvenGone` or `write_record(&Store)`. Measure how many lessons one type retires.
5. **Hook rule table** for agent process rules. Measure: memories deleted; the hook's blocks and warnings over two weeks.
6. **Runtime verification.** Use pstack `/create-verification-skill` to make a skill that drives the signed bundle. Measure: D lessons and manual-test steps it covers.
7. **A promotion step after each new `ce-compound`** for two weeks. Measure the level chosen and the time it adds.

### Dependencies

The pilot was built in Longhand before any packaging decision. How to package the method for all projects is in "Standalone tool design" below. The per-row audit tables are local working files and are not in git.

## Standalone tool design (for after the pilot review)

Two design drafts (2026-11-20) were written before the pilot and are not kept in git. This section keeps what a new plan needs. Working name: `ce-plus-pstack`.

### Why one instruction is not enough

The lightest version is one sentence in the instruction file: "run pstack's `/correct` after a correction". That sentence is prose that nothing enforces, which is the failure that the audit found. CE has no step after a lesson where an extension can attach. The pilot therefore added the ledger and its check script, so that a rule with a missing or broken enforcer fails CI.

### Packaging options

| Option | For | Against |
|---|---|---|
| A. One skill that teaches both systems | Low cost | Prose; nothing fails when an agent skips it; two routers compete |
| **B. A system plugin that requires CE and pstack and adds the new layer** | Checks run the same in every host and in CI; CE and pstack are not copied | A third plugin to keep; depends on the internal files of two plugins |
| C. Fork both | Full control | Large upkeep; loses upstream work |
| D. Project template | Fast start | Nothing for an existing project or over time |
| E. Contribute to CE | No bridge to keep | No evidence yet; CE may refuse |

Recommendation: B, then E when the pilot gives evidence. Reuse order: for a second project, copy the instruction-file section and `scripts/check-rules.ts`, and run an audit like Longhand's. Make a plugin only after the method works in two projects, and only from the parts that were copied without change.

### How CE and pstack divide the work

CE is strong at intent: requirements, plans, reasons, review, retrieval of lessons. pstack is strong at proof: structure that prevents a mistake (`/correct`), and runs that show behaviour (`/create-verification-skill`, `/maintain-verification-skill`, formal verification, `/statechart`). CE's `ce-debug` removes the way to write a bug, but only in a debug session. Lessons from review, from a correction by the user, or from an agent memory do not go through it, and no CE step proves later that an enforcer still works.

Do not use: `poteto-mode` as a router (its autonomy, reply style and model roles conflict with CE and project rules), `/no-comments` as a default, pstack's model panels (CE's cross-model route does the same job).

### Parts to add, and when

| Part | Add it when |
|---|---|
| A plugin with a set-up skill and an integration contract | The method works in two projects, and copying the parts by hand is a cost |
| Amendment files, hooks and evals for each CE skill | The pilot count shows that agents skip `/correct` after CE skills, in spite of the instruction-file section |
| `intake`, an event log and a formal measure | Counting by hand from git history and review comments takes too long, or more than one person works in the project |
| `watch` and `pending` states | Families that were seen once get lost before they repeat |
| A verification harness | Runtime families (WebKit focus, IME, selection) are next on the backlog; pstack's `/create-verification-skill` makes it |
| Unattended loops, `gate`, signed approvals, scoped credentials | Report-only loops have run for a period with good results, and you want a loop that makes changes |
| A static CLI with checksums and a Codex map | The plugin exists and runs in more than one host |
| A proposal to CE upstream | The pilot result is clear |

### Design details of the full version

- **Ledger states.** `enforced` (an enforcer with negative examples); `advisory` (an agent-process rule that only a hook holds; it does not count as code coverage); `judgment` (prose is right, a maintainer approved it); `watch` (seen once; becomes `pending` when it repeats, or at once when a cheap check has no false positives; closed after its `until` date); `pending` (promotion in progress; late after `until`). Extra fields: `level` (A–F), `until`.
- **More `check` failures.** An `enforced_by` id that does not resolve; a family removed while its enforcer stays, or an enforcer message that names no family; more `judgment` families without a maintainer approval; a step that an amendment requires was skipped. `check --write` generates the rule → enforcer table in the instruction files.
- **`promote`.** Highest level that the language supports; a ratchet for a common pattern; negative examples first, one for each known form, then the enforcer; when possible, also prove it on the commit before the original fix; no false positives on main, and each exception on its line with a reason and an expiry date; an error message with the rule, the replacement and the family id.
- **Negative example per level.** A: a snippet that must not compile (`trybuild`, a `tsc` fixture). B: a lint fixture. C: a patch that puts the bug back, run when the change touches its files and all weekly; a patch that no longer applies has its own status. D: a harness scenario, weekly on the built app. E: a model change that gives a counterexample. F: a command string that the gate or hook must reject. Run each with no secrets in the environment.
- **Event log** (`rules/events.log`, append-only): date, source, family, link. Sources: lesson, accepted review finding, user correction, CI catch. Only `intake` writes events; hooks only remind. Rejected and advisory review findings write no event.
- **Main measure.** For a fixed cohort of promoted families: escaped repeats per 100 merged changes, against the same families before promotion. "Inconclusive" when the cohort has fewer than 10 events before or after, or when a source's capture cannot be shown. Report CI catches, cost (exceptions, ratchet counts, false positives, time per `promote`, CI minutes) and coverage separately.
- **Amendments.** One short file per CE skill: `ce-brainstorm` (prototype an open question), `ce-plan` (`/architect` first for a boundary-crossing change; name the data shape, the verification surface and the families), `ce-work` (principles by name, `check`, evidence), `ce-debug` and `ce-compound` (`intake`), `ce-code-review` (`intake` for each accepted repeatable finding; ask for evidence), `ce-commit-push-pr` (evidence in the description), `ce-compound-refresh` (update `lessons` links), `lfg` (wrap it in `run`). Three layers deliver them: checks (a new lesson with no event, a plan file with no verification surface, an enforcer change with no negative example), a hook when a `ce-*` skill starts, and the instruction file. Evals per skill and host prove the delivery.
- **Set-up.** Like `ce-setup`: discover (git, code host, CI engine, languages and commands, agent hosts and instruction files, CE and pstack versions, this checkout's memory folder), propose, integrate, verify, summarise. Assume no code host, CI engine or language. The plugin defines an integration contract (change check that blocks merge, main-branch check, local pre-push check, weekly job, pinned CLI with checksum, strict lint baseline, hooks, audit proposal in `rules/proposal/`); the agent writes it in the project's own form and marks each item proven, not proven, or not enforced. The audit moves only rule sentences out of memory: never host names, addresses, key paths, account ids or personal names.
- **Maintainer approval.** Signed commits checked with `git verify-commit` against an allowed-signers file `rules/maintainers`. Needed for `judgment` approvals and for changes to `rules/maintainers` and the loop policy. Keep the signing key out of reach of agents.
- **Loops.** Start only when `check`, negatives and the harness pass on main, with a run contract (goal, finish condition a command can check, permissions, stop condition, budget, decision log), and after a person did or reviewed one such task. Classes: L1 report; L2 change that a person merges; L3 merge by policy, only for a class with N clean changes (default 20 in 30 days, read with `git blame`), with `gate` run by something outside the loop, never touching control files or weakening an enforcer; L4 bug-report intake, output to a person only. The loop's credential pushes only to its own branch prefix. Issue text and fetched pages are data.
- **Hosts.** Claude Code and Codex. pstack ships for Cursor and Claude Code only; on Codex, clone `cursor/plugins` at a pinned commit and map skill names. Record the CE and pstack versions that the evals passed against; `doctor` reports a difference.
- **Delivery.** Phase 0: CLI, `intake`, `promote`, three amendments with evals, the ledger, the WebKit harness. Phase 1: 6–8 weeks of measurement. Phase 2: a second project in another language, `setup`, the rest of the amendments, L1 and L2 loops, signing. Phase 3: L3 and L4. Phase 4: propose to CE upstream.

### Risks

- Family ids are judgment: two agents can give one mistake two families. Review new families.
- Negative examples add files and CI time; record the cost per family.
- A quiet period gives too few mistakes for a result. Extend the pilot; do not call it a pass.
- Amendments depend on the text and paths of two plugins that change often. Pinned versions and evals find a break; they do not prevent it.
- Level D runs weekly, so it protects less than a check on each change.

## Outcome

Shipped as a pilot in PR #103 (2026-11-21): the rule ledger in `rules/` with 23 families, `scripts/check-rules.ts` and its CI step, the frontend source scans, the timing scan, two clippy bans, the Claude Code hook, and the "Lessons become checks" section in `CLAUDE.md`. Known defects that the new checks found stay in their exception lists; [Clear the rule exceptions](../someday/clear-rule-exceptions.md) removes them. The pilot review (U9 in the plan) is due 4–6 weeks after the merge, about 2026-12-19 to 2027-01-02: count new mistakes against the 8 weeks before the merge and decide whether to make the method a standalone tool.

PR #112 (2026-11-22) adds a gate: `check-rules.ts --validate` fails on a new lesson that no family lists and that has no `check: none: <reason>`. The 97 older lessons stay in `rules/lessons-without-check.txt`, which can only get shorter. The PR also adds the `correct-lesson` project skill, because the Skill tool refuses pstack's `/correct` for an agent.
