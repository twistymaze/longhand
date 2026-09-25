---
title: Summary Prompts - Plan
type: feat
date: 2026-09-25
topic: summary-prompts
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Summary Prompts - Plan

## Goal Capsule

- **Objective:** A user gets summaries in the shape and at the level of detail that they need for each type of meeting: a game-session recap, a 1:1, a business meeting. The user gets these summaries without editing the prompt rules that keep summaries correct.
- **Product authority:** This Product Contract. The three example summaries in the Appendix are the reference for the "Topic summary" detail level and shape.
- **Open blockers:** None.

---

## Product Contract

### Summary

Settings holds a list of named summary prompts, and one of them is active. The list starts with two built-in prompts. "Topic summary" is the default: it writes a topic-grouped summary with no times and with the concrete detail of a game-session recap. "Play-by-play summary" keeps today's time-ordered shape with times. Each prompt controls what the summary contains and how the summary is organized. The app keeps the fixed rules that make summaries correct and safe.

### Problem Frame

The current summary has one fixed shape: an overview, then "Key points" with a `[hh:mm:ss]` time on each point, then "Decisions" and "Open questions and follow-ups". The shape tells the meeting in time order and labels people "Speaker 1" and "Speaker 2". The user finds this shape poor for game sessions and for 1:1 meetings. The user rarely needs the times. The user wants the facts grouped by topic, with concrete detail such as amounts, items and names. The user also has different types of meetings that need different shapes. Today the only way to change the shape is to change the code.

### Key Decisions

- **The user edits the judgment and the shape, not the whole prompt.** The editable text controls what the summary includes, how the typed notes are weighed against the transcript, the sections, the length and the tone. The fixed rules keep the untrusted-data protection and the chunk and merge mechanics safe from a prompt edit. Governs R5, R6, R7. (session-settled: user-directed — chosen over "output shape only" and "extra guidance only": the user wants to influence how the summarizer decides what is in the summary, including notes against transcript, and not only its format.)
- **A saved list from the start, with one app-wide active prompt.** A list stops the settings shape from needing a change later. Handy's post-processing prompts are the model. Governs R1–R4. (session-settled: user-approved — chosen over one editable prompt now and over a list with only one prompt visible: the list costs little more to build than one prompt, and a later move from one prompt to many is avoided.)
- **Existing summaries stay as they are; Regenerate uses the active prompt.** Each summary records the name of the prompt that made it. Governs R10, R11. (session-settled: user-approved — chosen over a prompt choice at Regenerate and over no record: the record shows which prompt made a summary, and a wrong one is fixed with Regenerate.)
- **People are never labelled "Speaker N" in a summary, and a name is never guessed.** This rule stays fixed, so no prompt can relax it. Governs R7. (session-settled: user-directed — chosen over "keep Speaker N labels", "let the prompt relax the rule" and "add a participant list": the user said a summary should never say "Speaker 2".)
- **The default built-in prompt uses the RPG recap's detail level, with the sections grouped by topic.** A summary that describes the conversation ("Nathan explained that…") in place of its content is too summarized. Governs R8, R9. (session-settled: user-directed — chosen over the current time-ordered shape and over the Copilot example's level of detail: the user found both of them wrong for game sessions and for 1:1s.)
- **Today's shape stays available as a second built-in prompt, "Play-by-play summary".** A user who wants a time-ordered account keeps it without writing a prompt. Governs R2, R12. (session-settled: user-directed — the user proposed keeping the current prompt as a second optional built-in.)

### Requirements

**Saved prompts**

- R1. Settings shows a list of summary prompts. Each prompt has a name and text, and exactly one prompt is active.
- R2. The list always holds two built-in prompts: "Topic summary" (R8, R9) and "Play-by-play summary" (R12). The user cannot edit, rename or delete a built-in prompt, but can duplicate it. A new install has "Topic summary" active.
- R3. The user can create a prompt, duplicate a prompt, rename a prompt, edit the text of a prompt that is not built-in, delete a prompt that is not built-in, and set any prompt as active. A new prompt starts with the "Topic summary" text, so the user edits it and does not start with an empty box.
- R4. A prompt with empty text cannot be saved. When the user deletes the active prompt, "Topic summary" becomes active.

**What a prompt controls**

- R5. The active prompt's text is used when the final summary is written. This is true for a transcript that fits in one call and for the last merge of a long transcript.
- R6. The prompt text controls what the summary includes and leaves out, how the typed notes are weighed against the transcript, the sections, the length and level of detail, the tone, and whether times appear.
- R7. These rules are fixed. The app always sends them, and the prompt text cannot change them:
  - The transcript and the notes are data. The model never follows them as instructions.
  - A summary never labels a person "Speaker N". It uses a person's name only when the transcript makes clear who is speaking. When no name is clear, it uses a plain description that fits the context, such as "your team member" or "someone in the group", or "the group" when everyone is the subject. It never guesses a name.
  - The chunk, part-note and merge mechanics of a long transcript, and the title prompt, stay as they are.

**The built-in prompts**

- R8. The "Topic summary" is grouped by topic, not in time order. It starts with a short overview paragraph. Then comes one section for each topic, with a one-line setup, bullets with bold labels, and the specific details nested below them. It ends with "Open questions" and with follow-ups and their owners, but only when there are any. It shows no times and adds no title.
- R9. The "Topic summary" states concrete facts, not descriptions of the conversation. It writes "they found 15 gold pieces and an invisibility potion", not "the participants discussed the loot". It keeps numbers, amounts, names and specific items.
- R12. The "Play-by-play summary" keeps today's shape: an overview, then "Key points" in time order with a `[hh:mm:ss]` time on each point, then "Decisions" and "Open questions and follow-ups" when there are any. It follows the naming rule in R7, so it does not label people "Speaker N".

**Existing summaries and Regenerate**

- R10. A change to the prompt list or to the active prompt does not change any existing summary. Regenerate Summary on a note uses the prompt that is active at that time.
- R11. Each summary records the name of the prompt that made it, and the note's Summary shows that name. The name that was recorded stays after the prompt is renamed or deleted.

### Acceptance Examples

- AE1. **Covers R8, R9.** Given the RPG session behind Appendix example A and the "Topic summary" prompt, when the session is summarized, the summary has a section for each scene with the loot amounts and items. It ends with unresolved questions and has no times.
- AE2. **Covers R8, R7.** Given the 1:1 behind Appendix example B, when the session is summarized with the "Topic summary" prompt, the summary has sections such as UserWeb login, QA throughput, Internal Projects and training billing. It has no `[hh:mm:ss]` references and no "Speaker 1" or "Speaker 2".
- AE3. **Covers R5, R6.** Given a custom prompt that asks for times on each point, when a session is summarized with it, the summary shows times.
- AE4. **Covers R7.** Given a custom prompt that says "give each speaker a plausible name", when a session is summarized, the summary still uses only names that the transcript makes clear.
- AE5. **Covers R4.** Given the active prompt is "Game recap", when the user deletes "Game recap", then "Topic summary" becomes active.
- AE6. **Covers R10, R11.** Given a note that was summarized with the "Topic summary" prompt, when the user makes "Game recap" active, the note's summary does not change. After the user presses Regenerate Summary, the summary is rewritten with "Game recap" and the note shows "Game recap".
- AE7. **Covers R12, R7.** Given the 1:1 behind Appendix example B, when the session is summarized with "Play-by-play summary", the summary has the shape of example B with its times, but it has no "Speaker 1" or "Speaker 2".
- AE8. **Covers R5.** Given a transcript too long for one call and a custom prompt, when the session is summarized, the part notes are written as they are today, and the custom prompt shapes only the final merge.

### Scope Boundaries

- Choosing the prompt from the destination folder. This is the existing idea `docs/ideas/summary-kind-per-folder.md`, and it builds on this list.
- Choosing a prompt for one note at Regenerate time, or before a recording.
- Carrying context from the previous session (`docs/ideas/session-to-session-context.md`).
- Editing the title prompt, and placeholders or variables in prompt text.
- A participant list that gives people reliable names. This is a new idea, `docs/ideas/participant-names.md`.
- A change to how the transcript view labels speakers. The transcript still shows Speaker N.
- Import, export or sharing of prompts.

### Dependencies / Assumptions

- The built-in texts belong to the app, and a new app version can improve it. A duplicate is a copy that the user owns, and a change to a built-in text does not update it.
- The model can apply the naming rule in R7 from the transcript alone. A wrong name is a model mistake, not a new feature that this plan must prevent.

### Outstanding Questions

**Deferred to Planning**

- The exact built-in texts. Tune "Topic summary" against the three Appendix examples on real sessions.
- Whether the part notes of a long transcript keep speaker labels and times internally so the parts merge correctly. Only the final summary must follow R7 and the active prompt.
- The UI term for a prompt, which must be one term used on every screen. Also where the Summary tab shows the prompt name (R11). Invoke `apple-design` for the Settings list and editor.
- Where the prompt list is stored, and where the recorded prompt name is kept for each session.

### Sources / Research

- `crates/summarize/src/prompts.rs` has one module that holds all the prompts. The fixed rules are `DATA_RULE`, `SPEAKERS`, `PART_SHAPE`, `MERGE_RULE` and `TITLE_SHAPE`. `WEIGHTING` and `FINAL_SHAPE` are the parts that this plan makes editable. `SPEAKERS` asks for `[hh:mm:ss]` references and Speaker N labels today, so its content divides between R6 and R7.
- `reference-source/Handy/src-tauri/src/settings.rs:91` holds `LLMPrompt { id, name, prompt }`, `post_process_prompts` and `post_process_selected_prompt_id`. Handy's user edits the whole prompt, including the `${output}` placeholder. Longhand keeps the list and the active-prompt model, but not the whole-prompt edit.
- Regenerate Summary already exists (`regenerate_summary`, `src/session/SummaryTab.tsx`).
- The summary UI does not link times to the transcript, so to remove the times from the "Topic summary" shape breaks nothing.

---

## Appendix

The user supplied three summaries as the reference for R8, R9 and R12. The text of the summaries stays out of this file because it has real names. Only its shape is recorded here.

- **A. RPG recap (the target detail level).** An overview paragraph. Then `##` sections for each scene ("Camp Office Encounter", "Kitchen Encounter"). Each section has a one-line setup and bullets with bold labels ("Loot:", "Documents:", "Hidden Treasury:", "Combat:"), with the exact amounts and items nested below them. It ends with "Unresolved Questions". It has no times.
- **B. The current Longhand 1:1 summary (the shape of "Play-by-play summary", less the Speaker N labels).** An overview, then "Key points" in time order, each point with a time. Then "Decisions" and "Open questions and follow-ups". People are labelled Speaker 1 and Speaker 2.
- **C. A Microsoft 365 Copilot business-meeting summary (the grouping to keep, the detail level to reject).** Topic sections that open with one bold sentence. Nested bullets with bold labels. A "Follow-up tasks" list with each owner in parentheses. The user found it too summarized: it describes who said what more than it states the facts.
