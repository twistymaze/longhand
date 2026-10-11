---
title: A crash between a save step and its record stranded or duplicated a completed session
date: 2026-09-24
category: logic-errors
module: Session save to the destination folder (crates/session-save state machine and the crates/pipeline save stage)
problem_type: logic_error
component: service_layer
severity: high
symptoms:
  - A same-volume session whose final rename completed before a crash was reported failed (SaveError::Lost) although its complete folder sat in the destination under its final name
  - A cross-volume session whose final rename completed before its Saved record was copied and saved a second time, under <name>-2
  - A relaunch after the Saved state (or set_home_path) was recorded routed to update_home and failed with "the working copy ... is gone", leaving the marker file in the saved folder
  - A cross-volume save resumed at Saved was logged and reported as moved rather than copied
root_cause: data_integrity
resolution_type: code_fix
framework_version: rust 1.94.0
related_components:
  - database
  - background_job
  - testing_framework
tags:
  - crash-recovery
  - state-machine
  - step-then-record
  - idempotent-steps
  - marker-file
  - atomic-rename
  - cross-volume-copy
  - session-save
---

# A crash between a save step and its record stranded or duplicated a completed session

## Problem

Save moves a finished session from the working location into the user's destination folder. The rule is that exactly one location holds a complete copy at every moment and a session is never lost (R22a/R22b). The work is a step-then-record state machine. `session_save::step` (`crates/session-save/src/lib.rs:155`) does one filesystem step and returns the next `SaveState` (`lib.rs:41-64`: `Working -> Copying -> Staged{staging, copied} -> Saved{path, working_retained} -> Done{path}`). `pipeline::save::run` writes each returned state to SQLite (`checkpoint::put`, `crates/pipeline/src/save.rs:175`) before it takes the next step.

So every step has a window where its effect is on disk but the durable record still names the previous state. After a crash in that window, the relaunch calls `step(previous_state)` against a filesystem that already shows the effect. Two defects in that window survived the original implementation (U11/U18), its crash tests and a plan review. A full-branch code review with a cross-model pass found both. Both are fixed in archived PR 1 (merged to main).

1. **The Staged step destroyed its own evidence.** It deleted the ownership marker, then renamed staging to the final name. A crash after the rename left no way to recognise the renamed folder.
2. **A side record bypassed the state machine.** `run` routed any session with `home_path` set to `update_home`, the path that adds a resumed session's new files to its existing folder. It did this without checking whether a first save was still mid-flight.

## Symptoms

Bug 1: crash, or a `fsync::sync_dir(root)` error, after `rename_new(staging, final)` but before `Saved` was recorded. The recorded state stays `Staged`, and staging no longer exists.

- **Same volume (`copied: false`):** `step(Staged)` found no staging and returned `SaveError::Lost("the staged copy at … is gone")`. `run` mapped that to `StageError::Failed` (`save.rs:158`). The session was reported failed, but its complete folder sat in the destination under its final name.
- **Cross volume (`copied: true`, working copy still present):** `step(Staged)` returned `Working` (`lib.rs:228-230`). The loop then wrote a new marker, renamed (got `CrossesDevices`), copied and staged again. The final rename found `NAME` taken and saved a second copy as `NAME-2` (`naming::candidate`, `crates/session-save/src/naming.rs:11-17`). The result was two folders for one session.

Bug 2: relaunch after `set_home_path` (`save.rs:173`) when the recorded state is `Saved`, or `Staged` if the crash fell between `set_home_path` and `checkpoint::put`. `run` went to `update_home`. That code saw a state other than `Done` and required the working copy (`save.rs:284-291`). On the same volume the working copy had been renamed away, so it failed with "the working copy at … is gone before … was updated". A completed session was stranded as failed, and its marker was left in the saved folder.

## What Didn't Work

- **Pre-existing crash tests** (`crates/session-save/tests/save.rs`) covered three cases:
  - The Working step's lost record: `a_rename_that_happened_before_its_record_is_recognised_by_its_marker`, `save.rs:264-277`.
  - A partial copy: `a_partial_copy_left_by_a_crash_is_this_sessions_and_is_redone`, `:235`.
  - A kill *before* the final rename, with `Staged` recorded and its effect not yet done: `a_kill_between_the_verified_copy_and_the_final_rename_keeps_both_copies_and_resumes`, `:173-199`.

  None re-ran `Staged` after its effect. Only one of the four boundaries had a lost-record test, so the pattern that proved the Working step correct was never applied to the others.
- **Pipeline tests** always reached the save stage through the runner from a clean start. None hand-built a mid-flight recorded state with `home_path` already set, so the routing check at the top of `run` never met that combination.
- **The marker's original contract** was "tell our staging dir apart from someone else's" (pre-fix doc comment on `MARKER`). It was scoped to the *staging* name, so deleting it just before staging stopped existing looked like tidy cleanup, not destroyed evidence.

- **The routing came later than the state machine (session history).** The home-folder update path (`update_home`, routed on `home_path`) arrived with U24, resuming any session with no time limit, a day after U11 built the state machine. Its runner test covered complete, resume, record, reprocess, update; nothing considered a session whose first save was interrupted. A new flow that reads a side record written by an existing state machine is where to look for this class.
- **The plan review had already caught one of these (session history).** A pre-implementation adversarial and cross-model review of the plan found the launch sweep deleting staging for sessions awaiting a destination on the grounds that "the working copy is authoritative", which was false once the first rename had moved it: the sweep could destroy the only copy. That is what produced the exactly-one-complete-copy rule and the sweep's "only sessions recorded as completed" restriction (`lib.rs:118-144`). Same shape, one step earlier; the lesson was applied to the sweep but not generalised to every step.

## Solution

**Bug 1: keep the marker through the final rename, search for it when staging is missing, and remove it one step later.**

Before (Staged step):

```rust
SaveState::Staged { staging, copied } => {
    if !exists(staging) {
        if *copied && exists(job.working) {
            return Ok(SaveState::Working);       // re-copies -> "NAME-2"
        }
        return Err(SaveError::Lost(/* "the staged copy … is gone" */));
    }
    // … drop LEFT_BEHIND …
    match std::fs::remove_file(staging.join(MARKER)) { /* evidence gone */ }
    fsync::sync_dir(staging)?;
    let root = staging.parent().unwrap_or(job.destination);
    for attempt in 1..=MAX_NAMES { /* rename_new(staging, candidate) */ }
```

After (`lib.rs:214-235`, `:269-281`):

```rust
SaveState::Staged { staging, copied } => {
    let root = staging.parent().unwrap_or(job.destination);
    if !exists(staging) {
        // The final rename happened and its record did not: the
        // marker, kept through the rename, finds the folder.
        if let Some(path) = (1..=MAX_NAMES)
            .map(|attempt| root.join(naming::candidate(job.folder_name, attempt)))
            .find(|path| is_ours(path, job.session_id))
        {
            return Ok(SaveState::Saved { path, working_retained: *copied });
        }
        if *copied && exists(job.working) { return Ok(SaveState::Working); }
        return Err(SaveError::Lost(/* … */));
    }
    // … drop LEFT_BEHIND, sync, rename loop (marker travels with the folder) …
}
SaveState::Saved { path, working_retained } => {
    match std::fs::remove_file(path.join(MARKER)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    fsync::sync_dir(path)?;
    if *working_retained && exists(job.working) { std::fs::remove_dir_all(job.working)?; }
    Ok(SaveState::Done { path: path.clone() })
}
```

Details that matter:

- The scan walks the same candidate sequence the rename loop uses (`lib.rs:219-221` vs `:247-248`) and the same `root`. Whatever name the rename won, the scan finds it.
- `is_ours` (`lib.rs:146-148`) compares the marker's contents with the session id. A same-named folder from another session, possibly another computer syncing into the same OneDrive folder, has no marker because its Saved step removed it, or has a different id. It never matches.
- The scan runs *before* the `copied && working exists -> Working` fallback. That ordering is what prevents the `-2` duplicate.
- `MARKER`'s doc comment (`lib.rs:30-34`) now states the lifetime contract: written before the working dir moves, kept through the final rename, removed in the `Saved` step.
- The rename loop still calls `fsync::sync_dir(root)?` after a successful rename (`lib.rs:251`). An error there still fails the step, but the retry now recovers through the scan.

**Bug 2: consult the authoritative recorded state before the side record** (`save.rs:87-103`):

```rust
let recorded = checkpoint::get::<SaveState>(store, session_id, Stage::Save, STATE)?;
let first_save_in_progress = matches!(
    recorded,
    Some(SaveState::Copying { .. } | SaveState::Staged { .. } | SaveState::Saved { .. })
);
if let Some(home) = home && !first_save_in_progress {
    return update_home(store, session_id, working, segments, &home, control);
}
```

`Working` and `Done` are deliberately left out of that list:

- A resumed session has no save state, because resuming a completed session deletes the `Stage::Save` checkpoints (`crates/session-core/src/jobs.rs:669-681`). `update_home`'s own `claim` then records `Working` (`save.rs:280`, `:349-354`), so `Working` plus `home_path` means an update in progress.
- `Done` plus `home_path` means only the bookkeeping remains. `update_home` handles that: `finished = true` (`save.rs:276-277`), so it skips the copy and goes straight to cleanup.

The same commit also seeds `copied` from a recorded `Saved { working_retained: true }` (`save.rs:134-140`). Before, it started `false` on relaunch, so the returned `Saved.copied` was wrong for a cross-volume save resumed at `Saved`.

Tests:

- `a_final_rename_that_happened_before_its_record_is_found_by_its_marker` (`crates/session-save/tests/save.rs:201-232`) runs the same-volume and cross-volume cases. It asserts that re-running `Staged` equals the first result, then that the run finishes with no `NAME-2`, no marker, and no working copy.
- `a_relaunch_after_the_final_rename_is_recorded_finishes_the_first_save` (`crates/pipeline/tests/runner.rs:1305-1359`) drives the save to `Saved` by hand, calls `set_home_path`, records `Saved`, and relaunches. It expects `Outcome::Finished` with the marker gone. It fails without the routing fix.

## Why This Works

With both fixes, every step, re-run from the previous state after its effect, returns the state its first run returned:

| Boundary | Effect on disk | Re-run from previous state recognises it by |
|---|---|---|
| Working -> Staged (same volume) | working renamed to staging | working gone and staging carries our marker (`lib.rs:159-166`) |
| Working -> Copying | marker written into working, nothing moved | no effect to detect; rewriting the marker is harmless, staging still absent |
| Copying -> Staged{copied:true} | complete staging | not recognised; staging is treated as partial, removed and copied again (`lib.rs:200-212`). Correct but slow: the working copy is still whole |
| Staged -> Saved | staging renamed to final name | staging gone and a candidate name carries our marker (`lib.rs:216-227`) |
| Saved -> Done | marker removed, retained working copy removed | `NotFound` ignored, `exists` guard (`lib.rs:273-280`) |

The general failure class has four parts:

1. **Step-then-record.** Filesystem effect first, durable record second. Recording first would claim an effect that may not have happened, so the window can't be closed. It can only be made recoverable.
2. **The effect-without-record window.** Each step must be idempotent *with respect to its own completed effect*, detected from disk alone. Idempotence against a clean start isn't enough.
3. **Evidence destroyed before the record that no longer needs it.** Anything recognition relies on (a marker, a name, a temp file) has to outlive the record of the step it proves. Delete it only in a later step, after that record is written. Bug 1 deleted the marker *inside* the step it had to prove.
4. **A side record used to bypass a mid-flight state machine.** `home_path` is written before the `Saved` state record (`save.rs:173` then `:175`) and doubles as "this session has a home". A router that trusts the side record sees "has a home" before the machine has finished giving it one. Branch on the authoritative state first. Treat side records as payload, not as routing input, until the state says the flow is complete.

## Prevention

**Checklist for adding or changing a step in `session_save::step`, or any step-then-record flow here:**

1. Write down the step's effect and its window: effect done, record not written. If the step has several effects, each partial point is its own window.
2. Say exactly how `step(previous_state)` recognises the completed effect from disk alone. Make it return the *same* next state, not an error and not an earlier state.
3. List the evidence that recognition reads. Confirm nothing in *this* step deletes or renames it away. Move its cleanup into the next step, and make that cleanup tolerate `NotFound`.
4. If recognition searches (like the candidate scan), use the same generator and root as the effect, so every outcome the effect can produce is covered.
5. Order the recovery branches so "effect already done" is checked before any fallback that redoes work. Redoing a non-idempotent effect is how duplicates happen.
6. Check every non-state record written alongside the state (`set_home_path`, `Event::*`, checkpoints under other keys), and every reader that branches on it. Each reader must check the recorded `SaveState` first.
7. Check what an error *after* the effect does, such as `sync_dir` failing post-rename. The caller keeps the previous state, so the retry must take the recognition path.
8. Update the `MARKER` doc comment or the `SaveState` variant docs when a lifetime or meaning changes. The contract belongs next to the code.
9. Add the two tests below: one per boundary, both volume modes.

**Other step-then-record flows in this repo:**

`crates/session-save/src/home_write.rs` isn't a state machine. It provides the atomic write primitives. `fill_atomic` (`home_write.rs:49-71`) writes an unpredictable create-new temp file, calls `sync_all`, renames over the target, then syncs the directory. The target is either the old file or the complete new one. That property is what the callers' idempotence relies on.

The flows built on it look consistent with the discipline for the cases read here. They have no dedicated lost-record tests, so treat that as a reading, not a proof:

- **`update_home`** (`save.rs:262-337`): every write can be repeated:
  - Audio already at the target is skipped (`save.rs:295-296`). That is safe only because `copy_atomic` never leaves a partial file under the final name.
  - Documents are overwritten (`:309-312`).
  - `Done` is recorded only after all writes (`:313-316`).
  - The working directory, the evidence and source for a re-run, is removed only after `Done` is recorded (`:318-320`).
- **`regenerate_summary`** (`save.rs:454-485`): writes `summary.md` and then marks Summarize complete (`:479-480`). A crash between them re-runs summarization and rewrites the file. That wastes work but stays correct.

Apply the checklist to anything new here, such as moving `session.md` into or out of a home folder, or a multi-file replacement.

**Test template: re-run the step from the previous state after its effect.**

```rust
#[test]
fn step_x_that_happened_before_its_record_is_recognised() {
    for cross in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (work, dest) = (working(root.path()), destination(root.path()));
        let job = Job { session_id: "s1", working: &work, destination: &dest, folder_name: NAME };
        let fs = CrossDevice { cross: Cell::new(cross) };

        // Drive to the state just before the boundary under test.
        let mut before = SaveState::Working;
        while !is_the_state_before_x(&before) {
            before = step(&before, &job, &fs).unwrap();
        }

        // Take the step once: its effect is now on disk.
        let after = step(&before, &job, &fs).unwrap();

        // The record of `after` is lost: re-run from `before`.
        let again = step(&before, &job, &RealFs).unwrap();
        assert_eq!(again, after, "cross-volume: {cross}");

        // Finish from the recovered state; assert the invariants, not just Done.
        let states = run(&job, &RealFs, again);
        assert!(matches!(states.last(), Some(SaveState::Done { .. })));
        assert_eq!(saved_files(&dest.join(NAME)), EXPECTED);   // one complete copy
        assert!(!dest.join(format!("{NAME}-2")).exists());      // no duplicate
        assert!(!dest.join(NAME).join(MARKER).exists());        // evidence cleaned up
        assert!(!work.exists());                                // working copy gone
    }
}
```

The helpers (`working`, `destination`, `CrossDevice`, `run`, `saved_files`, `EXPECTED`) already exist in `crates/session-save/tests/save.rs:12-91`. `is_the_state_before_x` stands for the matcher for the boundary under test.

For the routing class, add a pipeline test in the shape of `a_relaunch_after_the_final_rename_is_recorded_finishes_the_first_save`. For each recorded mid-flight `SaveState` (`Copying`, `Staged`, `Saved`), write that state and the side records that precede it, in the same order `run` writes them. Then call `runner.run_next()` and assert `Outcome::Finished` with the folder complete. Also cover the case where the side record is written but its state is not: the previous state is still recorded with the new side record already set. For example, `Staged` recorded with `home_path` already set is the crash between `save.rs:173` and `:175`.

## Related Issues

- Archived PR 1: the walking skeleton, including this fix.
- `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`: R22b and R22d (the exactly-one-complete-copy and sweep rules, lines 163-165), KTD17 (line 321), U11's save approach (lines 932-952), and the resume approach (line 1001) whose "a home folder is updated in place" rule is the unqualified form of bug 2's routing. Historical record; its U11 note on the marker (line 952) predates the marker being kept through the final rename.
