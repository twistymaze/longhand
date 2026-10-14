---
title: A summary that ran in a short wake with no network failed, and kept the default budget for the next run
date: 2026-10-14
category: logic-errors
module: Summary stage and its endpoint client (crates/summarize/src/context.rs probe, crates/summarize/src/client.rs send, crates/pipeline/src/summarize.rs produce)
problem_type: logic_error
component: service_layer
symptoms:
  - "A note that the user stopped just before closing the lid was saved with no summary: could not reach the endpoint: error sending request"
  - "Regenerate Summary on that note failed later, with the network up, after a reply was cut off at 2048 tokens"
  - "Transcribe Again on the same note worked at once, with the same endpoint and model"
  - "The log showed budget 8192 tokens (Default) for that run, and 1000000 tokens (Litellm) for every other run"
  - "After PR #15, a note that already had the saved 8192 plan still failed on each Regenerate Summary: the model's replies do not fit in its context"
root_cause: missing_validation
resolution_type: code_fix
severity: high
framework_version: rust 1.94.0
last_updated: 2026-10-14
tags: [summary, summary-regeneration, context-probe, budget, plan, checkpoint, stale-state, not-reachable, retry, sleep, wake, darkwake, network, context-limit, server-error]
related_components: [summarize, background_job]
---

# A summary that ran in a short wake with no network failed, and kept the default budget for the next run

## Problem

The user stopped a recording and closed the laptop lid at once. The Mac slept, then woke for 15 seconds with no network (a macOS maintenance wake, "DarkWake"). The summary stage ran in that wake. Its request failed, and the stage had already saved a budget plan with the 8192-token default. A later Regenerate Summary used that saved plan and failed again.

## Symptoms

- The app log (UTC times) showed, 4 seconds after the wake: `summarizing <id> with claude-haiku-5-5 on litellm.ewal.io: budget 8192 tokens (Default)`, then `summarization failed ...; saving without it: summarizing part 1 of 4: could not reach the endpoint: error sending request`.
- Other runs on the same endpoint logged `budget 1000000 tokens (Litellm)`.
- Regenerate Summary later logged `cut off a layer-0 reply at 2048 tokens; retrying with 4096` and failed after about 2 minutes.
- Transcribe Again on the same note finished the summary in 17 seconds.
- The other computers showed the note as not synced. The laptop slept again 11 seconds after the save, so the sync client could not send the files until the next wake.

## What Didn't Work

- **"Same endpoint, so the endpoint is not the cause."** This was correct, and it was the important clue. Transcribe Again and Regenerate Summary used the same endpoint, so the difference had to be saved state. Transcribe Again deletes the summary checkpoints (KTD14 of the recognition models plan). Regenerate Summary resumes them when the prompt did not change.
- **The note folder alone did not explain the failure.** `longhand.json` showed only the final, good result. The cause came from three sources together: the app log of the laptop (`~/Library/Logs/net.ewal.longhand/Longhand.log`, UTC times), the sleep and wake lines of `pmset -g log` (local times), and the note record. The `pmset` lines showed `Clamshell Sleep` at the stop, a `DarkWake ... 15 secs` with no user activity, and the next sleep 11 seconds after the save.
- **The app log does not record why a Regenerate Summary fails.** The error goes only to the window. Its cause came from the saved plan, not from the log.

## Solution

Three changes, in PR #15.

1. **The probe tells "cannot reach" from "no data".** `context::probe` returned `None` both when the endpoint did not answer and when its answers named no context size. `context::resolve` then gave the 8192 default, and `produce` saved that plan as a checkpoint (`crates/pipeline/src/summarize.rs:545`) before the first completion request. Now `probe` returns an error when the first probe cannot reach the endpoint (`crates/summarize/src/context.rs:117-123`), and the stage fails before it saves a plan (`crates/pipeline/src/summarize.rs:527`). After the endpoint answered once, a later probe that fails is a miss, as before, so a server that closes the connection on an unknown route still gets the default.

2. **A summary request tries again after a failed connection.** `Client::send` (`crates/summarize/src/client.rs:275`) waits and sends again after each wait of `RETRY_WAITS` (`client.rs:121`, about 7 minutes in total). It retries only a reqwest error with `is_connect()` (`client.rs:316`). Any other send error, a broken reply, or a response deadline can follow a request that the endpoint received, so a new try could make the model write the reply twice. Only the summary stage sets the waits. A title request is sent once (`crates/pipeline/src/summarize.rs:212`), because the title job holds the one processing permit, and a 7-minute wait for an optional title would stop the processing of the next note.

3. **Transcribe Again proposes a title when the note has none.** It skipped the title turn for every run, to keep the title that the note has (R16). This note never got a title, because its first summary failed before the title turn. The rule now applies only when the note has a title (`crates/pipeline/src/summarize.rs:870-884`).

## Why This Works

- With no saved plan, the next run probes the endpoint again and gets the real context size. Regenerate Summary then runs in one call, as Transcribe Again did.
- On macOS, a tokio timer stops while the Mac sleeps, because it measures time that does not include sleep. A retry wait that starts in a short wake continues after the next full wake, when the network is back. So the retry covers the lid-closed case without a wake event.
- `is_connect()` is the only send error that proves the endpoint did not receive the request. A retry of any other error can charge for the same summary twice.

## Prevention

- **Do not save a fallback value as a decision when the input was not available.** A default that the code uses because a probe failed is a guess for this run only. Saving it in a checkpoint makes every later run use the guess. The same rule applies to "folder not reachable" in [the Move doc](move-dropped-notes-when-old-folder-went-away.md).
- **Retry a network request only when the error proves that the request did not arrive.** Do not retry an LLM completion on a timeout or a closed connection.
- **Keep optional network work out of long waits when it holds a shared permit.**
- **Tests.** Each test fails without its fix:
  - `a_summary_that_cannot_reach_the_endpoint_keeps_no_budget_plan` (`crates/pipeline/tests/runner.rs`) reproduces the saved 8192/2048 plan with a closed local port.
  - `a_summary_waits_for_the_endpoint_to_come_back_and_completes` opens the port only after a `StageStarted { Summarize }` event, so the first try is always refused. A fixed delay from the test start did not work: transcription took longer than the delay, and the test passed with no retries.
  - In `crates/summarize/tests/client.rs`: a refused connection is retried, a connection that closes after the server read the request is sent once, and a reply that breaks is sent once.
  - `transcribe_again_on_a_note_with_no_title_stores_a_title_request`.
- **Forensics for a "failed after I closed the lid" report.** Collect the app log, `pmset -g log | grep -E '(Sleep|Wake|DarkWake)'` for the day, and the note's `longhand.json`. Convert the log's UTC times to local time before you compare them with the `pmset` lines.

## Follow-up: error answers and plans that were already saved (PR #17)

PR #15 did not cover two cases. A laptop on version 1.1.2 woke later the same day and showed both.

- **An error answer still gave the default.** The first probe got an answer from the server, not a connect failure, so PR #15's check did not stop the run. The run saved the 8192 plan again. The first completion then failed in the same second.
- **A note that already had the saved plan stayed broken.** Regenerate Summary keeps a saved plan when the prompt did not change (`crates/pipeline/src/save.rs`, `summarize_again`). At 8192 tokens the reply reserve can grow only to 4096 (half the context), so each run stopped with "the model's replies do not fit in its context". Only Transcribe Again, which deletes the summary checkpoints, could repair the note.

PR #17 changed four things:

1. `answer` and `is_server_problem` (`crates/summarize/src/context.rs:146-161`): a 5xx answer (but not 501), a 408 or a 429 on the **first** probe stops the run before a plan is saved, like a connect failure. A later probe that fails is still a miss, because a gateway can answer 503 for a route that it does not have.
2. `DEFAULT_LIMIT` is 128,000 (`context.rs:14`). The plan's KTD16 chose 8192; the user chose the large default. A server that gives no probe value and cuts a long prompt silently (Ollama's OpenAI route, for example) can now give a summary of part of the transcript with no error. The Settings footer tells the user to enter the model's limit for such a server.
3. `Plan::agrees_with` (`crates/pipeline/src/summarize.rs:117`, called at line 530): a saved plan is used again only while the Context limit setting gives its budget. A plan under an older default, under a learned limit, or under a limit that the user cleared is made again with its calls. A set limit keeps a plan under the same limit, whatever gave it, so its calls are not sent again.
4. A "prompt too long" rejection stops the run and asks the user to enter the limit in Settings > Summaries > Context limit (a lower limit when one is set). Longhand no longer learns and saves a smaller limit. `BudgetSource::Learned` stays only so that older saved plans still parse.

Tests: `a_server_error_from_a_probe_is_an_error_not_the_default` and `a_server_error_from_a_later_probe_or_a_route_not_implemented_is_a_miss` (`crates/summarize/tests/context.rs`), `regenerating_a_stub_planned_under_an_old_default_budget_plans_again` and `a_rejection_for_length_stops_the_summary_and_asks_for_the_context_limit` (`crates/pipeline/tests/runner.rs`), and `a_saved_plan_is_kept_only_while_the_context_limit_setting_gives_its_budget` (unit test in `summarize.rs`).

**Rule added:** when a fix stops a bad value from being saved, also repair the values that earlier versions already saved. Here, the check that decides whether a saved plan can be used again (`agrees_with`) did that repair.

## Related Issues

- [summary-regeneration-failed-with-no-working-folder.md](summary-regeneration-failed-with-no-working-folder.md): another Regenerate Summary failure, with a different cause.
- [retry-showed-failed-speaker-attempt-progress.md](retry-showed-failed-speaker-attempt-progress.md): state from a failed try carried into the next try.
- [resume-during-processing-deleted-live-text-proofs.md](resume-during-processing-deleted-live-text-proofs.md): discard checkpoints that cannot be trusted instead of resuming them.
- PR #15.
- PR #17.
