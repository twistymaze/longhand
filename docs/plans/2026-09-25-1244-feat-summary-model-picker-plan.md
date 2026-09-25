---
title: Summary Model Picker - Plan
type: feat
date: 2026-09-25
topic: summary-model-picker
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Summary Model Picker - Plan

## Goal Capsule

- **Objective:** A person who sets up summaries chooses a model from the models that their server actually offers, and sees whether the Base URL and API key work before the first summary fails.
- **Means:** The Model text field becomes a searchable picker that Longhand fills from the endpoint's list of models (R9). A status line reports the result of each list request (R14). Base URL starts as OpenAI's endpoint (R1).
- **Product authority:** This plan covers the Endpoint and Credential groups of the Summaries pane in Settings. Provider presets and per-model details are not active scope. The Product Contract wins on behavior.
- **Open blockers:** None.

---

## Product Contract

### Summary

In Settings, Model becomes a searchable picker that shows every model the server lists. Base URL starts as `https://api.openai.com/v1`, and the API key field moves above Model, because the list needs the key. A status line reports each list request: connected with a count of models, key rejected, server unreachable, or server that does not list models. The user can still type any model name.

### Problem Frame

Today Model is a free-text field (`src/settings/EndpointForm.tsx`). The user must know the exact model id, find it on the provider's website, and type it without a mistake. Nothing tells the user that the Base URL, the key or the model name is wrong until a note finishes and its summary fails. The note is then saved with a stub summary (`src/session/ProcessingStatus.tsx`). A new install also starts with an empty Base URL, so the user must find the OpenAI URL too, although OpenAI is the most common choice.

Longhand already reads the models list, but only to find the context limit of the saved model (`crates/summarize/src/context.rs`). No code gives the list to the UI.

### Key Decisions

- KD1. **The model list is also the connection check.** A list request proves that the server is reachable and that it accepts the key. *(session-settled: user-directed — chosen over a model list with no status and over a model list with provider presets: an early visible error is worth more than presets.)* Governs R14, R15.
- KD2. **Show every model, with search.** Name rules that remove embedding, speech or image models can hide a valid model on a gateway or a local server, and they need upkeep. *(session-settled: user-directed — chosen over hiding models that do not look like chat models, and over hiding them with a "Show all" control.)* Governs R9.
- KD3. **OpenAI is the default Base URL.** The user asked for it, so that a new install needs only a key and a model choice. Governs R1, R2.
- KD4. **No model is chosen for the user.** Model names change, and a hardcoded default goes out of date. *(session-settled: user-directed — chosen over a named default model and over an automatic choice from the list by a rule such as "newest small model".)* Governs R12.
- KD5. **A typed model name is always accepted.** Azure deployment names, gateway wildcard routes and servers without a models list need a name that the list does not show. *(session-settled: user-approved — proposed with the gateway and Azure cases shown; the user confirmed.)* Governs R10, R11, R13.
- KD6. **The check lists models only and sends no chat request.** A list request uses no tokens and costs nothing. The trade-off: a key that can list models but cannot chat still fails only at the first summary. *(session-settled: user-approved — proposed with that trade-off shown; the user confirmed.)* Governs R5.
- KD7. **Longhand sends no request to the default endpoint until the user acts.** The OpenAI default comes from Longhand, not from the user. So Longhand does not contact OpenAI until a key is saved, and opening Settings sends nothing. *(session-settled: user-approved — proposed with the privacy reason shown; the user confirmed.)* Governs R4.

### Requirements

**Default endpoint**

- R1. When no Base URL was ever saved, Base URL shows `https://api.openai.com/v1`, and Model is empty.
- R2. A saved Base URL does not change, and a Base URL that the user clears stays empty.

**Layout**

- R3. The Summaries pane shows Base URL, then API key, then Model, then the status line (R14). The Context limit field and the plain-HTTP warning stay as they are.

**Requesting the list**

- R4. Longhand requests the model list only for an endpoint that the user acted on: a key is saved for its host, or the user entered the Base URL. The built-in default Base URL with no saved key is never contacted.
- R5. The list request reads the list of models from the Base URL and sends no chat request.
- R6. Longhand requests the list again when the user commits a new Base URL, saves or removes the key, opens the Summaries pane, or selects a Refresh control next to the status line. Each request follows R4.
- R7. A list result applies only to the Base URL and key that it was requested for. A result for a Base URL or key that has since changed is discarded.
- R8. The list request follows the rules that apply to summary requests: it follows no redirect, and it sends the key only to the host that it is saved for.

**The picker**

- R9. Model is a searchable picker. It lists every model id that the server returns, sorted by name, with no filter.
- R10. The user can type a model name that is not in the list and save it.
- R11. When a list loads and the saved model is not in it, the picker still shows the saved model. A note says that this server does not list it. The saved model does not change.
- R12. Until the user chooses, Model is empty and shows "Choose a model". Longhand never chooses a model.
- R13. When no list is available because Longhand did not request it, the request failed, or the server does not list models, Model works as a text field.

**Status**

- R14. The status line reports the latest list request: checking, connected with the number of models, API key needed, API key rejected, server unreachable, or this server does not list models. Planning writes the exact text, in plain language.
- R15. The status line only reports. It does not block saving settings or recording, and processing still uses a typed model after a failed list request.

### Key Flows

- F1. First setup with OpenAI
  - **Trigger:** The user opens the Summaries pane on a new install.
  - **Steps:** Base URL shows the OpenAI URL, and the status line says that an API key is needed. Longhand sends no request. The user saves a key. The status line shows checking, then connected with the number of models. The user searches for a model and chooses it.
  - **Covered by:** R1, R3, R4, R6, R9, R12, R14
- F2. Local server with no key
  - **Trigger:** The user replaces the Base URL with a server on this computer, such as Ollama or LM Studio.
  - **Steps:** The user commits the URL. Longhand requests the list with no key, and the picker shows the local models.
  - **Covered by:** R4, R6, R9
- F3. Gateway that does not list models
  - **Trigger:** The user enters a gateway URL that returns no models list.
  - **Steps:** The status line says that this server does not list models. Model works as a text field, and the user types the model name.
  - **Covered by:** R10, R13, R14

### Acceptance Examples

- AE1. **Covers R4, KD7.** Given a new install with no key, when the user opens the Summaries pane, Longhand sends no request to `api.openai.com`.
- AE2. **Covers R2.** Given an install with the Base URL `http://localhost:11434/v1`, when the user updates Longhand, Base URL stays `http://localhost:11434/v1`.
- AE3. **Covers R11.** Given the saved model `gpt-4o`, when the server's list does not contain `gpt-4o`, the picker shows `gpt-4o` with the note, and summaries still use `gpt-4o`.
- AE4. **Covers R14, R15.** Given a key that the server rejects, when the list request returns, the status line says that the key was rejected, and the user can still type a model and save it.
- AE5. **Covers R7.** Given a list request that is still running, when the user changes the Base URL, the result of the first request does not change the picker or the status line.
- AE6. **Covers R9.** Given OpenAI's list of about 100 models, when the user types "mini", the picker shows only the models whose ids contain "mini", including embedding models if their names match.

### Scope Boundaries

- Provider presets (OpenRouter, Ollama, LM Studio and others) are not part of this plan. OpenAI is the only built-in default.
- The picker does not filter by model type.
- The picker does not show context length, price or other details for each model.
- The check sends no test chat request.
- A different model for each summary prompt is out of scope. The summary-prompts plan uses one app-wide model.
- API shapes other than OpenAI-compatible stay out of scope (walking-skeleton plan, KD12).

### Outstanding Questions

**Deferred to Planning**

- How settings tell "never saved" apart from "cleared by the user" for R1 and R2.
- The timeout for a list request, and whether a list result is kept between openings of the Summaries pane.
- Whether the context-limit probe in `crates/summarize/src/context.rs` can use the list response instead of a second request.

### Sources / Research

- `src/settings/EndpointForm.tsx` — the current Endpoint and Credential groups. The Credential group shows only when a Base URL is set.
- `src-tauri/src/managers/settings.rs` — `endpoint_configured()` needs a Base URL and a model. The defaults are empty.
- `crates/summarize/src/context.rs` — the existing request to the models list, used for the context limit. Its path is relative to the Base URL.
- `crates/summarize/src/client.rs` — no redirects, bearer key.
- `crates/secrets/src/lib.rs` — keys are saved in Keychain for each host and port.
- `reference-source/Handy/src-tauri/src/llm_client.rs` (`fetch_models`) and `reference-source/anarlog/apps/desktop/src/settings/ai/shared/list-openai.ts` — how both apps read `data[].id` from the models list. anarlog's `model-combobox.tsx` is a searchable picker that accepts typed names.
