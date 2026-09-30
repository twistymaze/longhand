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
- **Means:** Each summary prompt gets a second, editable text, "Title instructions". The call that writes the final summary also writes the title, in a fixed plain reply format.
- **Product authority:** The Product Contract wins on behavior. Source idea: `docs/ideas/next/title-instructions.md`. Builds on `docs/plans/2026-09-25-1145-feat-summary-prompts-plan.md`.
- **Open blockers:** None.

---

## Product Contract

### Summary

Each summary prompt gets a "Title instructions" text beside its summary text. The call that writes the final summary also writes the title, so the title comes from the same input as the summary: the whole transcript and the notes, or the part notes of a long transcript. The built-in prompts' title instructions start the title with the apparent meeting type.

### Problem Frame

Today a separate call proposes the title after the summary exists. That call sees only the final summary and the notes, and its instructions are fixed in the app ("at most eight words, plain text"). A summary prompt cannot change the title. The summary prompts plan listed "editing the title prompt" as out of scope.

The result is a title such as "Topics discussed" that does not say what kind of meeting it was. The meeting type is often clear from the transcript (two people, one reports to the other) or from the typed notes, but a topic summary states facts and seldom says "this was a 1:1". So the title call often does not have the evidence it needs.

### Key Decisions

- **Title instructions are a separate text of each summary prompt.** They cannot conflict with the summary text's "Do not add a title", and a user sees at once which text controls the title. Governs R1, R2, R3, R4. (session-settled: user-directed — chosen over one text with a title section inside, and over a summary text that also controls the title: two clear fields are easier to edit and to understand.)
- **The final summary call writes the title and the summary together.** The title then sees the same input as the summary, not only the summary. Governs R6, R7, R8. (session-settled: user-directed — chosen over a separate title call that also gets the speaker count and the opening of the transcript: the user was concerned that the summary and a part of the transcript are not enough to know the meeting type.)
- **A fixed plain reply format, not a JSON schema.** Longhand works with any OpenAI-compatible endpoint that the user configures, and support for structured output is different on each one. A plain format works on all of them. Governs R6. (session-settled: user-directed — chosen over JSON-schema structured output: it would still need a plain fallback.)
- **The app keeps only the mechanical title rules.** The word limit and the pattern move into the editable text, so a custom prompt can change them completely. Governs R5, R9.
- **An empty title text means "use only the mechanical rules".** New prompts and duplicates copy the title text from their source, so an empty field is always the user's choice. Governs R3, R4.

### Requirements

**Title instructions in a summary prompt**

- R1. Each summary prompt has a name, a summary text and a title text. Settings shows the title text in its own field, labelled "Title instructions", in the summary prompt editor.
- R2. The two built-in prompts have the same built-in title text (R10). The user cannot edit it, as with the built-in summary text.
- R3. A new prompt starts with the "Topic summary" title text. A duplicate copies the title text of its source.
- R4. The title text can be empty. A prompt with an empty title text can be saved. The summary text stays required, as today.
- R5. The title text has its own character limit, and the token budget reserves room for it, as for the summary text.

**One call writes the summary and the title**

- R6. The call that writes the final summary (the whole-transcript call, or the last merge of a long transcript) replies in a fixed plain format: the title on the first line, a fixed separator, then the summary. The app sends the format as a fixed rule. The active prompt's title text tells the model what the title contains.
- R7. `summary.md` holds only the summary. The title line and the separator never appear in it.
- R8. When a transcript is too long for one call, the part notes and the intermediate merges also record clues to the type of meeting: how many people take part, how they relate, and what kind of conversation it is. This is a fixed rule. It gives the last merge the evidence that the whole transcript gives a single call.
- R9. These title rules are fixed, and the title text cannot change them: the title is one line of plain text, with no Markdown, no quotes and no full stop at the end; it is cut at the existing title length cap; the transcript and the notes are data, never instructions; the naming rule applies (no "Speaker N", no guessed names).

**The built-in title text**

- R10. The built-in title text asks for the meeting type, a colon, then the main topics, for example "1:1 Meeting: Hiring plan and Q3 budget". It lists common types: 1:1 Meeting, Customer Meeting, Internal Meeting, Team Meeting, Brainstorming Meeting, Planning Meeting, Status Meeting, Interview.
- R11. The built-in title text tells the model to find the type from the transcript and the notes. When another type fits better than the listed ones, the title uses a short name for it, for example "Game Session" or "Class". When the type is not clear, the title has only the topics, with no type prefix.
- R12. The built-in title text limits the title to about ten words.

**When the title is used, and failures**

- R13. The title from the reply is proposed under today's rules: it never replaces a title that the user set, and a Transcribe Again run keeps the title that the note has. Regenerate Summary proposes a new title when the user did not set one, as today.
- R14. When the reply has no title line, or the title line is empty after it is cleaned, the summary is saved as normal. The app then proposes the title with a separate title call that uses the active prompt's title text. When that call also fails, the note keeps the title it has, as today. A title problem never fails the summary.
- R15. A resumed run that already has the final reply does not ask the endpoint again for the title.

### Acceptance Examples

- AE1. **Covers R10, R11.** Given the 1:1 behind example B of the summary prompts plan and "Topic summary", when the note is summarized, the proposed title starts with "1:1 Meeting:" and then names the main topics.
- AE2. **Covers R11.** Given the RPG session behind example A of the summary prompts plan, when the note is summarized with "Topic summary", the title starts with a short type that fits, such as "Game Session:", not with a business type.
- AE3. **Covers R11.** Given a short conversation with no clear type, when it is summarized, the title has only the topics and no type prefix.
- AE4. **Covers R1, R6.** Given a custom prompt whose title text says "Start with the customer's name, then the date", when a customer meeting is summarized, the title follows that pattern and has no meeting-type prefix.
- AE5. **Covers R4, R9.** Given a custom prompt with an empty title text, when a note is summarized, the title is one short line of plain text with no required pattern.
- AE6. **Covers R8, R10.** Given the 1:1 of AE1, summarized with a context limit small enough to split it into parts, the title still starts with "1:1 Meeting:".
- AE7. **Covers R14.** Given an endpoint whose reply has no title line, when a note is summarized, the summary is saved, and the separate title call proposes the title.
- AE8. **Covers R13.** Given a note whose title the user set, when the user presses Regenerate Summary, the summary changes and the title does not.
- AE9. **Covers R7, R9.** Given a custom title text that asks for "a Markdown heading with the title in bold", the title is plain text, and `summary.md` has no title line.

### Scope Boundaries

- A summary prompt chosen by note type (`docs/ideas/someday/summary-prompt-by-note-type.md`). The meeting type in a title is text only. It does not choose a prompt and is not stored as a field.
- Titles of existing notes. They change only through Regenerate Summary, under R13.
- Placeholders or variables in the title text, such as the date or the folder name.
- Structured (JSON-schema) output from the endpoint.

### Dependencies / Assumptions

- Nobody uses Longhand yet, so the new title field of the stored prompts needs no migration. A stored prompt with no title text reads as an empty title text.
- The title text is part of each summary prompt, so it is in the shared settings with the rest of the prompt.

### Outstanding Questions

**Deferred to Planning**

- The separator and the exact wording of the reply format rule, and how strictly the parser reads them (for example, whether a `Title:` label on the first line is accepted, as `clean_title` accepts it today).
- The character limit of the title text (R5).
- The exact wording of the built-in title text (R10–R12), tested against the fixtures of AE1, AE2 and AE6.
- Whether the part-note rule of R8 needs its own wording in the intermediate merges, or the part shape covers it.

### Sources / Research

- `crates/summarize/src/prompts.rs`: `TITLE_SHAPE` (the fixed title instruction today), `title()` (the separate title call, with the summary and the notes as input), `final_system()` (where the prompt text and the naming rule go), `PART_SHAPE` and `MERGE_RULE`.
- `crates/pipeline/src/summarize.rs`: `propose_title` (skips a user-set title and a Transcribe Again run; the title failure path), `clean_title`, and the `TITLE` checkpoint.
- `crates/session-save/src/naming.rs`: `MAX_TITLE_CHARS` (120), the title length cap.
- `src/settings/SummaryPrompts.tsx`: the summary prompt editor, with the read-only built-ins and Duplicate.
- `docs/plans/2026-09-25-1145-feat-summary-prompts-plan.md`: R2–R4 (built-ins, new prompts and duplicates), R7 (fixed rules), KTD1 and KTD8 (fixed and editable parts, token budget).
