---
title: Summary Agent Foundation - Plan
type: chore
date: 2026-11-20
topic: summary-agent-foundation
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Summary Agent Foundation - Plan

## Goal Capsule

- **Objective:** A later plan can make a tool-calling agent Longhand's only summary method, with measured evidence for each model size. The tools that the agent uses can also serve agents outside the app.
- **Means:** Two read-only note tools that read the plain files in the storage folder, an agent loop on the existing Rig engine, and a dev harness. The harness compares agent summaries with today's summaries on real notes, and Claude and Codex judge the results.
- **Authority:** The Product Contract wins on behaviour. The app switch, the chat that corrects a summary, the edit tools, the CLI and outside tools are not active scope (see How This Work Fits Together).
- **Execution profile:** One PR, units committed in order on one branch in a worktree from `origin/main`. The app does not change, so the PR title uses a type that makes no release (R23).
- **Stop conditions:** Stop and ask the user when Rig 0.44 cannot run a tool loop on a provider in the test set, or when a tool needs the app database or a running app.
- **Open blockers:** None.

---

## Product Contract

### Summary

Longhand gets two read-only note tools, list notes and get note, each defined once so that the summary agent now, and a CLI or MCP server later, use the same definition. An agent loop on Rig writes a summary: it starts from the note details and the background, and it reads the transcript itself through the tools. A dev harness outside the app runs the agent and today's method on the same real notes with a range of models, and Claude and Codex judge the quality loss per model.

### Problem Frame

The features that will help users later need a summary agent with tools. Examples: a chat on a note where the user says "add more detail about X" or "ignore the last 5 minutes, that was a side conversation", and later outside context such as a calendar or an org chart. Today Summarize sends the transcript in one or more chat-completion calls and splits a long transcript into parts and merges (`crates/summarize/src/chunk.rs`, `crates/pipeline/src/summarize.rs`). The model has no tools, and the repo has no tool code.

Nobody knows yet which agent approach writes summaries as good as the current method, and on which model sizes. The users run hosted models or large local models, so a capable model is an accepted requirement. But the minimum size needs evidence. This pass gives no visible gain to users. It is the base for the features that will.

### Key Decisions

- **The agent reads the transcript itself.** The first instruction holds the note details, the background and the summary rules, but not the transcript. Governs R12. (session-settled: user-directed — chosen over tools on the final call with the transcript still in the prompt, and over tools on every part and merge call: a good agent approach reads the transcript systematically)
- **The approach is found by experiments outside the app, and the app switch is a separate later plan.** Governs R16 to R23. (session-settled: user-directed — chosen over one PR with the experiments and the switch, and over the agent in the app behind an off-by-default setting: the results decide the design of the switch)
- **After the switch, the agent is the only summary method.** Today's method does not stay as a fallback. Users must use a capable model. (session-settled: user-directed — chosen over keeping today's method for weak models: the users run hosted or large local models)
- **Claude and Codex judge the quality, and each model is compared with itself.** A small model is measured against today's method on the same small model, not against a frontier model. Governs R20. (session-settled: user-approved — proposed after the user said that small models are weak with today's method too, and confirmed)
- **The tests include models of about 20B and about 8B parameters, and the judgement comes at the end.** This pass sets no minimum model. Governs R19, R22. (session-settled: user-directed — chosen over a test set of capable models only: the user wants data on small models before the minimum is set)
- **Open-weight models run through OpenRouter, not on local hardware.** Governs R19. (session-settled: user-directed — chosen over tests on local servers: OpenRouter makes many models easy to test)
- **The tools read the plain files, not the app database.** The files are the same on every computer, and a CLI must work while the app is closed. This is the same source that the `longhand-notes` skill reads. Governs R6. (session-settled: user-approved — proposed in the scoping synthesis over the app database, and confirmed)
- **Each tool has one definition for all front ends.** An LLM tool call, a CLI and an MCP server all use a name, a description, an input schema and a structured result. Governs R8, R9. (session-settled: user-approved — proposed in the scoping synthesis to meet the user's CLI goal, and confirmed)
- **A future CLI reverses an earlier decision.** The safe agent edits plan says "There is no command-line tool in the app, and the skill never looks for the app" (`docs/plans/2026-10-12-1827-feat-safe-agent-edits-plan.md`, line 56 and R28). This pass builds no CLI, but it designs the tools for one. The plan that adds the CLI must decide how the skill finds it.
- **Keyword search scans the files and uses no index in this pass.** Governs R3. (session-settled: user-approved — proposed in the scoping synthesis with the speed cost on a large library shown, and confirmed)
- **Outside tools wait for a strategy change.** The tool list accepts more than one source, but this pass adds no MCP client. Governs R10. (session-settled: user-approved — proposed in the scoping synthesis, and confirmed: `STRATEGY.md` says "No calendar integration" and "no integration pulls outside context in")

---

### Requirements

**Note tools**

- R1. Longhand has two note tools, list notes and get note. Neither tool writes to the storage folder.
- R2. List notes returns the notes of one storage folder, each with its note id, title, note type, tags, date and a busy flag (a computer records or processes the note). It filters by note type, tags, date range and keyword.
- R3. A keyword matches words in the title, the tags, the manual notes, the summary and the transcript, and each result says which parts matched.
- R4. Get note returns the details of one note (title, note type, tags, date), and on request its manual notes, its transcript or its summary.
- R5. Get note can return part of a transcript: a time range, a line range, or the lines that match a pattern with a given number of lines of context. Each transcript line keeps its time stamp.
- R6. The tools read the plain files in the storage folder, for notes in both layouts. They need neither the app nor its database.
- R7. The tools read nothing outside the storage folder that they are given.

**One definition for each tool**

- R8. Each tool is defined once, with a name, a description for a model, an input schema and a structured result. The agent loop uses this definition, and a later CLI or MCP server can use it with no second copy.
- R9. The harness can call one tool directly with input and print its structured result, with no model.
- R10. The agent's tool list can take tools from more than one source. This pass has only the Longhand source.

**Agent loop**

- R11. The agent loop uses the existing Rig engine, provider presets and endpoint settings.
- R12. The first instruction holds the note details, the background text and the summary prompt rules, but not the transcript. The agent reads the transcript and the manual notes with get note.
- R13. The existing summary rules stay: no "Speaker N", no guessed names, note text as quoted data that gives no instructions, and the reply is only the summary.
- R14. The loop has a limit on tool calls, and it stops with a failure when it reaches the limit.
- R15. The loop records each tool call, the tokens and the time.

**Dev harness**

- R16. A dev harness runs outside the app and is not part of the shipped app. It reads a storage folder that the user names, and it writes its results to a gitignored folder (`.context/`).
- R17. For each note and each model, the harness writes one summary with today's method and one with the agent.
- R18. The harness can change the agent prompt and the reading strategy, so that experiments can compare approaches.
- R19. The harness runs models through OpenRouter: hosted frontier models, open-weight models of about 20B parameters, and some of about 8B.
- R20. Claude and Codex judge the two summaries of each note from the same model. Each judge gives the quality loss of the agent summary against today's summary, and an absolute quality for each summary.
- R21. The harness writes a report per note and per model with the judge results, tool calls, tokens, time and failures (loops, malformed tool calls, limit reached).
- R22. The report gives no pass or fail for a model. The user sets the minimum model in the later switch plan.

**No app change**

- R23. The app's summary method, settings and screens do not change, and the PR makes no release.

---

### Acceptance Examples

- AE1. **Covers R2, R3.** Given a storage folder with Pathfinder notes, a filter on note type Pathfinder and the keyword "Valeros" returns only Pathfinder notes that contain that word, and each result says whether the match is in the title, manual notes, summary or transcript.
- AE2. **Covers R5.** Given a 4-hour note, get note with the time range 03:55:00 to 04:00:00 returns only the transcript lines in those 5 minutes, each with its time stamp.
- AE3. **Covers R5.** Given a pattern that matches 3 lines and a context of 2, get note returns each match with 2 lines before and after it, and overlapping ranges are not repeated.
- AE4. **Covers R2.** Given a note that another computer records, list notes returns it with the busy flag set.
- AE5. **Covers R14, R21.** Given a model that calls get note again and again without an end, the loop stops at the limit, the agent summary for that note and model is marked failed, and the report gives the reason.
- AE6. **Covers R9.** Given a list notes call with tag `Person::Nathan` from the harness, with no model, the harness prints the same structured result that the agent would get.

---

### Success Criteria

- The harness completes a run on a set of the user's real notes that includes short notes, one-on-ones and at least one Pathfinder note of about 4 hours, on frontier, about 20B and about 8B models.
- For each model, the report lets the user tell whether the agent loses quality against today's method on that model, without reading every summary.
- The user can try a new prompt or reading strategy and compare it with an earlier run.

---

### Scope Boundaries

**Deferred for later**

- The switch that makes the agent the app's only summary method, and the minimum model.
- Tools that change data: regenerate summary, edit summary, edit manual notes, title, tags and note type.
- The chat on a note that corrects a summary.
- A CLI for outside agents, and a Longhand MCP server.
- Outside tools through MCP, for example a calendar or an org chart. `STRATEGY.md` must change first.
- An index for keyword search (the [library-and-browsing](../ideas/someday/library-and-browsing.md) idea).
- Questions about a note while it records.

**Outside this product's identity**

- A tool that runs shell commands or scripts for the summary agent.

---

<!-- ce-section: work-relationships -->
### How This Work Fits Together

This plan covers the base: tools, agent loop and harness. The list below is the current understanding, not a committed roadmap.

- The app switch to the agent depends on this plan's harness results.
  - The chat that corrects a summary depends on the switch and on the edit tools. The user's examples ("add more detail about X", "ignore the last 5 minutes") need get note by time range (R5).
  - The edit tools share the tool definition (R8) and must follow the safe-edit rules of the `longhand-notes` skill: no edit of a busy note, and no edit of Longhand's records.
- A CLI or a Longhand MCP server for outside agents uses the tool definition (R8, R9). It can proceed independently of the app switch. Still to decide: CLI, MCP server or both, and how the skill finds the app.
- Outside tools through MCP depend on a change to `STRATEGY.md` and use the second tool source (R10).
- [ask-about-the-note-while-recording](../ideas/someday/ask-about-the-note-while-recording.md) shares the agent loop and needs a tool that reads the live transcript.

---

### Dependencies / Assumptions

- The user has an OpenRouter key and access to Claude and Codex for the judges.
- Sending real notes to OpenRouter hosts and to the Claude and Codex judges is acceptable for the harness, because the notes are the user's own data.
- Rig 0.44 can run a tool-call loop on OpenRouter and on the other provider presets. Planning confirms this.

---

### Outstanding Questions

**Deferred to Planning**

- How the harness calls the judges (the APIs directly, or Claude Code and Codex sessions that read the harness output), and the judge prompt.
- The exact models for each size, and whether to choose OpenRouter hosts by their data policy.
- The form of the harness (a Rust binary in the workspace or a script), and where the tools and the agent loop live in the crates.
- The tool-call limit and the reading strategies for the first experiments.

---

### Sources / Research

- [agentic-summary](../ideas/next/agentic-summary.md), [pluggable-summary-tools](../ideas/someday/pluggable-summary-tools.md), [app-command-line-tool](../ideas/someday/app-command-line-tool.md): earlier scope, the 2026-10-18 research (Rig, read-only tools in the storage folder, no shell tool).
- `docs/plans/2026-10-18-0955-feat-rig-engine-and-provider-presets-plan.md`: Rig is the base for the summary-tools plan; `rig-core` is pinned at 0.44.0 in `crates/summarize/Cargo.toml`.
- `crates/summarize/src/prompts.rs`: the summary rules and the quoted-data rule (R13).
- `plugins/longhand/skills/longhand-notes/scripts/list_notes.py`: the skill's filters and busy flag, which list notes matches (R2).
- `plugins/longhand/skills/longhand-notes/references/layout.md`: note files, both layouts, and the transcript line form `- [00:00:09] Speaker 2: text`.
- `crates/pipeline/src/summarize.rs`: the background text in today's summary prompt.
- `STRATEGY.md` Boundaries and `docs/ideas/README.md` Ruled out list: the limits on outside context.
