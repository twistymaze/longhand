---
title: A Retry showed the percent of the failed speaker attempt before the new attempt started
date: 2026-09-29
category: logic-errors
module: Speaker progress checkpoint (crates/pipeline/src/progress.rs, crates/pipeline/src/runner.rs run_stage, src/processing/describe.ts speakerMeter, src/session/useSessions.ts)
problem_type: logic_error
component: service_layer
severity: medium
symptoms:
  - "After Retry of a failed note, Finding speakers showed the old percent (for example 80%) and then dropped to the lower percent of the new attempt"
  - "progress::snapshot returned the saved percent of the failed attempt while the job waited for the permit and prepared audio"
  - "speakerMeter ignored live events of another attempt but used saved progress unconditionally"
root_cause: data_integrity
resolution_type: code_fix
framework_version: rust 1.94.0
related_components:
  - frontend
  - background_job
tags: [speaker-progress, checkpoint, retry, diarize, snapshot, attempt-id, stale-state, cross-model-review]
---

# A Retry showed the percent of the failed speaker attempt before the new attempt started

## Problem

The speaker stage (`Stage::Diarize`) saves its progress as a checkpoint under `SPEAKER_KEY` (`"speaker-progress-v1"`, `crates/pipeline/src/progress.rs:86`). The saved value is a `SpeakerProgress`: an attempt id, a percent, and a `measuring` flag (`crates/pipeline/src/progress.rs:99-105`). `speaker_sink` writes it on each report and sends the same value as a `SpeakerProgress` event (`crates/pipeline/src/runner.rs:1160-1186`). The attempt id is a nanosecond timestamp plus a counter (`crates/pipeline/src/runner.rs:1162-1166`).

The meter has one rule: the percent does not go back (`src/processing/describe.ts:108-111`). A new attempt of the speaker stage broke this rule. For a short time, the view showed the percent of the earlier attempt. It then dropped to the lower percent of the new attempt.

The defect was in PR twistymaze/longhand#23 before merge. The same PR fixed it.

## Symptoms

- After Retry on a failed note, or after a new process reclaims an interrupted job, the processing view showed an old speaker percent (for example 80%). A moment later it showed the new attempt's value (for example 10%).
- No error occurred. Tests for one attempt passed. The defect showed only across two attempts on the same note.

The cause is a window between two steps:

1. Retry keeps the job's place and its checkpoints (`crates/pipeline/src/runner.rs:331-332`). `jobs.stage` changes only in `set_stage` (`crates/session-core/src/jobs.rs:1460-1466`), so the job still records `Diarize` as its current stage.
2. `dequeue_session` applies `Event::Dequeued` (`crates/session-core/src/jobs.rs:1428`), and the note becomes `Processing` (`crates/session-core/src/state.rs:249-251`).
3. `run_stage` then waits for the processing permit and prepares audio (`crates/pipeline/src/runner.rs:886-922`). Only after that does it call `set_stage` and send `StageStarted` (`crates/pipeline/src/runner.rs:948-952`).

During step 3, all three conditions in `progress::snapshot` were true: the note is `Processing`, the current stage is `Diarize`, and `Diarize` is not done (`crates/pipeline/src/progress.rs:48-57`). So `snapshot` returned the saved value of the failed attempt.

The frontend then made the error visible:

- `speakerMeter` used the saved progress unconditionally. It dropped a live event whose attempt was different from the saved attempt.
- `useSessions` re-reads the note detail at most once each `REREAD_MS` (1000 ms, `src/session/useSessions.ts:29`), with no trailing re-read (`src/session/useSessions.ts:85`, `src/session/useSessions.ts:91`). The old saved value could stay on screen until the next re-read, and then the percent went down.

## What Didn't Work

- **An attempt id on the saved value alone.** The id lets the view compare two values. But before the first live event of the new attempt, the view has only the saved value. It cannot know that this value belongs to an earlier attempt. The old `speakerMeter` also trusted the saved value over the live event, so when the two ids were different, it discarded the correct one.
- **Replacement by the first report.** The comment on `speaker_sink` says that the first report replaces the earlier progress (`crates/pipeline/src/runner.rs:1158-1159`). This is true, but the first report comes after the permit wait and the audio preparation. The window is before that report.
- **Same-family review.** Two reviewers from the same model family saw the window and judged it too short to matter. An independent Codex review found the defect, and a validator confirmed it. The permit wait has no upper limit: another note can hold the permit for a full processing run, so the window can be long.

## Solution

The fix has three parts, one in the runner and two in the frontend.

**1. Delete the saved speaker progress at the start of every speaker stage run, before any wait.**

Before (`crates/pipeline/src/runner.rs`, start of `run_stage`):

```rust
    ) -> Result<(), StageError> {
        let _permit = self
            .permit
            .clone()
            .acquire_owned()
            .await
```

After (`crates/pipeline/src/runner.rs:881-890`):

```rust
    ) -> Result<(), StageError> {
        // A new attempt does not show the progress of an earlier attempt.
        if stage == Stage::Diarize {
            lock(&self.env.store).delete_checkpoint(id, stage, crate::progress::SPEAKER_KEY)?;
        }
        let _permit = self
            .permit
            .clone()
            .acquire_owned()
            .await
```

`delete_checkpoint` removes one key of one stage (`crates/session-core/src/jobs.rs:1595-1601`). Other `Diarize` checkpoints, such as saved segment results, stay.

**2. Treat the live event as the current attempt in `speakerMeter`.**

Before (`src/processing/describe.ts`):

```ts
  const saved = detail.speakers;
  const current = live && (!saved || live.attempt === saved.attempt) ? live : undefined;
```

After (`src/processing/describe.ts:115-116`):

```ts
  const current = live;
  const saved = detail.speakers && (!live || live.attempt === detail.speakers.attempt) ? detail.speakers : null;
```

The saved value counts only when no live event exists or when both have the same attempt id.

**3. Forget speaker events on every other processing update.**

Before, `useSessions` forgot speaker events only on `stage_started`. After, it forgets them on any update that is not `progress` or `speaker_progress`, which includes the end of a stage (`src/session/useSessions.ts:94-100`):

```ts
        // Any other update ends the speaker attempt, so its events no longer count.
        setSpeakers(forget);
```

## Why This Works

- The runner deletes the saved value before the permit wait. `snapshot` can then return only a value that the current attempt wrote (`crates/pipeline/src/progress.rs:48-57`, `crates/pipeline/src/progress.rs:202-208`). The window between `Dequeued` and `StageStarted` shows no speaker percent, not an old one.
- The live event always comes from the attempt that runs now. If a saved value from another attempt reaches the view (for example from a detail read that started before the delete), `speakerMeter` ignores it. The percent then goes up only within one attempt.
- When a stage ends, the view forgets the speaker events of that attempt. A later attempt does not start with an old live event in memory.

## Prevention

- Clear each progress value that is saved per attempt when a new attempt begins, before any wait (permit, lock, preparation, or network). Do not rely on the first report of the new attempt to overwrite it.
- Check every state that a reader can see between "the job is running" and "the stage started". If a reader uses saved state in that window, the saved state must belong to the current attempt or be absent.
- In a view, prefer the newest live source. Use saved state only when it has the same attempt id as the live source, or when no live source exists.
- Get an independent review for timing windows. Do not accept "the window is short" when the window contains a wait with no upper limit.

The runner test records the saved attempt at each `StageStarted { stage: Diarize }` event in the events sink (`crates/pipeline/tests/runner.rs:494-515`). The test cancels a first attempt that saved progress, reclaims the job in a new process, and runs it again (`crates/pipeline/tests/runner.rs:1050-1097`). It asserts that the new attempt starts with no saved progress:

```rust
    // The new attempt starts without the earlier attempt's saved progress.
    assert_eq!(*second.script.saved_at_start.lock().unwrap(), vec![None]);
```

(`crates/pipeline/tests/runner.rs:1090-1091`). Without the fix in `run_stage`, this assertion failed with `left: [Some("...-1")]`, the attempt id of the cancelled attempt.

The frontend test "saved progress of an earlier attempt does not count" (`src/processing/describe.test.ts:75-77`) asserts that a live event of attempt `b` at 10% wins over saved progress of attempt `a` at 80%:

```ts
expect(speakerMeter(running(progress("a", 80, true)), progress("b", 10, true)))
  .toEqual({ percent: 10, measuring: true });
```

Use the same pattern for any other value that is saved per attempt: record the saved value at the start event of the new attempt, and assert that it is absent.

## Related Issues

- PR twistymaze/longhand#23 (speaker progress; this defect and its fix).
- [save-state-machine-crash-window-recovery.md](save-state-machine-crash-window-recovery.md): durable per-step state across a resume. A resume there also deletes the checkpoints that must not carry over.
- [synced-summary-did-not-refresh-open-note.md](synced-summary-did-not-refresh-open-note.md): another stale view that `useSessions` fed.
