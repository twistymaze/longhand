---
title: LM Studio sends an unsupported reasoning level as "on" and does not refuse it
date: 2026-10-30
category: integration-issues
module: Summary requests to LM Studio (crates/summarize/src/client.rs, crates/summarize/src/context.rs probe)
problem_type: integration_issue
component: service_layer
severity: medium
symptoms:
  - "With Thinking set to Automatic (Low), a summary on qwen3.8-flash-next took 213 s and used reasoning at full length"
  - "Longhand sent reasoning_effort low and got HTTP 200, with no refusal and no fallback"
  - "The LM Studio server log had: Reasoning setting 'low' is not supported by model ... Supported settings: 'on', 'off'. Falling back to reasoning setting 'on'."
root_cause: wrong_api
resolution_type: documentation_update
related_components: [summarize, frontend]
tags: [lm-studio, reasoning, reasoning-effort, thinking, qwen, local-models, summary-speed, reasoning-budget]
retire_when: "LM Studio refuses an unsupported reasoning_effort with an HTTP 400, or applies the nearest supported level; check the server log for the 'Falling back to reasoning setting' warning after a request with reasoning_effort low to an on/off model"
---

# LM Studio sends an unsupported reasoning level as "on" and does not refuse it

## Problem

PR #85 added the Thinking setting. For LM Studio, Automatic is Low. Longhand sends `reasoning_effort: "low"` in each summary request. On `qwen3.8-flash-next`, a summary still took 213 s, and the model reasoned at full length.

## Root cause

Each LM Studio model has its own list of reasoning levels. Many Qwen GGUF models accept only `off` and `on`. When a request gives a level that is not in the list, LM Studio does not refuse it. It writes a warning in its server log, uses `on`, and replies with HTTP 200. Longhand gets no signal, so the R17 fallback for a refused automatic Low does not start.

`GET /api/v1/models` gives the list for each model in `capabilities.reasoning`:

```json
"reasoning": { "allowed_options": ["off", "on"], "default": "on" }
```

Models seen on 2026-10-30:

| Model | `allowed_options` |
|---|---|
| qwen3.8-flash-next | off, on |
| qwen3.8-27b-uncensored-heretic | off, on |
| qwen/qwen3.8-27b | off, low, medium, xhigh, on |

Longhand's probe reads `/api/v0/models`, not `/api/v1/models`, so it does not see this list.

The Reasoning Budget value in the LM Studio model settings (1,024 tokens on that model) did not apply to an API request. A test request without a reasoning field used 5,999 reasoning tokens, until it reached `max_tokens` 6,000.

## Solution

The user accepted the behaviour: a model with only `on` and `off` thinks fully at Low, Medium and High, and Off is the one faster choice. The Thinking text in Settings > AI Provider > Advanced now says this. The Thinking menu shows only "Automatic", and the text next to it gives the default of the provider.

On a model that accepts `low` (`qwen/qwen3.8-27b`), Automatic finished the same note in 83 s.

## Prevention

Do not use the HTTP status to find out whether LM Studio applied a reasoning level. Read the LM Studio server log, or read `capabilities.reasoning.allowed_options` from `/api/v1/models`. If Longhand must know the level that LM Studio uses (for example, to show only the allowed levels in the Thinking menu), read it from that field in the probe.
