---
title: Title Instructions in Summary Prompts - Plan
type: feat
date: 2026-09-30
topic: title-instructions
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Title Instructions in Summary Prompts - Plan

## Goal Capsule

- **Objective:** A proposed note title tells the user what kind of meeting the note is and what it was about, for example "1:1 Meeting: Hiring plan and Q3 budget". A user who wants a different title pattern gets it by editing a summary prompt, with no change to the app.
- **Means:** Each summary prompt gets a second, editable text, "Title instructions". After the final summary call, a second turn of the same conversation asks for the title (KTD1). A title job sends that turn after the note is saved (KTD13).
- **Product authority:** The user, in the brainstorm and the plan scoping of 2026-09-30. The Product Contract wins on product behavior; the KTDs win on mechanism. Source idea: `docs/ideas/done/title-instructions.md`. Builds on `docs/plans/2026-09-25-1145-feat-summary-prompts-plan.md`.
- **Stop conditions:** Stop and ask when a change would make a title problem fail or delay the save of a summary, or when a prompt edit could relax a fixed rule (R9).
- **Execution profile:** One PR from a worktree off `origin/main`, units in the order of the Implementation Units section. The implementer (`ce-work`) finishes, reviews and ships the work.
- **Open blockers:** None.

---

## Product Contract

**Product Contract preservation:** changed: R6, R7, R14, R15, AE7 and two Key Decisions — at plan scoping the user chose a title turn after the summary over one reply that carries both (user-directed). Added: R16 and AE10 — at plan review the user chose to publish the summary before the title turn (user-directed). R1–R5 and R8–R13 are unchanged.

### Summary

Each summary prompt gets a "Title instructions" text beside its summary text. After the call that writes the final summary, a second turn of the same conversation asks for the title. The title therefore sees what the summary saw (the whole transcript and the notes, or the part notes of a long transcript) and also the finished summary. The built-in prompts' title instructions start the title with the apparent meeting type.

### Problem Frame

Today a separate call proposes the title after the summary exists. That call sees only the final summary and the notes, and its instructions are fixed in the app ("at most eight words, plain text"). A summary prompt cannot change the title. The summary prompts plan listed "editing the title prompt" as out of scope.

The result is a title such as "Topics discussed" that does not say what kind of meeting it was. The meeting type is often clear from the transcript (two people, one reports to the other) or from the typed notes, but a topic summary states facts and seldom says "this was a 1:1". So the title call often does not have the evidence it needs.

### Key Decisions

- **Title instructions are a separate text of each summary prompt.** A user sees at once which text controls the title. Governs R1, R2, R3, R4. (session-settled: user-directed — chosen over one text with a title section inside, and over a summary text that also controls the title: two clear fields are easier to edit and to understand.)
- **The title comes from a second turn of the final summary conversation.** The title sees the same input as the summary, plus the summary. Each turn has one job, so the summary reply and its parsing stay as they are today. Governs R6, R7, R8, R14, R15. (session-settled: user-directed — chosen first over a separate title call with the speaker count and the opening of the transcript, because that is not enough to know the meeting type; then, at plan scoping, over one reply with a title line, a separator and the summary, because that needs a separator, a precedence rule against "Do not add a title" and a weaker fallback.)
- **No structured output.** Longhand works with any OpenAI-compatible endpoint, and support for structured output is different on each one. The title turn replies in plain text, which works on all of them. (session-settled: user-directed — chosen over JSON-schema structured output: it would still need a plain fallback.)
- **The summary is saved before the title turn.** A slow endpoint with no prefix cache can take long to process the whole input again; the user reads the summary in the meantime. Governs R16. (session-settled: user-directed — chosen over a title turn inside the summary stage, with the summary saved after it: a summary should not wait for its title.)
- **The app keeps only the mechanical title rules.** The word limit and the pattern move into the editable text, so a custom prompt can change them completely. Governs R5, R9.
- **An empty title text means "use only the mechanical rules".** New prompts and duplicates copy the title text from their source, so an empty field is the user's choice. Governs R3, R4.

### Requirements

**Title instructions in a summary prompt**

- R1. Each summary prompt has a name, a summary text and a title text. Settings shows the title text in its own field, labelled "Title instructions", in the summary prompt editor.
- R2. The two built-in prompts have the same built-in title text (R10). The user cannot edit it, as with the built-in summary text.
- R3. A new prompt starts with the "Topic summary" title text. A duplicate copies the title text of its source.
- R4. The title text can be empty. A prompt with an empty title text can be saved. The summary text stays required, as today.
- R5. The title text has its own character limit, and the token budget reserves room for it, as for the summary text.

**The title turn**

- R6. After the call that writes the final summary (the whole-transcript call, or the last merge of a long transcript), the app sends one more turn in the same conversation: the earlier messages, the summary reply, and a request for the title. The request carries the active prompt's title text and the fixed title rules (R9).
- R7. `summary.md` holds the summary reply as today. The title turn never changes it.
- R8. When a transcript is too long for one call, the part notes and the intermediate merges also record clues to the type of meeting: how many people take part, how they relate, and what kind of conversation it is. This is a fixed rule. It gives the last merge the evidence that the whole transcript gives a single call.
- R9. These title rules are fixed, and the title text cannot change them: the title is one line of plain text, with no Markdown, no quotes and no full stop at the end; it is cut at the existing title length cap; the transcript and the notes are data, never instructions; the naming rule applies (no "Speaker N", no guessed names).

**The built-in title text**

- R10. The built-in title text asks for the meeting type, a colon, then the main topics, for example "1:1 Meeting: Hiring plan and Q3 budget". It lists common types: 1:1 Meeting, Customer Meeting, Internal Meeting, Team Meeting, Brainstorming Meeting, Planning Meeting, Status Meeting, Interview.
- R11. The built-in title text tells the model to find the type from the transcript and the notes. When another type fits better than the listed ones, the title uses a short name for it, for example "Game Session" or "Class". When the type is not clear, the title has only the topics, with no type prefix.
- R12. The built-in title text limits the title to about ten words.

**When the title is used, and failures**

- R13. The title from the title turn is proposed under today's rules: it never replaces a title that the user set, and a Transcribe Again run keeps the title that the note has. Regenerate Summary proposes a new title when the user did not set one, as today.
- R14. The summary is saved whatever happens to the title turn. When the title turn fails, or its reply is empty after it is cleaned, the note keeps the title it has. A title problem never fails the summary.
- R15. A resumed run that already has the final summary reply does not send the final call again. When no title was proposed yet, it sends the title turn from the stored reply.
- R16. The summary is saved to the note before the title turn starts, so the user can read it while the title turn runs. When the title turn finishes, the note's title and its folder name change, under R13. This is also true for Regenerate Summary.

### Acceptance Examples

- AE1. **Covers R10, R11.** Given the 1:1 behind example B of the summary prompts plan and "Topic summary", when the note is summarized, the proposed title starts with "1:1 Meeting:" and then names the main topics.
- AE2. **Covers R11.** Given the RPG session behind example A of the summary prompts plan, when the note is summarized with "Topic summary", the title starts with a short type that fits, such as "Game Session:", not with a business type.
- AE3. **Covers R11.** Given a short conversation with no clear type, when it is summarized, the title has only the topics and no type prefix.
- AE4. **Covers R1, R6.** Given a custom prompt whose title text says "Start with the customer's company name, then the main request", when a customer meeting is summarized, the title follows that pattern and has no meeting-type prefix.
- AE5. **Covers R4, R9.** Given a custom prompt with an empty title text, when a note is summarized, the title is one short line of plain text with no required pattern.
- AE6. **Covers R8, R10.** Given the 1:1 of AE1, summarized with a context limit small enough to split it into parts, the title still starts with "1:1 Meeting:".
- AE7. **Covers R14.** Given an endpoint that fails the title turn, when a note is summarized, the summary is saved and the note keeps the title it had.
- AE8. **Covers R13.** Given a note whose title the user set, when the user presses Regenerate Summary, the summary changes and the title does not.
- AE9. **Covers R7, R9.** Given a custom title text that asks for "a Markdown heading with the title in bold", the title is plain text, and `summary.md` has no title line.
- AE10. **Covers R16.** Given an endpoint that answers the title turn slowly, when a note is summarized, the note shows its summary before the title turn ends; when the title turn ends, the note's title and folder name change.

### Scope Boundaries

- A summary prompt chosen by note type (`docs/ideas/someday/summary-prompt-by-note-type.md`). The meeting type in a title is text only. It does not choose a prompt and is not stored as a field.
- Titles of existing notes. They change only through Regenerate Summary, under R13.
- Placeholders or variables in the title text, such as the date or the folder name.
- Structured (JSON-schema) output from the endpoint.
- Prompt caching controls. The title turn sends the same message prefix as the final call, so an endpoint that caches prefixes can reuse it, but the app sends no cache instructions.

### Dependencies / Assumptions

- Nobody uses Longhand yet, so the new title field of the stored prompts needs no data migration. A stored prompt with no title text reads as an empty title text.
- The title text is part of each summary prompt, so it is in the shared settings with the rest of the prompt.

### Sources / Research

- `crates/summarize/src/prompts.rs`: `TITLE_SHAPE` (the fixed title instruction today), `title()` (the separate title call), `final_system()`, `whole()`, `merge()`, `PART_SHAPE`, `overhead()`.
- `crates/pipeline/src/summarize.rs`: `Calls::summarize`, `call_as` (stores the raw reply under its key), `propose_title` (skips a user-set title and a Transcribe Again run; the failure path), `clean_title`, the `TITLE` checkpoint.
- `crates/session-save/src/naming.rs`: `MAX_TITLE_CHARS` (120), the title length cap.
- `src/settings/SummaryPrompts.tsx`: the summary prompt editor, with the read-only built-ins and Duplicate.
- `docs/plans/2026-09-25-1145-feat-summary-prompts-plan.md`: R2–R4 (built-ins, new prompts and duplicates), R7 (fixed rules), KTD1 and KTD8 (fixed and editable parts, token budget).

---

## Planning Contract

### Key Technical Decisions

- KTD1. **The title turn extends the final call's messages.** The final call (`whole`, or `merge` with `last`) keeps its messages exactly as today. The title turn sends those messages, then the summary reply as an assistant message, then one user message: the title request with the title text and the fixed title rules. The fixed rules stay at the end, after the title text, as the naming rule follows the summary text today. The title request starts by saying that the summary is done and that the earlier instructions about the summary, such as "Do not add a title", do not apply to this reply. The prefix is byte-identical to the final call, so an endpoint that caches prefixes can reuse it. Governs R6, R9. (session-settled: user-directed — chosen over one reply with a title line and a separator: see the second Key Decision.)
- KTD2. **The separate summary-only title call is removed.** No fallback call replaces it (R14): a failed title turn leaves the title as it is, as a failed title call does today. The title request keeps the phrase "Propose a title for this session". The pipeline test helper `is_title_call` (`crates/pipeline/tests/runner.rs`) reads the second message of a request, which in the title turn is the transcript; it must look for the phrase in the last message.
- KTD3. **The Summarize stage stores the title request; it does not send it.** `Calls::summarize` returns the final call's messages with the reply. Where `propose_title` runs today, the stage builds the whole title-turn request (KTD1) with its reply cap (KTD12) and stores it as a pending title request of the session, then writes `summary.md` and completes as today. It stores nothing when no title turn is due: a user title, a Transcribe Again run, or no speech. On a resumed Summarize stage, the final reply comes from its checkpoint (`call_as` returns the stored raw reply) and the messages are built again from the same inputs, so the stored request is the same (R15). The pending request must survive Save and the removal of the working folder; it is a row in the store, not a file.
- KTD13. **A title job on the saved note sends the stored request.** The note page shows the processing status, not the summary, while a session is processing (`src/session/SessionPage.tsx`), so the title turn cannot be a stage. After Save, the session is saved as today, and the runner queues a title job for it, as `regenerate_summary` runs on a saved session: it takes a claim on the note and the processing permit, sends the request, cleans the reply, proposes the title with `set_title(Proposed)` (which never replaces a user title, R13), deletes the pending request, releases the claim, and runs the folder reconcile so the folder takes the new name. A failure logs, deletes the pending request and keeps the title (R14). The runner then sends an event so the Home list and the note page show the new title. At launch, the runner queues a title job for each saved session that still has a pending request (R15). Regenerate Summary publishes `summary.md`, releases its claim, and then queues the same title job (R16). A delete or a move of the note cancels a pending title job and deletes its request.
- KTD4. **The token budget reserves room for the title turn.** The title turn costs the final call's input, plus the summary (at most the final call's reply reserve), plus the title request (fixed rules and a title text up to its limit), plus a minimum title reply. A new `prompts::title_reserve` gives the cost of the title request at the title-text limit plus that minimum reply. Subtract it in the whole-call fit check of `Calls::summarize` (which does not use `overhead` today), in `chunk_allowance` and in `fit_notes`, so every call that can be the final call leaves room for its title turn. `prompts::overhead` stays the final-call cost: the "prompt too long" half-check (`overhead("", text) * 2 > allowance`) must not include the title reserve, or both built-ins fail at 2048 tokens.
- KTD12. **The title turn's reply cap is what the context has left.** After the summary is known, the cap is `min(room.reserve, room.limit − with_margin(cost of the title-turn messages))`. A small fixed cap would let a reasoning model spend it all on thinking and be cut off; the full reserve could exceed the context. When the cap is below the minimum title reply, the title turn is not sent, and the run logs it as a title failure (R14).
- KTD5. **The title text has a limit of 1,000 characters.** A title text is a few sentences; 1,000 keeps the reserve of KTD4 small. The summary text keeps its 4,000.
- KTD6. **`Prompt` gets a `title_text` field with a serde default.** `Prompt` is stored as JSON in the pipeline `PROMPT` checkpoint, in `shared-settings.json` and in the old local store, and none of its fields has a default today. A default of an empty string reads every stored prompt. The mirrors get the same field with the same serde default: `SummaryPrompt` and `SummaryPromptEntry` in `src-tauri/src/managers/settings.rs`, and `PromptSnapshot` in `crates/session-core/src/jobs.rs`. Without the default on `SummaryPrompt`, `legacy_prompts` drops every stored custom prompt, because it skips entries that do not parse.
- KTD7. **The prompt snapshot gets a new column in a new migration.** `PromptSnapshot` is stored in the `prompt_snapshot_*` columns of SQLite (migration `0013_prompt_snapshot.sql`). Add `0015_prompt_snapshot_title.sql` with a `prompt_snapshot_title_text` column, `NOT NULL DEFAULT ''`, and register it in the migration list in `crates/session-core/src/schema.rs`. Never edit migration 0013 (`docs/solutions/workflow-issues/edited-applied-migration-breaks-older-databases.md`).
- KTD8. **A prompt file keeps the title text in its body, after a fixed marker line.** The file stays one Markdown file: front matter (`id`, `name`), the summary text, then the line `<!-- longhand: title instructions -->`, then the title text. A file with no marker has an empty title text. The renderer writes the marker only when the title text is not empty. An HTML comment is invisible when the file is rendered and cannot be mistaken for a heading in a summary text. A summary text that contains the marker line is refused, so a file always parses back to the texts that were saved. `MAX_FILE_BYTES` grows by 4 bytes for each character of the title limit plus the marker line, as it already allows 4 bytes for each character of the summary text.
- KTD9. **Two prompts are the same only when their title texts are also the same.** `resolve` in `crates/shared-settings/src/prompts.rs` groups files with one id by text to find conflict copies, and `merge_prompts` in `crates/shared-settings/src/join.rs` matches prompts by id or name and text. Both compare the pair (summary text, title text). The Regenerate reuse check in `crates/pipeline/src/save.rs` (`same_prompt`) stays on the summary text: the title turn is not a cached layer, and Regenerate deletes `TITLE` and any pending title request in every case.
- KTD10. **The title text has its own command and its own editor state.** A new command `set_summary_prompt_title_text(id, text)` mirrors `set_summary_prompt_text`. In `SummaryPrompts.tsx`, the draft, base, save error and "changed on another computer" state are kept per prompt and per field, so an edit or a conflict in one field never blocks the other. Rules for shared files apply: patch only the field that changed, keep unknown fields (`docs/solutions/integration-issues/shared-settings-in-a-synced-folder-rules.md`).
- KTD11. **The meeting-type clue rule goes into `PART_SHAPE`.** `PART_SHAPE` is in every part note and every intermediate merge, so one sentence covers R8.

### High-Level Technical Design

The final summary conversation, before and after:

```mermaid
sequenceDiagram
  participant S as Summarize stage
  participant V as Save stage
  participant T as Title job
  participant E as Endpoint
  S->>E: system (fixed rules, summary text, naming rule) + user (transcript or part notes, notes)
  E-->>S: summary reply
  S->>S: store pending title request (same messages + assistant summary + title request), write summary.md
  S->>V: stage complete
  V->>V: publish note folder; session saved, summary readable (R16)
  V->>T: queue title job
  T->>E: stored title request (title text, fixed title rules last)
  E-->>T: title reply
  T->>T: clean_title, set_title(Proposed) (R13), delete request, folder reconcile, title event
```

Today the title is a new call with only the summary and the notes, made inside the Summarize stage before `summary.md` is written.

### Assumptions

- Every OpenAI-compatible endpoint that Longhand supports accepts an assistant message in the request. The chat completions format requires it.

### Risks

- **A slow endpoint with no prefix cache processes the input again.** On a local model with a long transcript, the title can take tens of seconds or minutes. The title job runs after the note is saved (KTD13), so only the title and the folder name wait. The title job also takes its turn with the processing of other notes.
- **The folder is renamed after the user may have opened it.** A title job that ends while the user has the note folder open in Finder renames it under them. The existing reconcile already does this for a user rename; accepted.
- **Another branch adds a migration.** Plans in progress on `main` can also add migration 0015. Renumber at merge as `docs/solutions/workflow-issues/parallel-branches-bump-same-schema-version.md` describes.

---

## Implementation Units

### U1. Title text in the prompt type and the prompts

- **Goal:** `Prompt` carries a title text, the built-ins have the built-in title text, and the prompt module builds the title turn and reserves its budget.
- **Requirements:** R2, R5, R6, R8, R9, R10, R11, R12; KTD1, KTD2, KTD4, KTD5, KTD6, KTD11.
- **Dependencies:** None.
- **Files:** `crates/summarize/src/prompts.rs`, `crates/summarize/tests/prompts.rs`, `crates/summarize/tests/merge.rs`.
- **Approach:**
  1. Add `title_text` to `Prompt` with a serde default, and a `TITLE_TEXT_LIMIT` of 1,000.
  2. Write the built-in title text (R10–R12) and give it to both built-ins.
  3. Replace `TITLE_SHAPE` with the fixed title rules of R9, without the word limit. Include the naming rule's intent for titles.
  4. Replace `title()` with a title-turn builder that takes the final call's messages, the summary reply and the title text, and keeps "Propose a title for this session" in the request.
  5. Add the meeting-type clue sentence to `PART_SHAPE` (KTD11).
  6. Add `title_reserve` (KTD4). Leave `overhead` as the final-call cost.
- **Patterns to follow:** `final_system` places the fixed `NAMING_RULE` after the editable text; `PROMPT_TEXT_LIMIT` and its overhead tests.
- **Test scenarios:**
  - The title turn's messages start with the final call's messages unchanged, then an assistant message with the summary, then a user message that contains the title text and "Propose a title for this session".
  - The fixed title rules come after the title text in the title request.
  - The title request starts by saying that the earlier summary instructions, such as "Do not add a title", do not apply to this reply.
  - An empty title text gives a title request with only the fixed rules.
  - A title text that says "use a Markdown heading" still gets the rule "plain text, no Markdown" after it. Covers AE9.
  - Both built-ins have the same title text, which names "1:1 Meeting" and asks for no type prefix when the type is not clear.
  - The part note and the intermediate merge messages contain the meeting-type clue rule; the whole call and the last merge messages do not need it.
  - `title_reserve` grows with the title text and covers a title text at its limit.
  - A summary text and a title text at their limits still fit at 8192 tokens; each built-in still passes the "prompt too long" half-check at 2048.
  - A `Prompt` JSON without `title_text` parses with an empty title text.
- **Verification:** The summarize crate tests pass, and the overhead tests cover both limits.

### U2. Title text in the shared prompt files

- **Goal:** Prompt files store, read, merge and compare the title text.
- **Requirements:** R1, R3, R4, R5; KTD8, KTD9.
- **Dependencies:** U1.
- **Files:** `crates/shared-settings/src/prompts.rs`, `crates/shared-settings/src/join.rs`, `crates/shared-settings/src/cache.rs`, `crates/shared-settings/src/view.rs`, `crates/shared-settings/tests/prompts.rs`, `crates/shared-settings/tests/join.rs`, `crates/shared-settings/tests/two_computers.rs`.
- **Approach:**
  1. Parse the body into summary text and title text at the marker line; render the marker only for a title text that is not empty (KTD8).
  2. Add `title_text` to `PromptFile` and to the `From<&PromptFile>` conversion.
  3. Check the title length in a title check next to `check_text`; an empty title text is valid. Make `check_text` refuse a summary text that contains the marker line, and grow `MAX_FILE_BYTES` (KTD8).
  4. Add a write for the title text next to `set_prompt_text`, and make `create_prompt`, `rename_prompt`, `rewrite` and `write_new_prompt` carry the title text.
  5. Compare (text, title text) in `resolve` and in `merge_prompts` (KTD9).
- **Patterns to follow:** `set_prompt_text` and `rewrite`; the shared-settings rules in `docs/solutions/integration-issues/shared-settings-in-a-synced-folder-rules.md`.
- **Test scenarios:**
  - A file with summary text, marker and title text reads both texts; rendering it again gives the same bytes.
  - A file with no marker reads an empty title text.
  - A title text with Markdown headings and blank lines survives a round trip.
  - Setting the title text changes only the part after the marker and keeps unknown front matter lines.
  - A title text over 1,000 characters is refused; an empty one is accepted.
  - A summary text that contains the marker line is refused.
  - A title text of 1,000 four-byte characters, with a summary text at its limit, stays within `MAX_FILE_BYTES` and round-trips.
  - Two files with one id, the same summary text and different title texts are treated as a conflict copy.
  - The merge keeps two prompts with the same name and summary text but different title texts.
  - Two computers: a title text edit on one computer reaches the other through the folder.
- **Verification:** The shared-settings tests pass.

### U3. Title text in the snapshot and the app settings

- **Goal:** The app settings and the prompt snapshot of a recording carry the title text, and new prompts and duplicates get it.
- **Requirements:** R2, R3, R4; KTD6, KTD7, KTD10.
- **Dependencies:** U1, U2.
- **Files:** `crates/session-core/migrations/0015_prompt_snapshot_title.sql`, `crates/session-core/src/schema.rs`, `crates/session-core/src/jobs.rs`, `crates/session-core/tests/jobs.rs`, `src-tauri/src/managers/settings.rs`, `src-tauri/src/managers/shared.rs`, `src-tauri/src/managers/capture.rs`, `src-tauri/src/commands/settings.rs`, `src-tauri/src/lib.rs`, `src/bindings.ts`.
- **Approach:**
  1. Add the migration, register it in `schema.rs`, and add the field to `PromptSnapshot`, its insert and its read.
  2. Add `title_text` with the serde default of KTD6 to `SummaryPrompt`, `SummaryPromptEntry` and `PromptSnapshot`, and the title limit to `SummaryPromptList`.
  3. `create_prompt` uses the Topic summary title text; `duplicate_prompt` copies the source title text (R3).
  4. Add the `set_summary_prompt_title_text` command and register it; regenerate `src/bindings.ts`.
  5. Fill `title_text` in the capture snapshot and in `active_summary_prompt_snapshot`.
- **Patterns to follow:** `set_summary_prompt_text` and its validation tests in `src-tauri/src/managers/shared.rs`; migration 0013.
- **Test scenarios:**
  - A job saved with a snapshot that has a title text reads it back; a job row from before the migration reads an empty title text.
  - A new prompt has the Topic summary title text; a duplicate of a prompt with a custom title text has that title text.
  - Setting the title text of a built-in prompt is refused.
  - A custom prompt in the old local store, stored without a title text, is read with an empty title text and not dropped.
  - Setting a title text over the limit is refused, and an empty one is saved.
  - The prompt list reports the title limit.
- **Verification:** The session-core and app tests pass, and the bindings file matches the commands.

### U4. The title request in the Summarize stage

- **Goal:** The Summarize stage stores the title request instead of sending a title call, and the budget leaves room for it.
- **Requirements:** R5, R6, R7, R13, R15; KTD1, KTD2, KTD3, KTD4, KTD12.
- **Dependencies:** U1, U3.
- **Files:** `crates/pipeline/src/summarize.rs`, `crates/pipeline/src/runner.rs`, `crates/pipeline/tests/runner.rs`.
- **Approach:**
  1. Make `Calls::summarize` return the final call's messages with the reply (KTD3).
  2. Replace `propose_title` with a step that keeps its early returns (a user title, a Transcribe Again run, no speech) and stores the title-turn request with its reply cap (KTD12) as a `title-request` checkpoint of the Summarize stage. Check that no Save path clears it before the title job runs (U6).
  3. Subtract `title_reserve` in the whole-call fit check, `chunk_allowance` and `fit_notes` (KTD4).
  4. Carry `title_text` in `first_summary_prompt` and in the `PROMPT` checkpoint.
  5. Update `is_title_call` and the helpers that use it to read the last message (KTD2).
- **Patterns to follow:** `propose_title` and `call_as` today.
- **Test scenarios:**
  - A whole-transcript run stores a title request whose messages start with the final call's messages, then the summary reply, then the title request with the prompt's title text.
  - A long run stores a request built on the last merge's messages.
  - The Summarize stage sends no title call, and `summary.md` holds the final reply.
  - A note with a user title, a Transcribe Again run and a session with no speech store no request.
  - A Summarize stage stopped after the final call and resumed sends no final call again and stores the same request.
  - A transcript that fits one whole call only without the title reserve now splits into parts, and its stored request fits the context.
  - A short summary gets a reply cap up to the reserve; a summary near the reserve gets a smaller cap that still fits; a cap below the minimum stores no request, and the summary completes.
- **Verification:** The pipeline tests pass, including the existing prompt tests.

### U6. The title job on the saved note

- **Goal:** After the note is saved, a title job sends the stored request, proposes the title and renames the folder, without delaying the summary.
- **Requirements:** R13, R14, R15, R16; KTD13.
- **Dependencies:** U4.
- **Files:** `crates/pipeline/src/runner.rs`, `crates/pipeline/src/save.rs`, `crates/pipeline/src/summarize.rs`, `crates/pipeline/src/folder_rename.rs`, `crates/pipeline/tests/runner.rs`, `src-tauri/src/managers/processing.rs`, and the frontend code that maps runner events to the Home list and the note page.
- **Approach:**
  1. Add a title job to the runner, modelled on `regenerate_summary`: a claim on the note, the processing permit, `Unattended` control.
  2. Queue it when the Save stage completes, when Regenerate Summary has published `summary.md` and released its claim, and at launch for each saved session with a pending request.
  3. Send the request, clean the reply with `clean_title`, call `set_title(Proposed)`, delete the request, release the claim, then run the folder reconcile (`folder_rename::reconcile`) so the folder takes the name.
  4. On a failure, a cut-off or an empty reply: log, delete the request, keep the title (R14).
  5. Send a runner event when a title is proposed, and refresh the note's title on Home and on the note page from it.
  6. Cancel a pending title job and delete its request when the note is deleted, moved to another storage folder, or regenerated.
- **Patterns to follow:** `Runner::regenerate_summary` and its claim; `folder_rename::rename_locked` for the reconcile after a title change; the launch reconcile `reconcile_saved`.
- **Test scenarios:**
  - After Save, the session is saved and its `summary.md` is in the note folder before the title request is sent. Covers AE10.
  - A title job proposes the cleaned title reply and renames the folder to the new name. Covers AE4.
  - A title reply such as "# **Title:** Weekly sync." gives "Weekly sync". Covers AE9.
  - A failed title turn keeps the old title and folder name and deletes the request. Covers AE7.
  - An empty title reply proposes nothing.
  - A user rename while the title job runs keeps the user's title. Covers AE8.
  - A note with a user title gets no title job on Regenerate. Covers AE8.
  - A restart with a pending request runs the title job once at launch; a restart after the job ended runs nothing.
  - Regenerate Summary publishes the new summary and releases its claim before its title turn is sent.
  - Deleting the note while a title job waits for the permit cancels the job and leaves no request.
  - The title event reaches the app, and the Home list shows the new title.
- **Verification:** The pipeline tests pass, and in a signed build a note shows its summary before its title changes.

### U5. The title field in the prompt editor

- **Goal:** Settings shows and saves "Title instructions" for each prompt.
- **Requirements:** R1, R2, R4, R5; KTD10.
- **Dependencies:** U3.
- **Files:** `src/settings/SummaryPrompts.tsx`, `src/settings/SummaryPrompts.test.tsx`.
- **Approach:**
  1. Invoke `apple-design` before the layout work.
  2. Add a second text field, labelled "Title instructions", below the summary text, with its own character count against the title limit.
  3. Key the draft, base, save error and "changed on another computer" state by prompt and field (KTD10).
  4. Show the built-in title text read-only, as the built-in summary text is.
  5. Allow an empty title text; keep the "A prompt can't be empty." rule for the summary text only.
- **Patterns to follow:** The summary text field, its autosave and its conflict notice in the same file.
- **Test scenarios:**
  - A built-in prompt shows its title text read-only.
  - Typing in the title field saves the title text with the title command and does not save the summary text.
  - An empty title field saves and shows no error.
  - A title text over the limit shows the count in the error state and does not save.
  - A title text changed on another computer shows the notice for the title field only; an edit in the summary field still saves.
  - A save error in the title field names that field.
- **Verification:** `bun run typecheck` and `bun test ./src/` pass, and a manual look at the editor in a signed build.

---

## Verification Contract

| Gate | Command or check | When |
|---|---|---|
| Format and lint | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` | Each unit |
| Rust tests | New and related tests of the changed crates | Each unit |
| Bindings | `cargo test -p longhand export_bindings`, then no diff in `src/bindings.ts` | U3 |
| Rust tests, broad | `cargo test --workspace --features nemo-runtime/test-helper` | Once, before the push for the PR |
| Frontend | `bun run typecheck`, `bun test ./src/` | U5, and before the PR |
| Repo checks | `scripts/check-windows.sh`, `scripts/check-licences.sh`, `scripts/check-provenance.sh` | Before the PR |
| Manual test | Signed bundle with the user's endpoint (see `docs/development-workflow.md#manual-tests`): a 1:1 note (AE1), a game session (AE2), a short note with no clear type (AE3), a custom title text (AE4), the summary shown before the title changes (AE10), and the editor fields | After U5 |

---

## Definition of Done

- Every unit's verification holds and all gates in the Verification Contract pass.
- No code calls the removed summary-only title call, and the fixed title rules have no word limit.
- AE4–AE10 each have an automated test; AE1–AE3 and AE10 passed in the manual test.
- Code from abandoned approaches is removed from the diff.
