---
title: Background for Summaries - Plan
type: feat
date: 2026-10-05
topic: summary-background
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Background for Summaries - Plan

## Goal Capsule

- **Objective:** Summaries and titles spell the user's names and terms correctly and name the people the user told Longhand about. The user writes this background one time, in free-form Markdown, and every summary uses it.
- **Means:** A new "Background" text on the Summaries page of Settings, above the summary prompts. Summarize adds it to the system prompt of each model call. A second unit gives the summary prompt and title instruction boxes the same Markdown editor.
- **Product authority:** The user, in the brainstorm of 2026-10-05. Source idea: `docs/ideas/next/summary-background.md`. This is a smaller step before `docs/ideas/someday/agentic-summary.md`; Background is the text that idea calls "context about the user".
- **Open blockers:** None.

---

## Product Contract

### Summary

Settings gets a Background section on the Summaries page, between Model and the summary prompts. The user writes free-form Markdown there: their name, their work, the names and terms that often occur, and words that the transcript often gets wrong. Every summary and title request includes this text in a labelled section of the system prompt. The section uses the My Notes editor, and the summary prompt and title instruction boxes change to the same editor.

### Problem Frame

The transcript often spells names and terms wrong, and the summary repeats the errors. The summary model knows nothing about the user, their company or the people they meet. The only place to tell it is the summary prompt text. That text is per prompt, the built-in prompts cannot be changed, and facts about the user do not belong in instructions about the shape of a summary.

The fixed naming rule also stops the model from using a name unless the transcript or notes make it clear. Even when the user knows who was in the room, the summary says "your team member".

### Key Decisions

- **One Background for all prompts, not one for each prompt.** The user writes it one time, and it can later feed the agent's "context about the user" tool. Governs R1, R5.
- **Background is the user's own text, not quoted data.** The model reads it as instructions and facts, like the prompt text. The fixed rules still win over it. Governs R6, R8.
- **Background is evidence for names.** The model can use it to name a person, including to decide which speaker is a person it names, when the data and the Background together make that clear. It never invents a name. Governs R7. (session-settled: user-directed — chosen over "fix spelling only" and over "also identify the user": the rule that a name must come from the data alone is too strict after the user has said who people are.)
- **Background sits above the summary prompts.** Most users will fill it in; fewer users will change the built-in prompts, which are the more advanced setting. Governs R2. (session-settled: user-directed — chosen over a position below the prompts: Settings goes from common to advanced.)
- **The name is "Background".** It is neutral, it fits facts about a company as well as a person, and it does not collide with the existing "Context limit" setting. Governs R2. (session-settled: user-directed — chosen over "About you" and "Summary context".)

### Requirements

**Background text**

- R1. Settings has one Background text. It is free-form Markdown with no required structure.
- R2. The Background section is on the Summaries page, after Model and before the summary prompts. Its hint text tells the user what to write: names, work, terms they use, and words the transcript often gets wrong.
- R3. The Background uses the same Markdown editor as the My Notes tab. In Settings, each editor has its own accessible name and a hint line under its visible label that stays visible while the user types, as the prompt boxes have today. The My Notes placeholder does not show in Settings.
- R4. A change saves automatically, the same as the other Settings texts. There is no Save button.
- R5. The Background is shared across computers through the storage folder, the same as the summary prompts. When the storage folder is not available, the section is read-only and says why, the same as the summary prompts.
- R6. The Background has a size limit, shown as a character count the same as the prompt text. A text over the limit is not saved and the count shows the error.

**Use in summaries**

- R7. Every model call that Summarize makes includes the Background in a labelled section of the system prompt: the call for a whole transcript, the part-note and merge calls for a long transcript, the final merge, and the title turn. An empty Background adds nothing.
- R8. Every fixed rule about names counts the Background as evidence: the naming rule of the final call, the label rule of part notes and merges, the "keep every name exactly as it was said" rule of part notes, and the title rules. A name is used, or a spelling corrected, when the transcript, the notes and the Background together make clear who or what is meant. The rules still forbid "Speaker N" in the summary and title, and still forbid invented names. The rules mention the Background only when the Background is not empty.
- R9. The fixed rules keep their priority over the Background, the same as over the prompt text. A Background cannot relax them.
- R10. A summary uses the Background as it is when the summary starts. A change to the Background does not change existing summaries; Regenerate uses the current Background.

**Markdown editor for prompt texts**

- R11. The summary prompt text and the title instructions text in Settings use the same Markdown editor as the Background.
- R12. A built-in prompt shows its texts in the editor as read-only, as it does today.
- R13. Character limits, the count, the save delay and the save on leaving the box stay as they are today.

### Acceptance Examples

- AE1. **Covers R7.** **Given** a Background that says "Pria is spelled Priya", **when** the transcript says "Pria will send the budget", **then** the summary says "Priya will send the budget".
- AE2. **Covers R8.** **Given** a Background that says "I meet Sam each Monday for a 1:1", **when** a two-person transcript has one speaker addressed as Sam, **then** the summary names Sam, and does not name the other speaker unless the data or the Background makes that clear.
- AE3. **Covers R8.** **Given** a Background that lists Sam and Priya, **when** nothing in the transcript connects a speaker to either name, **then** the summary uses a plain description, not a guessed name.
- AE4. **Covers R7.** **Given** an empty Background, **when** the user summarizes, **then** the system prompt is the same as before this change.
- AE5. **Covers R7.** **Given** a transcript long enough to be split into parts, **when** Summarize runs, **then** each part note already uses the Background spelling.
- AE6. **Covers R12.** **Given** the built-in Topic summary is selected, **when** the user clicks its text, **then** the editor shows the formatted text and takes no input.
- AE7. **Covers R8.** **Given** the AE2 Background and a transcript long enough to be split into parts, **when** Summarize runs, **then** the part notes write "Speaker 2 (Sam)" and the final summary and the title name Sam.

### Scope Boundaries

- The summary agent and its tools (`docs/ideas/someday/agentic-summary.md`) and outside tools such as a calendar (`docs/ideas/someday/pluggable-summary-tools.md`).
- A Background that the model changes by itself, like a memory.
- A different Background for each prompt or each type of note.
- Names on the speaker labels of the transcript. R8 changes only the summary and title text.

### Dependencies / Assumptions

- Background is text that the user types, so it is inside the `STRATEGY.md` boundary "Nothing writes into Longhand". It is sent to the summary endpoint with the transcript, so it goes only where the user already chose to send the transcript.
- The "Ruled out" item "Speaker identity" in `docs/ideas/README.md` is about enrollment and voiceprints. R8 adds no stored speaker identity; it only lets the summary text use a name that the user wrote.

### Outstanding Questions

**Deferred to Planning**

- The size limit for R6. Start near the prompt text limit of 4,000 characters, and check what a local model's context window allows, because every call carries the full Background.
- Where the Background is stored in the storage folder, and its file format.
- How the editor in Settings differs from the My Notes editor, for example a smaller height, and whether the `/` menu and toolbar fit a Settings box.
- The exact wording of the labelled section and of the changed naming rules.
- The editor saves through a Markdown round trip, which joins single line breaks and changes list markers. A Background written one fact per line, or a prompt file written outside the app, changes shape on its first edit. Decide whether the editor must keep single line breaks in these texts.

### Sources / Research

- `crates/summarize/src/prompts.rs`: `final_system()` builds the final system message (data rule, fixed rules, prompt text, then `NAMING_RULE`); `part_system()` builds the part-note and non-final merge messages with no prompt text; `title_turn()` reuses the final call's messages; `PROMPT_TEXT_LIMIT` and `TITLE_TEXT_LIMIT`.
- `crates/shared-settings/src/prompts.rs`: prompts are one `.md` file each in `.longhand/prompts/` in the storage folder, with front matter and a title marker line.
- `docs/solutions/integration-issues/shared-settings-in-a-synced-folder-rules.md`: rules for settings shared through a synced folder.
- `src/settings/SummaryPrompts.tsx`: the two `<textarea>` boxes, the one-second save delay, the save on blur, and the read-only state for built-in prompts.
- `src/settings/EndpointForm.tsx`: the Summaries page order (endpoint, Model, summary prompts, Context limit).
- `src/editor/NoteEditor.tsx`: the My Notes editor (`markdown`, `onChange`, `autoFocus`, `editable`).
- `docs/plans/2026-09-30-1604-feat-title-instructions-plan.md`: the earlier plan for a Settings text that goes into the prompt.
