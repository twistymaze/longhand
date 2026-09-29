---
title: Pluggable tools for the summary agent
description: "Let the summary agent use outside tools, such as today's calendar, through a plug-in system like MCP"
area: transcript-and-summary
added: 2026-09-28
source: "the user, in the idea review of 2026-09-28, as a follow-up to agentic-summary"
related: [agentic-summary]
---

# Pluggable tools for the summary agent

## Idea
A general way to give the [summary agent](agentic-summary.md) more tools, for example through MCP. One example: a tool that reads the calendar appointments of the day, so that the agent can find the meeting's topic and the attendees.

## Why
Business context would help the agent understand a note. The user does not want Longhand to build an integration for Outlook and every other calendar. A plug-in system would let the user connect the tools they already have.

## Notes / Current state
- **Conflicts with `STRATEGY.md`.** The Ruled out list has "Calendar integration" and "Anything that writes into Longhand from outside", and the strategy says "no integration pulls outside context in". Change the strategy before any work on this idea.
- **No arbitrary commands.** The user does not want a tool that runs shell commands or arbitrary scripts.
- It depends on [agentic-summary](agentic-summary.md).
