---
title: System audio capture
description: "Record the computer's audio with the microphone; built in PR #12 and hidden until stereo processing has a new design"
area: capture
added: 2026-09-21
source: "Deferred from the walking skeleton (docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md, Scope Boundaries and How This Work Fits Together)"
related: [live-transcript-view, per-channel-processing, mixer-apps-and-microphone-copies, faster-diarization]
---

# System audio capture

## Idea
A second capture path for the computer's own audio, alongside the microphone, with its permission surface. The same work lets the user choose the microphone instead of only the system default.

## Why
Needed for meetings held on calls. Irrelevant to an in-person room, which is why the skeleton left it out.

## Why parked
The capture works, but processing a stereo note is not reliable. The mono mix makes a fault in one channel a fault in the whole note, and two Teams test calls failed Verify. It waits for a new processing design ([per-channel processing](../someday/per-channel-processing.md)), which waits for [faster diarization](../next/faster-diarization.md).

## Current state
- Planned in `docs/plans/2026-09-26-0827-feat-system-audio-and-microphone-choice-plan.md` and built in PR #12:
  - a Computer audio switch for each note;
  - a stereo recording of the raw microphone and the raw system audio;
  - a live mix with an echo gate, and echo removal after Stop;
  - a mono mix for the existing pipeline.
- The microphone choice ships. Computer audio is **hidden and turned off** (`SYSTEM_AUDIO_AVAILABLE` in `src-tauri/src/managers/capture.rs`, `features.systemAudio` in `src/lib/features.ts`). The capture code, the processing code and their tests stay in the tree.
- Why it is hidden: on 2026-09-26 two Teams test calls in the signed bundle failed Verify, each for a different reason. The mono mix makes a fault in one channel a fault in the whole note:
  - A mixer app put a late copy of the microphone and extra copies of the call on the system channel ([mixer apps and microphone copies](../someday/mixer-apps-and-microphone-copies.md)).
  - A speaker echo gave false hits in the copy check.
- Next: design the processing of a stereo note again. The likely design is [per-channel processing](../someday/per-channel-processing.md). It needs [faster diarization](../next/faster-diarization.md) first.
- Windows follows one output device (anarlog's rule). When the followed device is silent while another plays, the recording page warns.
