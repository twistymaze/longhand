---
title: AI provider presets on a Rig engine
description: "Choose OpenAI, Anthropic, Gemini, OpenRouter, Ollama, LM Studio or Custom in one AI Provider pane, with Rig in place of the summary client"
area: transcript-and-summary
added: 2026-10-18
source: "the user, in the agent tech stack research and brainstorm of 2026-10-18"
related: [agentic-summary, summary-endpoint-anthropic-format, ask-about-the-note-while-recording]
plan: docs/plans/2026-10-18-0955-feat-rig-engine-and-provider-presets-plan.md
---

# AI provider presets on a Rig engine

## Idea
Rig sends the summary and title requests for every provider. A new AI Provider pane in Settings offers a short list of providers plus Custom (OpenAI-compatible) in one group, in place of the three cards in Summaries. Each provider remembers its key, model and address.

## Why
Today the user must know an OpenAI-compatible address and whether it ends in `/v1`, and Claude works only through a gateway. The later agent work needs an engine that can call tools.

## Notes / Current state
The plan holds the requirements. It is the first step before [agentic-summary](../someday/agentic-summary.md).

## Outcome

Shipped in PR #39: summaries go through Rig, and the AI Provider pane has the presets OpenAI, Anthropic, Gemini, OpenRouter, Ollama, LM Studio and Custom. The same PR names the steps of a summary in parts.
