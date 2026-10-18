---
title: Ask questions about a note while it records
description: "A chat panel during recording answers questions about the meeting so far, for example 'recap the last 5 minutes' after the user stepped out"
area: transcript-and-summary
added: 2026-10-18
source: "the user, in the agent tech stack research of 2026-10-18"
related: [agentic-summary, live-transcript-view]
---

# Ask questions about a note while it records

## Idea
During recording, the user opens a chat panel and asks questions about the meeting so far. Examples: "Recap the last 5 minutes", "What did they decide about the budget?", "Who asked for the report?". The model answers from the live transcript and the user's notes. It can use the same read-only tools as the [summary agent](agentic-summary.md), so it can also look at earlier notes.

## Why
The user steps out of a meeting, or loses focus, and wants to catch up without reading the full live transcript.

## Notes / Current state
- The live transcript already exists during recording. The agent needs a tool that reads it by time range ("the text since 14:05"), because it is not final and can be only in memory.
- Uses the same agent loop as [agentic-summary](agentic-summary.md). The research of 2026-10-18 recommends Rig with a small set of read-only tools that Longhand writes, confined to the storage folder. No shell tool.
- U7's attention budget applies: the panel must not draw the participants' attention to the screen. It opens only when the user asks.
- Speech in the room is untrusted input to the model (prompt injection). Answers render without remote images or raw HTML.
- Each question sends part of the transcript to the summary endpoint during the meeting. The panel must say which endpoint, as the processing page does.
