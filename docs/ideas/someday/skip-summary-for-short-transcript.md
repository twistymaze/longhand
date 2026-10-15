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
- anarlog removed its summary length modes and its length scaling in 7eb62eab and 351ff7f5 (review of 2026-10-15). The short-transcript skip is still in anarlog.
