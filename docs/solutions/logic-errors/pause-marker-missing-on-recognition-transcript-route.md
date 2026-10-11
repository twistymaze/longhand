---
title: The pause marker was missing because the recognition route writes its own transcript
date: 2026-10-10
category: logic-errors
module: pipeline transcript (join and recognition routes)
problem_type: logic_error
component: service_layer
symptoms:
  - "A finished note that was paused and continued shows no pause marker on its Transcript tab."
  - "transcript.md has no `--- pause hh:mm:ss ---` line and no `--- gap hh:mm:ss ---` line, although session.md has two segments and `ends_in_pause: true`."
  - "Every pause test passes, because the tests build the transcript only through join.rs."
root_cause: logic_error
resolution_type: code_fix
severity: high
tags: ["transcript", "recognition", "join", "pause", "segments", "route", "saved-prefix"]
---

# The pause marker was missing because the recognition route writes its own transcript

## Problem

Longhand writes `transcript.md` in two places. The pause feature (archived PR 51) added its pause line in only one of them. The first manual test of a signed build showed no pause marker. All unit tests passed.

## Symptoms

- On the Transcript tab, the text before and after a pause runs together, with no "Paused · N min" marker.
- `transcript.md` has no marker line between the segments. It has no gap line after a Stop and Resume either.
- In the store and in `session.md`, the pause flag is correct (`segments.ends_in_pause = 1` on the segment before the pause).
- The app log shows that the note reused the live transcript text ("live text covers all but 0 s of speech"), with no stage line for the join.

## What Didn't Work

- The plan, the units, and the first two code reviews all read the transcript from `crates/pipeline/src/join.rs` `build` (`join.rs:64`). The reviewers checked that code, and it was correct. Nobody asked whether the note in the test goes through it.
- The display code was also correct (`src/session/TranscriptTab.tsx` draws a dashed marker for a pause line). The first hypothesis was that only the screen dropped the marker, but `transcript.md` itself had no marker line.
- A check of the binary for the text `--- pause ` found it, so the build was not stale.

## Solution

The runner gives the Join stage to the recognition route when the note has a recognition snapshot (`crates/pipeline/src/recognition/stages.rs:158`, `Stage::Join => write_transcript(...)`). That route builds the transcript with `recognition::transcript` from the recognition rows only. Before the fix, the function did not get the segments, so it could not write a line between them.

The fix gives the segments to `recognition::transcript` (`crates/pipeline/src/recognition.rs:199`; the caller is `stages.rs:629`). The function puts a boundary line before the first row of each later segment, and puts the lines of segments with no text after them at the end. One function now makes the line for both routes:

```rust
// crates/pipeline/src/join.rs:174
pub(crate) fn boundary_line(before: &Segment, after: &Segment) -> TranscriptLine {
    let duration_ms = after.offset_ms.saturating_sub(before.end_ms());
    if before.ends_in_pause {
        TranscriptLine::Pause { duration_ms }
    } else {
        TranscriptLine::Gap { duration_ms }
    }
}
```

`join::build` and `recognition::transcript` (`recognition.rs:212`) both call it. The test `a_recognition_transcript_has_a_line_between_segments` (`crates/pipeline/tests/recognition_routes.rs:60`) covers a pause, a Stop/Resume gap, and a pause with no text after it on the recognition route.

A review of the other code that reads segments, done after the fix, found two more defects of the same kind:

- **Saved-prefix reuse.** `crates/pipeline/src/recognition/saved_prefix.rs` compared the segments in the saved `session.md` with the segments in the store with `Segment`'s `PartialEq`. That comparison now included `ends_in_pause`. A note saved while paused has the flag in `session.md`, and Resume clears it in the store, so the reuse check failed. The whole note was recognized again on Resume. The fix compares the segments without the flag (`same_segments`, `saved_prefix.rs:90`). The test is `saved_prefix_accepts_a_note_saved_while_paused` (`saved_prefix.rs:541`).
- **A dropped segment at Pause.** Pause flagged the last kept segment of the note. When the segment that the pause closed was under a second, the app dropped it, so the flag went to an earlier segment. That segment could end at a Stop, and the boundary then showed as a pause. `close_segment` now returns whether it kept the segment (`src-tauri/src/managers/capture.rs:3290`), and Pause flags only a kept segment (`capture.rs:1661`).

## Why This Works

The pause marker comes from the segment data, and both routes now read the segments. The recognition route was not wrong before the pause feature by design. It was written when no line between segments carried any meaning that the summary or the user needed, so it left out the gap line too. A feature that adds meaning to a segment boundary must reach every writer of `transcript.md`. The shared `boundary_line` makes the rule one decision, not two copies that can differ.

## Prevention

- Before you plan or review a change to the transcript, list every writer of `transcript.md`: `join::run` (`crates/pipeline/src/join.rs`) and `write_transcript` in `crates/pipeline/src/recognition/stages.rs`. A note with a recognition snapshot goes through the recognition route (`crates/pipeline/src/runner.rs:1330`, `execute_prepared`). Both test notes in this session took that route. A test through `join::build` alone does not cover the notes that users make.
- When you add a field to `Segment`, search for `Segment` comparisons and `==` on segment lists (for example, the saved-prefix check). A new field changes every derived `PartialEq`.
- When an action flags "the last kept segment", check the case where the segment that the action closes is dropped (under one second).
- In the manual test, open `transcript.md` of the test note as well as the screen. Also check the app log for the route ("live text covers…" means the recognition route).

## Related Issues

- `docs/solutions/logic-errors/verify-stopped-on-detected-sound-without-words.md`: the same two-route shape in the verify stage, fixed with one shared function for both routes.
- `docs/solutions/logic-errors/granite-word-timing-misclassified-as-absent.md`: another case where a recognition-route capability needed a check per route.
- Archived PR 51 (pause and continue a recording note).
