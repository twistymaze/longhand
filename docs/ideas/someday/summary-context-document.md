---
title: A context document for each summary
description: "Before the timeline, the summary pipeline makes a short document with the people, the spelling of names and terms, the expected agenda and open items, and every later step uses it"
area: transcript-and-summary
added: 2026-11-23
source: "summary lab work of 2026-11-22 and 2026-11-23: the user's session-distill skill (canon pass) and a second-opinion review of the summary pipeline"
related: [agentic-summary]
---

# A context document for each summary

## Idea
The summary pipeline makes a short context document before it writes the timeline. The document has a fixed form and a size limit, for example at most a tenth of the context:

- **People:** the speaker labels of each person, the name, the evidence, the source and how certain it is.
- **Terms:** the correct spelling of names and terms that the transcript often gets wrong.
- **Expected agenda:** what the session was probably about.
- **Open items:** items from earlier notes that are still open.

Each later step gets the document by the same path as the Background and the note details today: the timeline parts, the writers, the auditors and the reviser.

Context sources fill the document. The code drives each source; an optional agent step with tools can write the same document. Sources inside the `STRATEGY.md` boundaries:

- Earlier notes with the same tags, type or folder (the `list_notes` and `get_note` tools already exist).
- The Background and the user context in Settings.
- A first pass over the transcript that resolves names, as the canon pass of the user's session-distill skill does.

The user can see and correct the document. A correction stays with the note, and "regenerate" uses it. This takes the place of the questions that the session-distill skill asks the person during the run.

## Why
- The transcript spells names and terms wrong. One map of names, used by each step, gives the same name everywhere.
- The timeline pipeline reads the transcript in parts. Without a shared map, a name that part 1 resolves is not known in part 3. The lab now passes a people list from part to part; this idea makes that list a real document with more sources.
- The future tools for earlier notes (and possibly outside sources) need one clear place in the pipeline. This document is that place: a new source adds content to the document, not a new step.

## Notes / Current state
- Rules:
  - The context step is optional, and its failure never stops the summary.
  - Its content is untrusted data: text in an earlier note can look like an instruction.
  - Its content is a hint, not a source. A statement in the summary that only the context document supports is an error; the review step can check for it as a sixth error kind.
- The context step sends text from other notes to the summary endpoint. The user must choose this in Settings, and the processing page must say so.
- **Open, needs a decision of the user:** outside sources such as a calendar through MCP. `STRATEGY.md` has the boundaries "No calendar integration" and "Nothing writes into Longhand … no integration pulls outside context in", and the ideas README lists calendar integration under Ruled out. An outside source needs a change to the strategy first.
- The summary lab work (plan `docs/plans/2026-11-20-1211-chore-summary-agent-foundation-plan.md`, branch `worktree-summary-agent-foundation`) has the timeline pipeline, the review step and a people list from part to part. It has no context document yet.
