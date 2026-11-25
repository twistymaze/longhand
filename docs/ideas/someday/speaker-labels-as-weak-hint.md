---
title: Speaker labels only as a weak hint in the summary prompts
description: "The summary prompts treat a name beside a speaker label as clear and keep labels in part notes, but a label is reliable only where the speaker changes"
area: transcript-and-summary
added: 2026-11-23
source: "summary lab work of 2026-11-23: the timeline pipeline swapped speakers when it carried label-to-name pairs; the user's correction about diarization"
related: [summary-context-document, agentic-summary]
---

# Speaker labels only as a weak hint in the summary prompts

## Idea
Change the fixed rules of the app's summary prompts so that a speaker label is only a weak hint:

- `PART_LABELS` (part notes and intermediate merges) tells the model to keep the labels and to write a name beside a label, such as "Speaker 2 (Sam)". A later step then reads that pair as a fact.
- `NAMING_RULE` (the final call) says that "a name written beside a label, such as Speaker 2 (Sam), is clear".

Both rules assume that one label is one person for a stretch of the transcript. The diarization is reliable only where it shows that the speaker changes. Over a short stretch the same label is often the same person, but a label can change to another person at any time, and one person has more than one label over a one-hour session.

## Why
In the summary lab, a timeline pipeline that carried label-to-name pairs from one transcript part to the next swapped speakers through whole summaries: the judges found systematic misattribution, for example the two people of a one-on-one with their roles reversed. The fix in the lab (`TURN_RULE` in `crates/summarize/src/prompts.rs` on the `worktree-summary-agent-foundation` branch) uses a label only as a weak hint together with what is said, and gives a name only when what is said supports it. The same risk is in today's chunked summaries, where the part notes write "Speaker 2 (Sam)" and the merge reads it as clear.

## Notes / Current state
- Measure the change with the summary lab before it ships: the judges count misattributed statements for each summary.
- The change touches the fixed rules that every summary prompt gets, so it also changes the summaries of one-call notes.
