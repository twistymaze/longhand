---
title: Summary Speed with Local Models - Plan
type: feat
date: 2026-10-29
topic: summary-speed-with-local-models
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Summary Speed with Local Models - Plan

## Goal Capsule

- **Objective:** A summary with a local reasoning model finishes at the first try, as fast as the user's server allows, and the user can control how much the model thinks and how many requests go at the same time.
- **Product authority:** This Product Contract. The user confirmed the scope on 2026-10-29: reply allowance, thinking control, requests built by Rig, and parallel summary parts, as one feature.
- **Open blockers:** None.

---

## Product Contract

### Summary

Longhand gives each summary request a reply allowance from the room that is left in the context, instead of a fixed maximum of 8,192 tokens. Each AI provider gets two advanced settings: Thinking, and Requests at the same time. Rig builds the OpenAI-compatible requests natively, so that Rig handles the differences between providers.

### Problem Frame

On 2026-10-29 the user summarized an 18-minute one-on-one with `qwen3.8-flash-next` on LM Studio, with a 131,072-token context. The prompt was 6,571 tokens, so the note went in one request. The model used all 8,192 reply tokens on hidden reasoning and wrote no summary text (`finish_reason: length`, `reasoning_tokens: 8192`). Longhand sent the whole request again with 16,384 tokens, and the model started its reasoning again from zero. The second try finished after 10,869 reasoning tokens. The first try cost about 160 seconds and gave nothing.

The 8,192 maximum comes from the first summary code (2026-09-22), when the default context limit was also 8,192. The default context limit became 128,000 in PR #17, but the reply maximum did not change. No document records a reason for it.

Longhand sends no reasoning control, so each model reasons as much as its default allows. A test on the same LM Studio server showed that `reasoning_effort: "none"` gave 0 reasoning tokens and `"low"` about half of the default. LM Studio ignored `chat_template_kwargs.enable_thinking`.

A long transcript that does not fit one request goes in summary parts. Longhand sends the summary parts and the merge groups one at a time, although LM Studio loaded the model with 4 slots.

The Rig migration (`docs/plans/2026-10-18-0955-feat-rig-engine-and-provider-presets-plan.md`, R2 and KTD3) kept the OpenAI-compatible request body byte-for-byte the same as before Rig. Longhand therefore writes `messages` and `"stream": false` itself through Rig's additional parameters. LM Studio and Custom use a Longhand gateway dialect, and Rig refuses every generation option on an unknown gateway, reasoning included. So Rig cannot send a reasoning level to LM Studio today.

### Key Decisions

- **One feature, four parts.** The reply allowance, Thinking, requests built by Rig and parallel summary parts ship in one PR, because together they make local summaries fast. Governs R1–R15. (session-settled: user-directed — chosen over the reply allowance, Thinking and Rig requests without parallel summary parts: the user wants all four now)
- **The reply allowance comes from the room left, with limits.** The rule replaces a fixed maximum that did not follow the context limit. Governs R1, R2, R3. (session-settled: user-directed — chosen over a larger fixed fraction with a higher ceiling and over a user setting for the maximum reply: the user wants the allowance to follow the context, with a buffer of at most 90% of the context)
- **Thinking is a per-provider setting with a default per provider kind.** Local providers start at Low because a summary gains little from long reasoning; hosted providers start at Model default so that their requests do not change. Governs R5, R6, R8. (session-settled: user-approved — chosen over Model default everywhere, over no setting with Low for local providers, and over Off for local providers: the user accepted Low as the local default)
- **Requests at the same time is a per-provider setting.** Longhand does not know a server's slot count, and a request beyond the server's capacity waits in the server's queue. Governs R12, R13. (session-settled: user-approved — chosen over fixed numbers with no setting, over a count reported by the server, and over a default of 1 everywhere)
- **Rig builds the requests.** Rig owns the differences between provider APIs; Longhand keeps its prompts, chunking and merge. The byte-parity rule of the Rig migration (R2 and KTD3 of that plan) ends here. Governs R9, R10, R11. (session-settled: user-directed — chosen over keeping hand-built request bodies: the user said that the purpose of Rig was to let Rig handle the complexity of the LLM APIs)
- **The new settings are advanced.** Most users never change them, because the defaults suit each provider kind. Governs R15.
- **A refused thinking level is shown, not replaced.** If Longhand silently went back to Model default, it would ignore the user's choice. Governs R7.

### Requirements

**Reply allowance**

- R1. The reply allowance of a summary request is the context that is left after the prompt, but not more than 90% of the context limit, and not more than the model's output maximum when Longhand knows it.
- R2. For a hosted provider whose output maximum Longhand does not know, the reply allowance stays below a fixed safe value, so that the provider does not refuse the request.
- R3. Whether a note goes in one request or in summary parts does not change: the split decision uses the same minimum reply room as today, and only the allowance of each request grows.
- R4. When a reply is cut off at its allowance, Longhand still retries with a larger allowance, as today.

**Thinking**

- R5. Each AI provider has a Thinking setting with the values Model default, Off, Low, Medium and High. Model default sends no reasoning control.
- R6. Thinking starts at Low for LM Studio and Ollama, also for entries that exist before the update, and at Model default for every other provider. A new install and an update give the same defaults.
- R7. When the model or the provider refuses the chosen Thinking level, the summary fails with a message that names the Thinking setting and asks the user to choose another level.
- R8. The Thinking setting applies to every request of a summary, including the summary parts, the merges and the title request.

**Requests built by Rig**

- R9. Rig builds the request body for every provider. Longhand no longer writes the messages or the stream field itself.
- R10. Rig sends the Thinking level in each provider's own field. For Custom, whose server Rig cannot know, a level other than Model default goes as the standard OpenAI `reasoning_effort` field.
- R11. The token-cap behaviour stays: LM Studio and Custom send `max_tokens` for every model id, and a request that a server refuses because of `max_tokens` is sent once more with `max_completion_tokens`.

**Parallel summary parts**

- R12. Each AI provider has a Requests at the same time setting. Its default is 4 for OpenAI, Anthropic, Gemini and OpenRouter, 2 for LM Studio and Ollama, and 1 for Custom.
- R13. A summary in parts sends up to that number of summary parts at the same time, and also up to that number of merge groups in each merge layer.
- R14. The summary progress counts finished summary parts and never goes back while the requests run.

**Settings**

- R15. Context limit, Thinking and Requests at the same time are in the Advanced group of the AI Provider pane, and that group is closed by default.

### Acceptance Examples

- AE1. **Covers R1, R3.** Given LM Studio with a 131,072-token context and a note whose prompt is 6,571 tokens, the note goes in one request with a reply allowance of about 118,000 tokens (90% of the context), and the reasoning model finishes at the first try.
- AE2. **Covers R1, R2.** Given an Anthropic model whose output maximum Longhand does not know, the request's `max_tokens` is the fixed safe value, and the provider accepts the request.
- AE3. **Covers R3.** Given a transcript that went in 3 summary parts before this change at the same context limit, it still goes in 3 summary parts.
- AE4. **Covers R6.** Given an LM Studio entry saved before the update, after the update its Thinking setting shows Low, and its next summary request asks for low reasoning. An OpenRouter entry shows Model default, and its request has no reasoning field.
- AE5. **Covers R7.** Given Thinking: Off and a hosted model that cannot turn reasoning off, the summary fails with a message that names the Thinking setting. Longhand does not send the request again without the level.
- AE6. **Covers R10.** Given Custom with Thinking: Low, the request contains `"reasoning_effort": "low"`. Given Custom with Model default, the request contains no reasoning field.
- AE7. **Covers R12, R13, R14.** Given LM Studio with Requests at the same time 2 and a transcript in 5 summary parts, at most 2 requests run at any moment, and the progress goes from 0 to 5 parts without going back.
- AE8. **Covers R15.** Given the AI Provider pane with LM Studio active, the Advanced group is closed. When the user opens it, it shows Context limit, Thinking and Requests at the same time.

### Success Criteria

- On the user's LM Studio setup, the 2026-10-29 one-on-one gets a summary with one summary request and no cut-off retry.
- With Thinking at Low, the same note takes clearly less time than with Model default on the same model.
- Summaries from OpenAI, Anthropic, Gemini, OpenRouter, LiteLLM (Custom) and LM Studio still succeed after the change to requests built by Rig.

### Scope Boundaries

- Keeping finished summary parts after a cut-off reply stays a separate idea (`docs/ideas/`, keep-finished-parts-on-cut-off-reply).
- The summary queue still runs one note at a time. Summarizing several notes at the same time is not in scope.
- Streaming replies is not in scope.
- Longhand does not ask the server how many slots it has.
- A thinking token budget (a number of tokens) is not offered; Thinking has levels only.

### Dependencies / Assumptions

- LM Studio accepts `reasoning_effort` values none, low, medium and high on `/v1/chat/completions`. The 2026-10-29 test confirmed none and low on LM Studio with `qwen3.8-flash-next`.
- Current OpenAI-compatible servers (LM Studio, LiteLLM, vLLM, OpenRouter) accept a system message whose content is an array of text parts, as Rig sends it. This is not tested yet.
- Ollama runs a second request at the same time only when its server allows more than 1 parallel request. Otherwise the second request waits; the 30-minute reply deadline limits that wait.
- Existing users have saved settings at schema version 7, so the new settings need a migration that keeps older builds working.

### Outstanding Questions

**Deferred to Planning**

- Which Rig dialect LM Studio uses (Rig's llama.cpp dialect, or the Longhand gateway with Longhand adding `reasoning_effort`), and whether that dialect keeps `max_tokens` (R10, R11).
- The fixed safe allowance for hosted providers whose output maximum is unknown, and which providers report an output maximum that Longhand can read (R1, R2).
- How the Ollama context size (`num_ctx`) follows the larger allowance and the parallel requests without high memory use or a model reload (R1, R13).
- What Rig does with the Thinking setting on a model that does not reason, for each provider (R5, R7).
- How a cut-off reply in one summary part acts on the summary parts that run at the same time (R4, R13).
- The exact UI wording of the two settings and their help text (R12, R15).

### Sources / Research

- LM Studio log `~/.lmstudio/server-logs/2026-10/2026-10-29.1.log` and Longhand log `~/Library/Logs/net.ewal.longhand/Longhand.log`, 2026-10-29 22:22–22:33 UTC: the cut-off and the retry.
- `crates/summarize/src/tokens.rs:43`: the current reply allowance, `(limit / 4).clamp(512, 8_192)`.
- `crates/pipeline/src/summarize.rs:724`: the cut-off retry; `:929`: the one-request decision; `:956` and `:1009`: the sequential loops for summary parts and merge groups; `:1060`: the title request uses the same reserve.
- `crates/summarize/src/client.rs:366`: the Longhand gateway dialect; `:1019`: the request built through additional parameters; `:92`: the request deadlines.
- `crates/summarize/src/context.rs:76`: the LiteLLM probe ignores the model's output maximum.
- Rig 0.44 `src/completion/options.rs:212` (`Reasoning`, `Effort`) and `src/providers/openai/options.rs:518` (unknown gateway refuses options), `:1097` (llama.cpp mapping).
- `docs/plans/2026-10-18-0955-feat-rig-engine-and-provider-presets-plan.md`: R2, KTD2, KTD3 (byte parity) and KTD7 (Ollama context size).
- `src/settings/ProviderForm.tsx:470`: the Advanced group; `src/settings/RecognitionOptions.tsx:456`: a closed Advanced options group.
- LM Studio 0.4.8 changelog (adds `reasoning_effort`): https://lmstudio.ai/changelog/lmstudio-v0.4.8
