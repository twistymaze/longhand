---
title: Summary in the language of the transcript
description: "Write the summary and title in the main language of the transcript, with an optional setting for a fixed language"
area: transcript-and-summary
added: 2026-10-15
source: "anarlog review, 2026-10-15: commits 4d6187bb, 02afc2f1"
related: [transcribe-cpp-update]
---

# Summary in the language of the transcript

## Idea
Tell the summary model to write the summary and the title in the main language of the transcript. A setting can choose a fixed language in its place.

## Why
`crates/summarize/src/prompts.rs` has no language instruction. A model can write an English summary of a German meeting. anarlog added this in 4d6187bb and 02afc2f1.

## Notes / Current state
- Upkeep review, 2026-10-19 (issue #47): transcribe.cpp 0.4.0 has a langid role that detects the spoken language. With `transcribe-cpp-update`, Longhand can use it to choose the summary language.
