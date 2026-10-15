---
title: One processing permit made the audio stages of a new note wait for slow summary requests
date: 2026-10-15
category: performance-issues
module: Processing runner and summary queue (crates/pipeline/src/runner.rs, crates/pipeline/src/summary_queue.rs, crates/summarize/src/client.rs)
problem_type: performance_issue
component: service_layer
severity: high
symptoms:
  - A new note waited 7.5 minutes after Stop before transcription started, behind three Regenerate Summary runs
  - Two Regenerate Summary requests for one note both ran
  - A reply that the LiteLLM proxy retried came 1 s after Longhand's 600 s response deadline, so the summary was lost
  - The note page showed the first step of a waiting note as running
root_cause: concurrency
resolution_type: code_fix
framework_version: rust 1.94.0
related_components:
  - background_job
  - frontend
tags:
  - summary-queue
  - processing-permit
  - priority-queue
  - heartbeat
  - llm-timeout
  - regenerate-summary
  - title-job
---

# One processing permit made the audio stages of a new note wait for slow summary requests

## Problem

The runner had one processing permit (skeleton plan R20c, KTD20). Every stage of every note took it, also the Summarize stage, Regenerate Summary and title jobs. A slow summary endpoint therefore stopped the local audio processing of new notes.

## Symptoms

- On 2026-10-14, a new meeting waited 7.5 minutes after Stop. Three queued regenerations had the permit before it. One Azure request took 340 s.
- Another request stalled 600 s at the LiteLLM proxy. LiteLLM sent it again and got the reply 1 s after Longhand closed the connection at its 600 s response deadline. Longhand lost the reply.
- Two Regenerate Summary requests for one note both ran.
- Title jobs also waited behind the new note.
- A note that waited for the permit showed its first step with the running dot.

## What Didn't Work

- **A tokio `Semaphore` for the summary work.** A semaphore gives permits in FIFO order. A new note must go before a regeneration, and a regeneration before a title job. A FIFO queue cannot do that.
- **Starting title jobs only from the run loop.** In the first version, the run loop started the queued title jobs between notes. While a note was in its audio stages, the loop waited for that note, and the summary queue stayed idle with title jobs in it. The correctness review and the Codex cross-model review found this before merge.
- **A heartbeat for each stage.** Before the change, only a running stage kept the claim alive. When a note waits for its summary turn, no stage runs. After `CLAIM_STALE_AFTER` (30 s, `crates/session-core/src/jobs.rs:398`), another instance could take the claim of a note that is still alive.
- **The job row's stage as "the running stage".** The job row names the next stage to run, not a stage that runs now. The UI read it as a running stage, so a note that waited for the permit looked active.
- **Reading the lease count in a test right after `run()` returns.** A title job runs in its own task. On CI it still held its power lease when the run loop ended, and the assert failed only on CI (`a_recording_does_not_pause_a_summary_but_keeps_the_next_note_waiting`, `crates/pipeline/tests/runner.rs`).

## Solution

PR #20.

1. **Two queues.** The local permit (`Runner::local_permit`, `crates/pipeline/src/runner.rs:1600`) now covers only the audio stages: assemble, transcribe, speakers, verify and join. The Summarize and Save stages of a note, Regenerate Summary and title jobs take turns in the new summary queue (`crates/pipeline/src/summary_queue.rs`), one job at a time.
2. **Priority with cancel-safe waiters.** `Class` (`summary_queue.rs:18`) has three classes: `NewNote`, `Regeneration`, `Title`. The queue is FIFO in each class. A `Waiter` (`summary_queue.rs:95`) removes its ticket when it is dropped before its turn, so a cancelled wait keeps no place. A `Turn` (`summary_queue.rs:82`) frees the turn on drop.
3. **One task for each note.** `run` starts a task for each claimed note. `NoteTask` (`runner.rs:2284`) owns the heartbeat from dequeue to the end of the task, so a note that waits for its summary turn keeps its claim. Cancel tokens are a map of child tokens of the shutdown token, one for each note, so cancel, resume, delete and quit reach a note in either queue.
4. **Title jobs start when queued.** `spawn_titles` (`runner.rs:875`) starts one task that runs the queued title jobs. Each title job waits for `Class::Title` in the summary queue (`runner.rs:901`), so the run loop does not wait for it.
5. **The runner reports the wait.** `RunnerEvent::Waits` tells what a note waits for: the recording, another note, its summary turn or the model. A step shows the running dot only after `StageStarted`.
6. **A longer reply wait.** `Limits::completion` (`crates/summarize/src/client.rs:47`) waits 30 minutes for the response headers and 32 minutes in total (was 600 s). The client still sends a request again only after an `is_connect()` error (`client.rs:508`).
7. **Request log.** Each request writes a start line and an end line. An `EndGuard` (`client.rs:158`) writes the end line with "cancelled" when the request future is dropped.
8. **A second regeneration is refused.** `Claims::regenerate` returns a `RegenerateRefusal` when a regeneration of the note waits or runs.

## Why This Works

- The one permit exists because the app loads one recognition engine at a time. Summary requests do not use that engine, so they do not need the permit.
- Save, Regenerate Summary and title jobs all write the note folder. The code in `crates/pipeline/src/save.rs` expects that only one of them writes at a time (`save.rs:1254`: "summary queue, so no other regeneration, Save or title job runs"). One summary queue keeps that rule, and the audio stages run in parallel with it.
- The class queue puts the work that the user waits for first. A new note after Stop goes before an old note's regeneration, and an optional title goes last.
- A heartbeat for the full note task keeps a claim alive for the full time that the note waits.
- In this incident, the LiteLLM proxy timed out one attempt at 600 s and tried again. A deadline of 600 s on Longhand's side ends the connection before the second attempt can reply. A 30-minute deadline covers a slow but working model. It helps fully only when the endpoint's own timeout is also longer.

## Prevention

- **Give a shared permit only to the work that uses the shared resource.** Name the resource in a comment at the permit. Network work that waits for a remote server must not hold a permit for a local resource.
- **Use a class queue, not a semaphore, when the order must follow priority.** Make the waiter cancel-safe: dropping the wait must remove its place.
- **When work moves from one loop to many tasks, move the per-task parts too.** Check the heartbeat, the cancel token, the "is working" state and the wait marks. Each must cover the full task, also the time that the task waits.
- **Do not show "running" from the job row.** The row names the next stage. Show a running step only after the runner sends `StageStarted`. [The speaker retry doc](../logic-errors/retry-showed-failed-speaker-attempt-progress.md) has the same window for a retry.
- **Set the reply deadline above the proxy's timeout plus one retry.** Keep the rule that a request is sent again only when the error proves that the endpoint did not get it (see [the network wake doc](../logic-errors/summary-in-a-wake-with-no-network-kept-the-default-budget.md)).
- **Tests** in `crates/pipeline/tests/runner.rs` cover:
  - a new note that transcribes while another note's request is held;
  - the priority of the summary queue;
  - a recording during a summary;
  - resume, cancel and quit with notes in both queues;
  - the heartbeat during a long wait;
  - a second regeneration;
  - title jobs during a recording and during another note's audio stages;
  - the reported wait.
- **Test traps.**
  - A test that holds a gated mock reply hangs when the code under test is wrong. It does not fail. Run such tests with a time limit.
  - When `perl -e 'alarm N; exec @ARGV'` stops `cargo test`, the test binary can continue to run. Stop it with `pkill -f deps/runner-` (auto memory [claude]).
  - After `run()` returns, a title task can still hold its lease. Wait for the count with `until(...)`, do not assert it at once.
