---
title: Granite word timing was treated as absent
date: 2026-09-28
category: logic-errors
module: ASR native timing adapter and separate speaker analysis
problem_type: logic_error
component: service_layer
symptoms:
  - "Five-minute checks passed, but full-length Granite processing failed with OutputTruncated."
  - "A short failing excerpt passed when the input included surrounding conversation."
root_cause: logic_error
resolution_type: code_fix
severity: high
tags: ["granite", "timestamps", "speaker-recognition", "windowing", "long-audio"]
---

# Granite word timing was treated as absent

## Problem

The ASR adapter treated token timestamps as the only native fine-grained timing.
Granite Plus supplied word timestamps. The false capability result caused separate
speaker analysis to split transcription input at speaker changes, removing useful
recognition context. [PR #18](https://github.com/twistymaze/longhand/pull/18) corrected
the adapter and retained bounded recovery for native output truncation.

## Symptoms

- The full recording failed on a 5.140-second window. That exact excerpt also
  failed in a fresh process, while its surrounding 30 seconds passed.
- Both halves of the failing excerpt passed. A split retry fixed that excerpt,
  but the full run later failed on a 1.390-second window.
- Adding silence did not fix the later excerpt. Including 20 seconds of
  surrounding conversation did. These observations implicated input context;
  they did not establish that every short input fails.

## What Didn't Work

Splitting a truncated decode was useful but did not fix the route. Recovery needs
at least one second per child, so it cannot split a 1.390-second input. Removing
that bound or accepting partial output would not establish complete recognition.
Successful five-minute checks also did not establish full-recording completion.

Earlier implementation checks used Whisper with short speech windows, rather
than a Word-only timing model (session history). Those results were not evidence
that every timestamp capability used the correct downstream route.

## Solution

`crates/asr/src/engine.rs` now retains the loaded artifact's timestamp kind:

```rust
match maximum {
    TimestampKind::Word | TimestampKind::Token => Some(maximum),
    _ => None,
}
```

The adapter requests that kind and reads the corresponding output collection.
Word entries become timed tokens with explicit word boundaries. Their times are
clamped to the decoded part and shifted by its offset. Merely changing the
capability predicate without reading the word collection would be incomplete.

`crates/pipeline/src/recognition/stages.rs` adds speaker-change cuts only when
native fine-grained timing is absent. Timed ASR can therefore use its normal
windows and join the separate speaker timeline afterwards. This does not decode
the entire recording as one unbounded window.

`crates/asr/src/recovery.rs` still retries only typed `OutputTruncated` errors,
with bounded depth and minimum child duration. Both children must succeed.
Other errors propagate; partial text is not accepted as a complete result.

## Why This Works

The route now uses the timing contract of the loaded artifact rather than
assuming that Word and Token capabilities are interchangeable. Speaker changes
do not need to become ASR input boundaries when native times support assignment
after recognition.

Integrated Granite attribution is a separate mode. It requests automatic timing
and can produce source-window precision. This correction does not make that
mode word-aligned or establish accurate speaker identities.

The full-length Granite plus separate Nemotron backend check passed after the
correction and reached the source tail. See the
[timing reference](../../reference/recognition-timings-2026-09-27.md) for results
and limits. That backend pass is distinct from the failed long native UI test.
Its debug elapsed time is not a laptop or release-app performance estimate.

## Prevention

- Test both the loaded capability and the actual returned collection. Keep the
  Word-only regression with an empty token vector in the ASR adapter tests.
- Check that a separate speaker route preserves the normal decode plan when
  native timing is available. Keep the context-sensitive pipeline regression.
- Use an exact failing excerpt and surrounding context as controls. After a
  local fix, repeat the long source that exposed the failure.
- Inspect the native producer before adding a fallback for allegedly missing
  output. A tentative missing-word concern in this investigation was withdrawn
  after the producer showed how it represented trailing text.
- Keep real-recording checks local and opt-in. Report completion, coverage,
  speaker quality, native UI behavior, and speed as separate results.

## Related Issues

- [Flexible speaker recognition plan](../../plans/2026-09-27-1526-feat-flexible-speaker-recognition-plan.md)
- [Recognition timing and validation record](../../reference/recognition-timings-2026-09-27.md)
