---
title: Plain Language and the Save Rename - Plan
type: refactor
date: 2026-09-24
topic: plain-language-save-rename
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Plain Language and the Save Rename - Plan

## Goal Capsule

- **Objective:** A reader of Longhand's code, docs or UI finds one plain word for each concept, and no text in the repository uses "land", "lands", "landed" or "landing" except to ban them. New text follows written rules for plain language, so the same type of problem does not start again.
- **Means:** Rename the process that moves a session into the Destination to "Save", with the end state "Saved". Add language rules to the project guidance.
- **Product authority:** This Product Contract wins on what changes. The Planning Contract wins on how, within the Product Contract. A unit overrides neither.
- **Execution profile:** A mechanical rename across Rust, TypeScript and Markdown. No behavior changes, except the UI text changes that R4 makes and the folder dialog title.
- **Stop conditions:** Stop and ask if a crash-recovery test in the save crate or the pipeline fails after a rename and the cause is not a missed name. Stop and ask if a rename seems to need a change in behavior.
- **Who finishes:** `ce-work` does U1 to U6 and runs the Verification Contract. The user deletes the old data (R11) and does the signed-bundle check.
- **Open blockers:** None.

---

## Product Contract

### Summary

Remove "land", "lands", "landed" and "landing" from code, schema, UI text, comments and docs. They stay only where a rule names them as terms to avoid. The process that moves a finished session into the Destination becomes "Save", and its end state becomes "Saved". New rules in the project guidance make code and docs follow ASD-STE100 and make UI text follow plain-language rules.

### Problem Frame

The walking skeleton used "land" as a metaphor for "a process gets to completion". The term spread into a crate name, an enum, a session state, a database column, a marker file name, test names, UI types, the glossary, the plan and a solutions doc: about 430 uses in 53 files. The UI hides the term, except in the folder dialog title, and says "saved" or "Ready", so the code and the UI use different words for the same step. The user does not want this metaphor anywhere. No rule stops an agent from adding a different metaphor of the same type in the future.

The app is not in use. All existing data is test data and the user can delete it.

### Key Decisions

- **The process is "Save" and its end state is "Saved".** It matches the words the UI already shows. (session-settled: user-directed — chosen over Move/Moved, Finalize/Finalized and Deliver/Delivered: it matches the UI text that already says "Saving to your folder" and "saved".) Governs R1, R2, R4.
- **All docs change, including history.** The walking-skeleton plan, its checkpoint log and the solutions doc change too, so no tracked file keeps the term. (session-settled: user-directed — chosen over changing only current docs and over changing only code: a partial change leaves two words for one step.) Governs R6.
- **Stored data breaks. No migration.** The state names, the column, the marker file and the schema change in place. The user deletes the old test data by hand. (session-settled: user-directed — chosen over a data migration: the app is not in use and the data is test data.) Governs R3, R11.
- **Written rules, no automatic check.** (session-settled: user-directed — chosen over a check script with a CI step: the user does not want an automatic check.) Governs R8, R9.
- **Two writing standards.** ASD-STE100 applies to code, comments, docs and agent guidance. UI text follows lighter plain-language rules, because STE's controlled vocabulary makes a UI sound mechanical. (session-settled: user-approved — proposed with the trade-off shown, and the user agreed.) Governs R8, R9.
- **The retroactive change is limited to the "land" terms.** Other existing text does not get an STE rewrite now. The rules apply to new and edited text. (session-settled: user-approved — the user said "landed" is the one term to fix retroactively.) Governs R6, R8.

### Requirements

**Rename in code and data**

- R1. No identifier in the repository contains "land", "lands", "landed" or "landing": crate names, module names, types, enum variants, functions, constants, test names and file names. The replacement uses "save" or "saved" when the name means the move into the Destination.
- R2. The stored and serialized names change with the code: the session state string, the stage name, the marker file name and the TypeScript types that the frontend reads.
- R3. The database column that holds a saved session's folder path gets a name without the term. The initial schema changes in place and no migration is added.
- R4. "Save" and "saved" have one meaning in the code, comments and UI text: the move of a session into the Destination. Other writes that use "save" today, such as note writes, settings writes and the Keychain entry, use a different word. Calls to third-party APIs whose names contain "save" do not change. *Revised after merge:* this reading was too strict. "Save" means to keep something in a place, so saving a key in Keychain or saving a setting is the same meaning. The Keychain button reads "Save" again, and Models.tsx uses `save` again. The settings test names and the audio error text keep the U4 wording.
- R5. A use of the term that does not mean the move into the Destination changes to plain words that fit its meaning. Examples: a model download that finishes, a sample rate that is written to a header, a file that goes into a folder.

**Rename in text and docs**

- R6. Comments, UI text, the project guidance, `CONCEPTS.md`, `docs/ideas/`, `docs/solutions/` and `docs/plans/` contain no use of the term. This includes the walking-skeleton plan's unit text and checkpoint log. The only exceptions are the places that name the term to ban it: the `CONCEPTS.md` entry (R7), the `CLAUDE.md` rule (R8), the memory (R10) and this plan.
- R7. The `CONCEPTS.md` entry for the process is named after "Save". The entries for Staging and Home folder use the new word. The entry records "land" and its forms as terms to avoid in all text, not only in UI text.

**Rules for new text**

- R8. The project `CLAUDE.md` tells agents to write code identifiers, comments, docs and agent guidance to ASD-STE100: one meaning for each word, one word for each concept, short sentences, active voice and no metaphors. The rule names "land" as an example of a metaphor to avoid.
- R9. The project `CLAUDE.md` tells agents to write UI text in plain language: short and direct, one term for each concept, no metaphors and no jargon. UI text does not have to use STE's controlled vocabulary.
- R10. The agent memory that bans "landed" in UI text widens to cover code and docs, and points to the new rules.

**Data cleanup**

- R11. The user deletes the old database files, and any hidden staging folders that an old build left in the Destination, before the first run of the new build. The app makes a new database at the next start. The downloaded models and `settings.json` stay.

### Success Criteria

- A case-insensitive whole-word search of tracked files for "land", "lands", "landed" and "landing" finds matches only in the exceptions that R6 lists. A search for identifiers that contain the term (for example `LandState` or `set_landed_path`) finds matches only in this plan.
- All local gates pass after the rename: the Rust tests, `bun test`, the typecheck and `scripts/check-windows.sh`.
- A session goes through the full process in the signed bundle, and the UI shows the same text as before the change, except the changes that R4 makes and the folder dialog title that R6 changes.

### Scope Boundaries

- Git history, branch names and old commit messages do not change.
- `reference-source/` does not change. It is read-only and not tracked.
- Other existing text does not get an STE rewrite now. Only the "land" terms change retroactively.
- No check script and no CI step.
- No change to what the UI shows, except where R4 changes a use of "save" and where R6 removes the term from the folder dialog title.

### Dependencies / Assumptions

- The app has no users and no data that must be kept.
- `src/bindings.ts` is generated by tauri-specta from the Rust types. The renamed Rust types produce the new TypeScript names. Nobody edits the file by hand.

### Outstanding Questions

**Deferred to Implementation**

- The exact words in each comment, log line and doc sentence that R5 and R6 change. KTD2 and KTD7 give the names and the rules. The implementer picks the words at each site.

### Sources / Research

- Uses by file: `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md` (114), `crates/pipeline/tests/runner.rs` (57), `crates/pipeline/src/land.rs` (50), `docs/solutions/logic-errors/save-state-machine-crash-window-recovery.md` (35), `crates/pipeline/src/runner.rs` (26), `src-tauri/src/commands/session.rs` (17), `crates/landing/` (about 30).
- Stored names: `SessionState::Landed` is serialized as JSON into the `state` column (`crates/session-core/src/jobs.rs`). The `landed_path` column is in `crates/session-core/migrations/0001_init.sql`, the only migration. The marker file is `MARKER` in `crates/landing/src/lib.rs`.
- The frontend compares the state string `"landed"` and the stage `"land"` in `src/processing/describe.ts` and `src/settings/Settings.tsx`.
- Uses with a different meaning: `crates/pipeline/src/runner.rs` (a model download), `crates/pipeline/tests/assemble.rs` (a sample rate in a header), `crates/pipeline/src/recover.rs` (a file in a folder), `crates/audio-cpal/tests/hardware.rs` (a test name).
- Existing uses of "save" with a different meaning: `src/settings/EndpointForm.tsx` (the Keychain entry), `src/settings/Models.tsx` and `src-tauri/src/managers/settings.rs` (settings writes), `crates/recorder/src/chunks.rs` (audio in the Working location, in an error the user sees).
- The database is `~/Library/Application Support/net.ewal.longhand/longhand.db`, with its `-shm` and `-wal` files.

**Product Contract preservation:** changed: R11 — adds the old staging folders to the manual cleanup, because a new build does not recognize their old marker file. Restructured, no scope change: the Problem Frame, Success Criteria and Scope Boundaries now name the folder dialog title, which R6 already required to change. Planning resolved the Outstanding Questions in place. The rest of the Product Contract is unchanged.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **The save crate is `session-save`, not `save`.** A crate named `save` next to a pipeline module named `save` makes `pub use save::…` ambiguous and makes `use crate::save` hide the crate. The name follows `session-core`. Governs R1.
- KTD2. **The rename map.** Each old name gets the new name below. A name not in the table follows the same pattern.

  | Old | New |
  |---|---|
  | crate `landing` (`crates/landing/`) | crate `session-save` (`crates/session-save/`, library `session_save`) |
  | `LandState` (variant `Landed`) | `SaveState` (variant `Saved`) |
  | `LandingError` | `SaveError` |
  | module `landing::post_landing` | module `session_save::home_write` |
  | `MARKER` value `.longhand-landing` | `.longhand-saving` |
  | module `pipeline::land`, struct `Landed` | module `pipeline::save`, struct `Saved` |
  | `Stage::Land` (stored `"land"`) | `Stage::Save` (stored `"save"`) |
  | `SessionState::Landed` (stored `"landed"`) | `SessionState::Saved` (stored `"saved"`) |
  | `Event::Landed`, `LandedEdit`, `LandingFailed` | `Event::Saved`, `SavedEdit`, `SaveFailed` |
  | `RunnerEvent::Landed`, `ProcessingStep::Landed` | `RunnerEvent::Saved`, `ProcessingStep::Saved` |
  | `CaptureError::Landing` | `CaptureError::Save` |
  | column `landed_path`, `set_landed_path`, `land::landed_path` | column `home_path`, `set_home_path`, `save::home_path` |
  | `notify_landed` | `notify_saved` |

  Governs R1, R2, R3.
- KTD3. **The column is `home_path`.** `CONCEPTS.md` calls this folder the Home folder, and the code already uses "home" for it (`home_intact`). A `saved_path` name would give the folder two names. Governs R3.
- KTD4. **Serialized names follow the Rust names. No serde aliases.** The snake_case serde derive makes the stored and emitted strings match the new variants. Aliases for the old strings would keep the term in the code. This is the how-level choice for the no-migration Key Decision, and it inherits that label. (session-settled: user-directed — chosen over a data migration: the app is not in use and the data is test data.) Governs R2, R3.
- KTD5. **Each unit builds and passes the gates on its own.** U1 and U2 cross the crate, Tauri and frontend layers, so each one changes all its layers in one commit and regenerates `src/bindings.ts`. The doc and rules units come last, so the docs describe the code as it is.
- KTD6. **A text search follows each code unit.** The compiler does not check string literals: the TypeScript compares `SessionEntry.state`, which is typed `string`, and tests assert on strings such as `Some("landed")`. The Verification Contract search is the check.
- KTD7. **Words for the other uses of "save".** The Keychain button and its confirmation use "Store" and "Stored". Code that writes settings or notes uses "write" or "apply". Error text about audio in the Working location uses "recorded". Governs R4. *Revised after merge:* this reading was too strict. "Save" means to keep something in a place, so saving a key in Keychain or saving a setting is the same meaning. The Keychain button reads "Save" again, and Models.tsx uses `save` again. The settings test names and the audio error text keep the U4 wording.
- KTD8. **No CI edit.** The cargo steps and their package selection do not change, and rust-cache already keys on `Cargo.lock`, so the `prefix-key` stays. `scripts/check-windows.sh` excludes a fixed list that does not name the crate, so the renamed crate is still linted.

### Assumptions

- The crash-recovery rules in `docs/solutions/logic-errors/save-state-machine-crash-window-recovery.md` depend on the order of steps and on the marker's lifetime, not on names. A rename that keeps the code order keeps them.

---

## Implementation Units

### U1. Rename the save crate and the pipeline save module

- **Goal:** The crate, its types, its marker file and the pipeline module that drives it use the KTD2 names.
- **Requirements:** R1, R2 (KTD1, KTD2, KTD4).
- **Dependencies:** None.
- **Files:**
  - Move `crates/landing/` to `crates/session-save/`: `Cargo.toml`, `src/lib.rs`, `src/post_landing.rs` to `src/home_write.rs`, `src/rename.rs`, `src/stage.rs`, `tests/landing.rs` to `tests/save.rs`, `tests/post_landing.rs` to `tests/home_write.rs`.
  - Move `crates/pipeline/src/land.rs` to `crates/pipeline/src/save.rs`. Modify `crates/pipeline/src/lib.rs`, `crates/pipeline/src/runner.rs`, `crates/pipeline/Cargo.toml`.
  - Modify `crates/pipeline/tests/runner.rs`, `crates/pipeline/tests/end_to_end.rs`.
  - Modify `src-tauri/Cargo.toml`, `src-tauri/src/managers/capture.rs`, `src-tauri/src/managers/processing.rs`, `src-tauri/src/commands/session.rs`, `Cargo.lock`.
- **Approach:**
  1. Move the files with `git mv` so that history follows them.
  2. Rename the package, the dependency entries and every `landing::` path.
  3. Rename the types, the variants, the constant value, the functions and the test names in these crates, per KTD2.
  4. Keep the order of steps and checks in `step` and in the pipeline's save routing unchanged.
- **Patterns to follow:** `crates/session-core/Cargo.toml` for a `session-` crate name.
- **Execution note:** Before any edit, run the save crate and pipeline tests once and note which pass. After the rename, the same tests must pass with only names changed.
- **Test scenarios:**
  - The renamed save crate tests pass unchanged except for names: a same-volume save moves once, a cross-volume save copies into staging then renames, and a relaunch after a crash in each step finds the folder.
  - The marker test asserts the new file name `.longhand-saving`. It is written before the first move and removed in the `Saved` step.
  - The pipeline runner tests that cover the save stage pass, including the crash-window cases and the "no destination" and "is missing" cases.
  - The end-to-end test saves a session into its Destination.
- **Verification:** The workspace builds. The save crate and pipeline tests pass. The KTD6 search finds no match in the files this unit touched.

### U2. Rename the session states, the stage, the events and the column

- **Goal:** The session model, the database schema, the Tauri types and the frontend use the KTD2 names.
- **Requirements:** R1, R2, R3 (KTD2, KTD3, KTD4, KTD6).
- **Dependencies:** U1.
- **Files:**
  - Modify `crates/session-core/src/state.rs`, `crates/session-core/src/jobs.rs`, `crates/session-core/migrations/0001_init.sql`, `crates/session-core/src/folder.rs`.
  - Modify `crates/session-core/tests/state_machine.rs`, `crates/session-core/tests/jobs.rs`.
  - Modify `crates/pipeline/src/save.rs`, `crates/pipeline/src/runner.rs`, `crates/pipeline/tests/runner.rs`, `crates/pipeline/tests/end_to_end.rs`.
  - Modify `src-tauri/src/managers/processing.rs`, `src-tauri/src/commands/session.rs`, `src-tauri/src/commands/processing.rs`.
  - Regenerate `src/bindings.ts`.
  - Modify `src/processing/describe.ts`, `src/processing/describe.test.ts`, `src/settings/Settings.tsx`, `src/session/SessionView.tsx`.
- **Approach:**
  1. Rename the enum variants, the events, the stage and the column, per KTD2.
  2. Edit `0001_init.sql` in place, including its header comment.
  3. Update the string literals that the compiler does not check: the `name()` strings in `state.rs`, the expected strings in `state_machine.rs`, and the frontend comparisons and stage key.
  4. Run the tests to regenerate `src/bindings.ts`, then check that its diff has only the renames.
- **Patterns to follow:** The existing snake_case serde derive on these enums.
- **Test scenarios:**
  - `SessionState::Saved` has the name `"saved"`, and `Stage::Save` has the stage key `"save"`.
  - The state machine test's transitions: `Processing` with `Saved` goes to `"saved"`, `Saved` with `SavedEdit` stays `"saved"`, and `Processing` with `SaveFailed` goes to `"awaiting_destination"`.
  - A saved session's `home_path` is written by `set_home_path` and read back by the session queries in `jobs.rs`.
  - `describe.test.ts` shows the save stage as "Saving to your folder" and a saved session as Ready.
  - The generated `src/bindings.ts` contains `kind: "saved"` and `home_path`, and has no other change.
- **Verification:** All gates in the Verification Contract pass. The KTD6 search finds no match in `crates/`, `src-tauri/` or `src/`, except the uses that U3 and U4 own.

### U3. Replace the other uses of the term in code

- **Goal:** No comment, log line, test name or UI string in code uses the term.
- **Requirements:** R1, R5, R6.
- **Dependencies:** U2.
- **Files:** The remaining matches of the KTD6 search in code. The known sites:
  - `crates/pipeline/src/runner.rs` (a model download), `crates/pipeline/src/recover.rs`, `crates/pipeline/src/assemble.rs`, `crates/pipeline/src/verify.rs`, `crates/pipeline/src/summarize.rs`, `crates/pipeline/tests/assemble.rs`.
  - `crates/audio-cpal/tests/hardware.rs`, `crates/recorder/src/wav.rs`, `crates/recorder/src/storage.rs`, `crates/recorder/tests/chunks.rs`.
  - `crates/session-core/src/anchors.rs`, `crates/session-core/src/timeline.rs`, `crates/summarize/src/chunk.rs`.
  - `src-tauri/src/commands/settings.rs` (the folder dialog title), `src-tauri/src/lib.rs`, `src-tauri/build.rs`, `src-tauri/src/managers/settings.rs`, `src-tauri/src/managers/destination.rs`, `src-tauri/src/commands/readiness.rs`.
  - `src/bindings.ts` doc comments (through the Rust doc comments they come from), `src/session/TranscriptPane.tsx`, `src/session/SummaryPane.tsx`, `src/session/NotesPane.tsx`, `src/index.css`, `.gitignore`.
- **Approach:**
  1. When a use means the save step, use "save" or "saved".
  2. For any other meaning, use the plain words for that meaning (R5).
  3. Change the log strings. No test asserts on them.
  4. The folder dialog title uses the same words as the Destination picker: "Choose where sessions are saved".
- **Test scenarios:**
  - Test expectation: none -- only comments, names and text change, and the U2 gates cover the renamed tests. A renamed test must keep its assertions.
- **Verification:** The KTD6 search finds no match in `crates/`, `src-tauri/`, `src/` or `.gitignore`. All gates pass.

### U4. Give "save" one meaning

- **Goal:** "Save" in our own code and UI means only the move into the Destination.
- **Requirements:** R4 (KTD7).
- **Dependencies:** U3.
- **Files:**
  - Modify `src/settings/EndpointForm.tsx`, `src/settings/Models.tsx`.
  - Modify `src-tauri/src/managers/settings.rs` (test variable names and the test name; not the `store.save()` calls).
  - Modify `crates/recorder/src/chunks.rs` (the error text about audio in the Working location).
- **Approach:**
  1. Apply KTD7 at each site.
  2. Leave third-party calls and config keys as they are: `store.save()`, autosave, the Actions cache and `scripts/make_icons.py`.
- **Test scenarios:**
  - The Keychain form's button reads "Store" and, after a store, the status line reads "Stored in Keychain for …".
  - The renamed settings test still proves that an unknown field loads and is dropped on the next write.
  - The recorder's low-disk error still names the bytes free and the floor.
- **Verification:** A case-insensitive search for "save" in `crates/`, `src-tauri/src/` and `src/` finds only the save step and the third-party calls in the Approach. All gates pass.

### U5. Change the docs and the glossary

- **Goal:** The docs use "Save" and "Saved", and the glossary records the terms to avoid.
- **Requirements:** R6, R7.
- **Dependencies:** U4.
- **Files:** `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`, `docs/solutions/logic-errors/save-state-machine-crash-window-recovery.md`, `docs/ideas/ux-redesign.md`, `CONCEPTS.md`.
- **Approach:**
  1. In the walking-skeleton plan, the solutions doc and `docs/ideas/ux-redesign.md`, change each use, including code names, which must match the KTD2 names. The unit IDs and the checkpoint log's order and facts do not change.
  2. Update the solutions doc's frontmatter `module` and `tags` if they use the term.
  3. In `CONCEPTS.md`, rename the "Landing" entry to "Save" and rewrite the Staging and Home folder entries. The avoid line lists "land", "lands", "landed" and "landing" for all text.
- **Test scenarios:**
  - Test expectation: none -- docs only.
- **Verification:** The KTD6 search finds matches only in `CONCEPTS.md`'s avoid line and this plan. The code names in the solutions doc exist in the code.

### U6. Add the language rules

- **Goal:** Agents that read the project guidance write code and docs to ASD-STE100 and UI text in plain language.
- **Requirements:** R8, R9, R10.
- **Dependencies:** U5.
- **Files:** `CLAUDE.md`. The agent memory `no-landed-in-user-facing-text.md` and its `MEMORY.md` line are in the user's Claude memory folder, outside the repository.
- **Approach:**
  1. Add a "Language" section to `CLAUDE.md` with two short rule lists: one for code, comments, docs and agent guidance (R8), and one for UI text (R9).
  2. Point to `CONCEPTS.md` for the terms to avoid, so the list has one owner.
  3. Rewrite the memory to cover code and docs, rename it to match, and update its index line (R10).
- **Test scenarios:**
  - Test expectation: none -- guidance only.
- **Verification:** `CLAUDE.md` has the section, written in the style it asks for. The memory index has one line for the widened memory and none for the old one.

---

## Verification Contract

| Check | Command | When |
|---|---|---|
| Frontend build and typecheck | `bun run build`, `bun run typecheck` | After U2, U3, U4 |
| Frontend tests | `bun run test` | After U2, U3, U4 |
| Rust format and lint | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` | After U1 to U4 |
| Windows lint | `scripts/check-windows.sh` | After U1 to U4 |
| Rust tests (these also regenerate the bindings) | `cargo test --workspace` | After U1 to U4 |
| Bindings are current | `git diff --exit-code src/bindings.ts` after the tests, once the regenerated file is committed | After U1, U2 |
| Provenance | `scripts/check-provenance.sh` | After U1 |
| Term search (KTD6) | `git grep -nIi land \| grep -vi wayland` (on 2026-09-24 every other match was the term) | After each unit |
| Signed bundle | `bun tauri build --bundles app`, then record, process and save one session | After U4, by the user, after R11 |

cargo and uv commands need the sandbox bypass in this repository.

---

## Definition of Done

- U1 to U6 are complete, and each unit's Verification holds.
- The term search finds matches only in the R6 exceptions.
- All gates in the Verification Contract pass on the last commit.
- The crash-recovery tests for the save step pass with only names changed.
- The user has deleted the old data (R11), and a session saves in the signed bundle.
- No leftover old files remain: no `crates/landing/`, no `crates/pipeline/src/land.rs`, and no unused code from abandoned attempts.
