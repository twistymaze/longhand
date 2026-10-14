---
title: Built-in Prompt Text in the Agent Skill - Plan
type: docs
date: 2026-10-14
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Built-in Prompt Text in the Agent Skill - Plan

## Goal Capsule

- **Objective:** An agent that uses the `longhand-notes` skill can read the full text of the two built-in summary prompts. It can explain what the default summary does, and it can make a changed copy of a built-in prompt as a new summary prompt without the user opening Settings.
- **Means:** a reference file in the skill holds a copy of the text, and a Rust test keeps the copy equal to the app (KTD1, KTD2).
- **Authority:** this plan, then `CLAUDE.md`, then `docs/development-workflow.md`.
- **Stop conditions:** stop and ask when the copy cannot be made byte-equal to the app text, or when the change needs an app code change other than a test or a code comment.
- **Execution profile:** one PR from a worktree. The title type is `docs` (for example `docs(skill): give agents the text of the built-in prompts`). No app release.

---

## Product Contract

### Summary

The skill gets a reference file with the text and the title instructions of "Topic summary" and "Play-by-play summary", in the form of a prompt-file body. `references/prompts.md` and the task table in `SKILL.md` send the agent to it. A test in the skill drift tests fails when the copy and the app are different.

### Problem Frame

The built-in prompts have no file in the storage folder. Their text is only in `crates/summarize/src/prompts.rs`. An agent that uses the skill knows their names and ids, but not their text. When the user asks for a changed version of the default prompt, the agent must guess, or send the user to Settings to use Duplicate. This makes the skill weak at its main prompt task: helping the user write custom prompts.

### Requirements

**Content**

- R1. The skill has one reference file with the full prompt text and the full title instructions of each built-in prompt, with its name and id.
- R2. Each built-in prompt's text and title instructions are in one block that an agent can put, unchanged, after the frontmatter of a new prompt file.
- R3. The file tells the agent that the text is the text of the current Longhand version, and that the installed app can be older or newer.

**Agent guidance**

- R4. `references/prompts.md` sends the agent to the new file to read a built-in prompt, and gives the steps to make a changed copy: a new file with a new id from `new_id.py`, the copied block, and then the change.
- R5. The task table in `SKILL.md` names the new file for "read the text of a built-in prompt".

**Drift**

- R6. A test fails when the text or the title instructions of a built-in prompt in the app and in the reference file are different.
- R7. The test fails when the app has a built-in prompt that the reference file does not have.

### Key Decisions

- **The copy is in the skill, not in app-written files.** (session-settled: user-directed — chosen over the app writing read-only built-in prompt files into the storage folder: a skill-only change needs no app release and adds no app-owned files to the user's folder; the difference between the skill on `main` and an older installed app is accepted because the built-in prompts seldom change.) Governs R1, R3.

### Scope Boundaries

- The fixed rules that Longhand adds to each prompt (data rule, naming rule) stay as a description in `references/prompts.md`. They are not copied, because a summary prompt cannot change them.
- No change to the app's behavior, to Settings, or to the built-in prompt text.

#### Deferred to Follow-Up Work

- An app command that prints the built-in prompts of the installed version: `docs/ideas/someday/app-command-line-tool.md`.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Each block is a prompt-file body.** The block holds the prompt text, the line `<!-- longhand: title instructions -->`, and the title instructions, as `render_prompt_file_with_title` in `crates/shared-settings/src/prompts.rs` writes them after the frontmatter. An agent then copies one block, and no assembly can go wrong. Both built-ins have the same title instructions (`MEETING_TITLE`), but each block repeats them, so that each block is complete.
- KTD2. **The test is in `crates/library/tests/skill_drift.rs`.** That file already holds the tests that make the skill and the app agree, and `docs/development-workflow.md` describes it so. The test reads the reference file from the repo, finds the block of each prompt from `summarize::prompts::built_ins()` by its id, and compares. `crates/library` gets `summarize` as a dev-dependency (it is already in the build through `shared-settings`).
- KTD3. **The test also does the copy procedure once.** For each block, it writes a prompt file with new frontmatter and the block, runs `scan_prompts`, and compares the text and the title instructions to the built-in. This proves that R2's "copy unchanged" gives a valid prompt with the same text, not only equal characters. This test needs no Python, so it runs on every computer.
- KTD4. **The comment on `built_ins()` names the copy.** The comment says "Their texts live only in code". It changes to say that the skill has a copy that the drift test checks, so that a person who changes a built-in prompt knows to change the reference file.

### Sources

- `crates/summarize/src/prompts.rs`: `TOPIC_SUMMARY`, `PLAY_BY_PLAY`, `MEETING_TITLE`, `built_ins()`.
- `crates/shared-settings/src/prompts.rs`: `render_prompt_file_with_title`, `TITLE_MARKER`, `scan_prompts`.
- `crates/library/tests/skill_drift.rs`: existing skill drift tests and `repo()`.
- `docs/plans/2026-10-12-1827-feat-safe-agent-edits-plan.md`: why the skill has no app command (KTD on Python scripts, user-directed).

---

## Implementation Units

### U1. Reference file and drift tests

- **Goal:** the skill holds the text of both built-in prompts, and tests keep it equal to the app.
- **Requirements:** R1, R2, R3, R6, R7; KTD1, KTD2, KTD3, KTD4.
- **Dependencies:** none.
- **Files:**
  - Create `plugins/longhand/skills/longhand-notes/references/built-in-prompts.md`.
  - Modify `crates/library/tests/skill_drift.rs`.
  - Modify `crates/library/Cargo.toml` (dev-dependency).
  - Modify `crates/summarize/src/prompts.rs` (comment only).
  - Modify `.gitattributes`: add `plugins/longhand/skills/longhand-notes/references/built-in-prompts.md text eol=lf`, with a comment like the existing ones. Windows CI checks out other text files with CRLF line ends, and then the compare fails.
- **Approach:**
  1. Write the reference file in ASD-STE100 style, like the other references: a short opening (R3), then one section for each built-in prompt with its name, its id and one fenced block (KTD1).
  2. Add the tests (KTD2, KTD3). Keep the parse simple: the section of a prompt contains its id, and its first fenced block is the body.
  3. Change the comment on `built_ins()` (KTD4).
- **Execution note:** write the tests first, and see them fail on a reference file with one changed word.
- **Patterns to follow:** `new_id_gives_an_id_that_the_prompt_scan_accepts` in `crates/library/tests/skill_drift.rs` (write a prompt file into a temporary storage folder, then scan it).
- **Test scenarios:**
  - Each built-in prompt from `built_ins()` has a block in the reference file, and the block equals its text, the marker line and its title instructions.
  - A change of one character in a block makes the compare test fail (check by hand once, do not commit).
  - A built-in prompt id that the reference file does not have makes the test fail with a message that names the id.
  - Each block, written after new frontmatter into `.longhand/prompts/` of a temporary storage folder, scans as a prompt whose text and title instructions equal the built-in.
- **Verification:** the new tests pass, and the full `skill_drift` test file passes on macOS and in the Windows CI job.

### U2. Agent guidance

- **Goal:** an agent finds the built-in text and knows how to make a changed copy.
- **Requirements:** R4, R5.
- **Dependencies:** U1.
- **Files:**
  - Modify `plugins/longhand/skills/longhand-notes/references/prompts.md`.
  - Modify `plugins/longhand/skills/longhand-notes/SKILL.md`.
- **Approach:**
  1. In `prompts.md`, change the opening paragraph: the built-in prompts have no file, but `references/built-in-prompts.md` has their text.
  2. Add the steps for "Make a changed copy of a built-in prompt" next to "Make a new prompt". Keep the existing rule: always a new id.
  3. Add one row to the task table in `SKILL.md`.
  4. Change the opening rule of `SKILL.md` that stops when there is no `.longhand/library` file: reading `references/built-in-prompts.md` needs no storage folder. Each task that reads or edits user files still needs one.
- **Patterns to follow:** the step lists in `references/prompts.md`.
- **Test expectation:** none -- Markdown guidance only; U1's tests cover the file that it points to.
- **Verification:** each link and file name in the changed text names a file that exists.

---

## Verification Contract

| Check | Command | When |
|---|---|---|
| Drift tests | `cargo test -p library --test skill_drift` | U1 |
| Format | `cargo fmt --all --check` | before each push |
| Lint | `cargo clippy --workspace --all-targets` | once at the end |
| Skill script tests | `uv run --no-project python -m unittest discover -s plugins/longhand/skills/longhand-notes/scripts/tests -t plugins/longhand/skills/longhand-notes/scripts` | once at the end (no script change is planned) |

## Definition of Done

- The reference file and both guidance changes are on one branch, and the PR title has the type `docs`.
- The drift tests pass locally and in CI.
- No experimental code or temporary test edits stay in the diff.
- The idea `docs/ideas/done/built-in-prompts-in-the-skill.md` has `plan:` set to this plan.
