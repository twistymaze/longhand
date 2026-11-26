---
title: Context limit from Rig's catalog in automatic detection
description: "With automatic detection, use the model's input limit in Rig's catalog, so OpenAI and Anthropic models do not get the 128,000-token default"
area: transcript-and-summary
added: 2026-11-21
source: "summary agent foundation session, 2026-11-21 (KTD14 of docs/plans/2026-11-20-1211-chore-summary-agent-foundation-plan.md)"
related: [agentic-summary]
plan: docs/plans/2026-11-25-1735-feat-summary-pipeline-in-app-plan.md
---

# Context limit from Rig's catalog in automatic detection

## Idea
When the context limit setting is automatic, the summary path uses the input limit of the model in Rig's built-in catalog when the catalog knows the model. Today the probe sends no request for OpenAI and Anthropic, and the catalog gives only the output maximum, so these models get the default of 128,000 tokens. `summarize::context::catalog_input_limit` already reads the limit for the summary agent.

## Why
A model with a larger window, for example Claude Haiku 4.5 (200,000 tokens) or GPT-6 Luna (922,000 tokens), gets a long transcript split into more parts than it needs. The change needs a new budget source, and a saved plan with that source must stay safe for an older build.

## Outcome
PR #118 (2026-11-26) shipped it: with automatic detection, an OpenAI or Anthropic model that the catalog knows gets the catalog's input limit. A value that the user sets still wins.
