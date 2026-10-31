---
title: Remove the transcripts of old notes
description: "A second setting, off by default, removes the transcript of a note after a delay that is never shorter than the audio delay; the database keeps no transcript or summary text after processing; Transcribe Again transcribes again"
area: library-and-sharing
added: 2026-10-30
source: "the user, in a brainstorm on 2026-10-30"
related: [remove-old-audio]
plan: docs/plans/2026-10-30-1438-feat-remove-old-transcripts-plan.md
---

# Remove the transcripts of old notes

## Idea
A shared setting in Settings > Storage removes the transcript of each saved note a set number of days after the note last recorded, for example 7 days for audio and 90 days for transcripts. The summary, manual notes, title and tags stay. A Remove Transcript action removes the transcript of one note, and any audio that is left, at once.

## Why
Privacy. A transcript holds every word that each person said. Some users want old meetings to keep only the summary and their own notes. Audio removal already does this for the audio.

## Notes / Current state
- Off by default, so current users keep their transcripts.
- The transcript delay is never shorter than the audio delay.
- The plan holds the requirements.
- Follow-up (found in code review): when the marker syncs to another computer before the deletions, that computer can keep the audio of the note until the audio delay ends. See the PR.
- Shipped in PR 87, merged 2026-10-31.
