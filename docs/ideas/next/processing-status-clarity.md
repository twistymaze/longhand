---
title: Clearer processing status
description: "Call the speaker stage 'Finding speakers' and give it a progress bar like Transcribing has"
area: interface
added: 2026-09-28
source: "the user, in the idea review of 2026-09-28"
related: [faster-diarization]
---

# Clearer processing status

## Idea
Two changes to the stage list that a note shows after Stop:

- **A plain label.** The speaker stage is called "Telling speakers apart" (`src/processing/describe.ts`). Call it "Finding speakers". Do not use "Identifying speakers": the text must not suggest that the app learns who a person is (R1 in the flexible speaker recognition plan).
- **A progress bar for the speaker stage.** Transcribing shows a bar that fills, but the speaker stage shows only a dot. It can take a long time, and the user cannot see how close it is to the end.

## Why
The user finds the current label awkward, and cannot see the progress of a slow stage.

## Notes / Current state
- **The order stays.** The speaker stage runs before Transcribing, and the list shows that order. There is a technical reason (KTD7 in the walking skeleton plan): transcription cuts the audio into windows of at most 30 s, and it cuts them at speaker changes when the turns are known first. Otherwise a window can hold two people, and a model that gives no word timestamps puts the whole window on one speaker. The user decided on 2026-09-28 not to change the pipeline or the order in the list.
- **Progress:** the speaker analysis has an `on_progress` callback (`crates/pipeline/src/diarize.rs`), and the runner sends `RunnerEvent::Progress` for some routes. `ProcessingStatus.tsx` shows a bar only when an event with a total arrives for the running stage. Find out which speaker routes send no total, or send it in a form that the UI does not read.
