---
title: A Resume survives an audio removal before its copy is done
description: "When another computer removes a note's old audio before a Resume has copied it, the Resume fails and Retry is unavailable; let processing go on without the old audio"
area: processing
added: 2026-09-30
source: "U5 of the audio removal plan (docs/plans/2026-09-30-0357-feat-remove-old-audio-plan.md, Risks)"
related: [remove-old-audio]
---

# A Resume survives an audio removal before its copy is done

## Idea
Resume copies the old audio of a note to the working folder on a background thread. If another computer removes that audio before the copy is done, and before this computer's in-use marker has synced, assembly cannot read the old segments. The note goes to ProcessingFailed, and Retry is not available, so the new recording is not processed. Let speaker processing and recognition go on with the new segments only, and keep the old result for the old part.

## Why
The new recording of the Resume is lost to processing in this case. The window is narrow: two computers, an old note, and a removal pass inside the sync delay.

## Notes / Current state
- Recorded as an open risk in the audio removal plan.
