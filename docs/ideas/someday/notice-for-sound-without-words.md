---
title: Notice for a long stretch of sound without words
description: "Show a notice on a note when more than 60 s of detected sound had no words after a gap fill"
area: transcript-and-summary
added: 2026-10-05
source: "ce-debug of two notes that stopped at the transcript check, PR #39 (conversation, 2026-10-05)"
related: []
---

# Notice for a long stretch of sound without words

## Idea

When the gap fill finds no words in more than 60 s of detected sound in a note, the note shows a short notice. Below 60 s, the note shows nothing.

## Why

Since PR #39, the transcript check accepts detected sound that a gap fill sent to the model and that had no words, for example background music or room sound. A short stretch is not important in a long meeting. A long stretch can show that the transcript misses speech that the model could not read. The user wants no notice for short stretches.

## Notes / Current state

- Verify already measures this time: `verify::wordless_ms` in `crates/pipeline/src/verify.rs`. Verify writes it only to the log.
- Coverage limitations (`RouteResult::coverage_limitations`) go into the recognition file only. The app shows no limitation now, so a new limitation alone does not make a notice.
- Verify runs after Join, so a value that Verify measures is too late for the transcript header. Measure it in Transcribe, or store it for the note view.
