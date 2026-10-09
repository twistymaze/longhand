---
title: Pause a recording
description: "Pause a note during a meeting and continue it later, without Stop: nothing is captured while paused, and processing starts only at Stop"
area: capture
added: 2026-10-09
source: "pause-recording brainstorm (conversation, 2026-10-09)"
related: [recording-state-widget]
plan: docs/plans/2026-10-09-1504-feat-pause-recording-plan.md
---

# Pause a recording

## Idea
A Pause action, separate from Stop. While paused, the microphone and Computer audio are closed and nothing is saved. Continue records into the same note. The transcript and the summary input mark each pause.

## Why
A long meeting can have a break with casual talk, or a discussion that is too sensitive to record. Stop and then Resume works, but Stop means "I'm done" and starts processing too early.

## Notes / Current state
The plan holds the requirements. A forgotten pause is left to the recording state widget idea.
