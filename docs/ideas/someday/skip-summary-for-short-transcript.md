---
title: No summary for a very short transcript
description: "Do not send a transcript of only a few words to the summary model"
area: transcript-and-summary
added: 2026-09-29
source: "anarlog review, 2026-09-29: commits 3f83fb10, 849b994a"
related: [summary-prompts]
---

# No summary for a very short transcript

## Idea
Skip the summary when the transcript is very short. anarlog uses fewer than 5 words or fewer than 160 characters.

## Why
Longhand skips only a transcript with no turns (`crates/pipeline/src/summarize.rs:391`). A transcript of a few words still goes to the summary model, which can invent a summary.

## Notes / Current state
- anarlog 849b994a also scales the length target of the summary with the length of the transcript. This has less value.
