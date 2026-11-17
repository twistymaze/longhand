---
title: A stored cut plan overwrote the segment of a resumed recording
date: 2026-11-17
category: logic-errors
module: pipeline long-silence cut (crash-safe steps, Resume)
problem_type: logic_error
component: service_layer
symptoms:
  - "After a processing pause or a crash during the long-silence cut, a Resume and a Stop lose the segment row of the new recording."
  - "Assemble can join chunks of two recordings into one segment, because both use the same segment index."
root_cause: data_integrity
resolution_type: code_fix
severity: high
tags: ["long-silence-cut", "checkpoints", "crash-safety", "resume", "segment-index", "stale-plan"]
---

# A stored cut plan overwrote the segment of a resumed recording

## Problem

The long-silence cut (`crates/pipeline/src/cut/apply.rs`) runs in steps that can continue after a crash. Step 1 stores the plan as a checkpoint (`Step::Planned`). Step 2 changes the segment rows in one store transaction (`Step::Rows`). Later steps rename and delete files. The first version finished any stored plan that was not done, without a check.

Code review found the defect before release (PR twistymaze/longhand#99). No user lost audio.

## Root cause

A plan at `Step::Planned` describes the rows at the time of planning. Two events can leave it stored before step 2:

- a crash after step 1;
- a processing pause or cancel at `control.proceed()`, which `finish` calls before step 2.

The note is then stopped, and the user can Resume it. The Resume gives the new recording the index max + 1 (`Timeline::next_index`). The stale plan can give the same index to a split part, because `detect` numbers the parts from the lowest index up. `insert_segment` in `crates/session-core/src/jobs.rs` writes with `ON CONFLICT (note_id, idx) DO UPDATE`. So when the next run finished the stale plan, step 2 replaced the row of the new recording.

`cut::unfinished` was also true at `Step::Planned`. That made `recover::register_chunks` wait, so the chunks of the new recording got no row at all after a crash.

## Solution

A plan at `Step::Planned` changed nothing in the store or on disk. So the next run deletes it and plans again from the current rows. Only a plan at `Step::Rows` finishes first, and the run then continues to plan the cut of any segment that no cut checked. `unfinished` is true only at `Step::Rows`.

After step 2, a Resume is safe. Step 2 writes the rows of every part index in the same transaction that stores `Step::Rows`, so `next_index` is above every part index. `rename_parts` and the deletes match chunks by their original index and times, so they do not touch the chunks of the new recording.

The test `a_resume_after_a_plan_that_stopped_at_step_1_keeps_the_new_recording` covers the pause case and the crash case.

## Prevention

Before a stored multi-step plan continues, ask: "Can a user action change the input of this plan between the checkpoint and the first change?" If it can, and the plan has not changed anything yet, plan again. Do not continue it. Treat an upsert on a key that the plan computes, here `(note_id, idx)`, as a sign that a stale plan can overwrite data without an error.
