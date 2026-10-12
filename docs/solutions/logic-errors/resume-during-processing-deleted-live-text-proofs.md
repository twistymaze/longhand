---
title: A Resume during processing deleted the live-text proofs, and Retry failed forever
date: 2026-10-11
category: logic-errors
module: pipeline recognition (durable live identity, Retry)
problem_type: logic_error
component: service_layer
symptoms:
  - "A note stops with \"Stopped while finding speakers\" and \"Saved live text has no compatible audio and model identity. Choose Transcribe Again explicitly.\""
  - "Retry gives the same error every time."
  - "Transcribe Again is not available, because the note is not saved."
root_cause: data_integrity
resolution_type: code_fix
severity: high
tags: ["live-transcript", "source-proof", "resume", "retry", "checkpoints", "start-over", "recognition"]
---

# A Resume during processing deleted the live-text proofs, and Retry failed forever

## Problem

The cleanup in `prepare_fingerprints` deleted the source proofs of the live text. When the user resumed a note before its first processing run ended, the next run could not verify the live text of the earlier segments. Prepare then stopped with an error, and each Retry repeated the same steps. PR twistymaze/longhand#7 fixed the cause and added two recovery paths.

## Symptoms

- The note page shows "Stopped while finding speakers" and the error above.
- Retry fails again at once, with the same error.
- The error tells the user to choose Transcribe Again. The app offers Transcribe Again only for a saved note (`src/session/SessionPage.tsx:477`), so the user has no way out.

The session that found this also had microphone dropouts and a microphone change. These did not cause the failure. The microphone change only added a segment.

## What Didn't Work

- **Reading the error text as the cause.** "No compatible audio and model identity" points at the audio or the model. Both were correct. The live identity was not stamped because proofs were missing, not because the audio changed.
- **Looking only at `bind_live`.** `bind_live` correctly found that the proofs were missing. The deletion happened earlier, in the cleanup loop of an earlier run. To find it, read the `checkpoints` table of the stuck note: the live rows (`seg-0000-live-*`) were present, and `live-source-0000000N` was present only for the last segment.

## Solution

Three changes, in `crates/pipeline/src/recognition/durable.rs`, `crates/pipeline/src/runner.rs`, `crates/session-core/src/jobs.rs`, and `src/session/ProcessingStatus.tsx`:

1. **Keep the proofs.** Before the fix, the cleanup kept only keys that contain `-live-`. `live-source-*` and `live-snapshot-*` start with `live-` and have no leading dash, so the cleanup deleted them. The cleanup now always keeps them (`durable.rs:466`):

   ```rust
   // before
   let keep_live = live_matches && (key.contains("-live-") || key == LIVE_IDENTITY_KEY);
   // after
   let keep_proof = key.starts_with("live-source-") || key.starts_with("live-snapshot-");
   let keep_live = keep_proof
       || live_matches && (key.contains("-live-") || key == LIVE_IDENTITY_KEY);
   ```

2. **Replace live text that cannot be verified.** In `bind_live` (`durable.rs:257`), a segment with a missing proof or a proof that does not match now has its live rows, feature windows, and epochs removed, and a reason is recorded. Recognition then uses the recorded audio. The live identity is stamped when each live row that remains has a verified proof (`durable.rs:358`). Before the fix, a missing proof only set `valid = false`. The rows stayed, the identity was not stamped, and prepare failed (`durable.rs:417`).

3. **Start over after a repeated failure.** Retry records the failed stage (`runner.rs:417`). If the same stage fails again, `retry_starts_over` is true (`runner.rs:59`) for Transcribe, Diarize, Join, and Verify (`runner.rs:49`). The next Retry shows "Start Over?" (`ProcessingStatus.tsx:155`). On confirm, `Store::start_over` (`jobs.rs:986`) removes the checkpoints of Assemble through Verify and the recognition tables, and the run starts again from the recorded audio. Cancel leaves the note as it is.

The error texts in `durable.rs` no longer name Transcribe Again.

## Why This Works

A proof describes the recorded audio of a segment, not a result. It stays valid while the audio stays the same, so no cleanup of results may delete it. A Resume changes the audio identity of the note, and `bind_live` must then check each earlier segment again. Without the proofs, that check cannot pass.

Live text is a result that the app can always compute again from the recorded audio. So live text that cannot be verified is not a reason to stop. It is a reason to transcribe that span again. Change 2 applies this to any cause of a missing or changed proof, including causes that are not known yet.

Change 3 covers failures that changes 1 and 2 do not cover. When the same step fails twice from the same checkpoints, a third attempt from those checkpoints will also fail. Only a run that does not use them can give a different result.

## Prevention

- **Recompute derived state that cannot be verified. Do not stop on it.** When a check finds that a checkpoint or a cached result does not match its inputs, discard it and compute it again from the source data. Stop with an error only when the source data itself is missing or damaged.
- **Do not name an action that is not available in the current state.** The old error told the user to choose Transcribe Again, which a failed note cannot use. The [verify coverage failure](verify-stopped-on-detected-sound-without-words.md) had the same pattern: Retry ran the same work and failed again.
- **Name the keys that a cleanup keeps.** A rule such as `key.contains("-live-")` also selects or drops keys that later code adds. List the kept prefixes, or keep the inputs (proofs, snapshots) in a separate namespace from the results.
- **Test a Resume between two prepare calls.** `resume_before_processing_ends_keeps_live_text_of_earlier_segments` in `crates/pipeline/tests/recognition_stages.rs` calls `prepare` for the first segment, adds a segment, and calls `prepare` again. Before the fix it failed with the user's error. `live_text_without_a_source_proof_does_not_block_verified_live_text` checks the mixed case. `a_retry_that_fails_at_the_same_stage_offers_to_start_over_from_the_audio` in `crates/pipeline/tests/runner.rs` checks the start-over path.
- **Diagnose a stuck note from its checkpoints.** Copy `longhand.db` and its `-wal` file, then list the `checkpoints` rows of the session. Compare the live rows with the `live-source-*` rows for each segment.

## Related

- [Verify stopped a note on detected sound without words](verify-stopped-on-detected-sound-without-words.md): the same shape, a stage that failed the same way on each Retry.
- [The pause marker was missing because the recognition route writes its own transcript](pause-marker-missing-on-recognition-transcript-route.md): the same module, and saved-prefix reuse on Resume.
- [A Retry showed the percent of the failed speaker attempt](retry-showed-failed-speaker-attempt-progress.md): checkpoint state across Retry.
- [Before a PR, rebase when main has new code](../workflow-issues/rebase-before-pr-when-main-moved.md): an earlier error text that named an action that could not fix the state.
