---
title: A resumed note seeded from transcript.md lost its silences and paragraphs, and could fail its checks
date: 2026-10-17
category: logic-errors
module: Resume from transcript (crates/pipeline/src/transcribe.rs seed_from_transcript, crates/pipeline/src/recognition/stages.rs verify)
problem_type: logic_error
component: service_layer
severity: high
symptoms:
  - "After a Resume, the silence lines and paragraph starts of the old segments were gone from transcript.md"
  - "A resumed note failed Verify for the old segments"
  - "A resumed note with a turn at the end of a segment failed Transcribe at each Resume"
root_cause: logic_error
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - resume
  - transcript
  - seed
  - join
  - verify
  - coverage
---

# A resumed note seeded from transcript.md lost its silences and paragraphs, and could fail its checks

## Problem

When a saved note is resumed, `seed_from_transcript` turns the turns in `transcript.md` into one window result for each old segment. Only the new segment is recognized. The join then writes `transcript.md` again from all the window results. The seeded pieces had ends that did not give back the same file (PR #30):

- A piece ended at the next turn, so the join found no pause. It removed the silence lines and the paragraph starts.
- A turn at the end of a segment gave a piece that ended 1 ms after the segment. `validate_result` refused a row outside the segment's input coverage. Each Resume seeded the same piece again, so the note could not finish.

Verify also checked the coverage of the old segments. Their seeded text has no recognized windows, so a resumed note failed Verify.

## Solution

Each piece end gives back what the join reads:

```rust
end_ms: own
    .get(i + 1)
    .map_or(segment.duration_ms, |(next, _, _)| *next)
    .saturating_sub(silence.unwrap_or(crate::join::PARAGRAPH_PAUSE_MS + 1))
    .max(*start_ms + 1)
    .min(segment.duration_ms),
```

- **Before the silence after the turn.** The join writes a silence line for the gap. A silence line after a pause or gap line is at the start of the next segment, so it does not shorten the piece before it (`last = None` on `Pause` and `Gap`).
- **Else more than `PARAGRAPH_PAUSE_MS` before the next turn**, so the join starts a new paragraph.
- **At least `start + 1`.** The recognition route widens a `(0, 0)` piece to its whole window.
- **At most the segment duration.** A piece at the segment end is then a time point, which the check accepts.

Verify judges coverage only for `segments[saved..]`, the segments that this run recognized. `validate_result` still checks all segments.

## Prevention

A seed that writes data for a later stage must give the output that stage would read back. Test the round trip: seed from a file, join, and compare the file. Test the edges: a turn at the segment start, a turn at the segment end, and a silence after a gap line. When a check covers recognized windows, skip the parts that have seeded data and no windows.
