---
title: Anthropic request format for the summary endpoint
description: "An option to send summary requests in the Anthropic Messages format, for endpoints that accept only that format"
area: transcript-and-summary
added: 2026-10-15
source: "Handy review, 2026-10-15: commit 14ae9c6"
related: []
---

# Anthropic request format for the summary endpoint

## Idea
Add a request-format option to the summary endpoint settings: OpenAI-compatible (now) or Anthropic Messages.

## Why
Longhand sends only `chat/completions` requests (`crates/summarize/src/client.rs`). Some endpoints accept only the Anthropic format, for example Claude on Microsoft Foundry. Handy added this option in 14ae9c6.

## Notes / Current state
- The walking-skeleton plan left other API shapes out "until a specific provider justifies it". Add this when a user needs such an endpoint.
- 2026-10-18: The Rig engine plan (`docs/plans/2026-10-18-0955-feat-rig-engine-and-provider-presets-plan.md`) keeps Custom OpenAI-compatible only. The summary-tools plan adds this format, because prompt caching through LiteLLM pays off only when an agent sends the transcript again on each turn. With Rig, it is the native Anthropic provider with a custom base URL.
