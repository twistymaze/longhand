---
title: Participant names for a session
description: "The user lists who was present, so that the summary can use names"
area: transcript-and-summary
added: 2026-09-25
source: "the summary prompts brainstorm (docs/plans/2026-09-25-1145-feat-summary-prompts-plan.md)"
related: [summary-prompts, transcript-review-and-correction]
---

# Participant names for a session

## Idea
The user tells the app who was present, and the summary uses those names for people. The app has one microphone and no attendee list, so today the summary uses a name only when the transcript makes clear who is speaking.

## Why
A summary reads better with names than with "someone in the group". Meeting tools such as Microsoft 365 Copilot get names from the attendee list.

## Notes / Current state
The summary prompts plan keeps a fixed rule: never "Speaker N", and never a guessed name. A participant list would give the model names that it does not have to guess. Mapping names to speaker turns in the transcript is a separate, harder problem.
