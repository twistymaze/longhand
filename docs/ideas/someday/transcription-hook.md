---
title: Script after a note is ready
description: "An optional user script that runs when a note's transcript or summary is ready, for power users"
area: processing
added: 2026-11-15
source: "upkeep review 2026-11-15: Handy v0.10.0 hooks/transcription"
related: []
---

# Script after a note is ready

## Idea
An optional script that the user installs runs after a note is saved with its transcript or summary. Handy v0.10.0 runs `hooks/transcription` from its data folder: the script gets the text on stdin, and its stdout becomes the result.

## Why
Power users can add text fixes, translation or their own agent step without a change to Longhand.

## Notes / Current state
- It runs user code, so it needs a clear opt-in and error reporting.
- Check against the STRATEGY.md boundary "Anything that writes into Longhand from outside": a script that only reads the note fits; a script whose output replaces the transcript may not. Decide this before a plan.
- It fits with the Longhand agent skill and agent-safe note edits.
