---
title: Make a note from an audio file
description: "Make a new note from an audio file that another device recorded, for example an iPhone Voice Memo sent by AirDrop"
area: capture
added: 2026-10-13
source: "conversation about iPhone capture, 2026-10-13"
related: [iphone-capture]
plan: docs/plans/2026-10-24-0928-feat-import-audio-recordings-plan.md
---

# Make a note from an audio file

## Idea
The user chooses an audio file, or drops it on Home. Longhand makes a new note from it and processes it on this computer: transcript, speaker turns and summary, as for a recording. The note gets its date and time from the file. The user then sets the note type, tags and notes as usual.

## Why
Sometimes a laptop is not available, for example in a hallway or at a table where a laptop is not suitable. A phone is available. With this feature, any recorder app (Voice Memos, for example) is sufficient, and AirDrop, iCloud Drive, OneDrive or a cable moves the file. Longhand needs no phone app and no sync of its own.

`STRATEGY.md` permits this: "A recording you made on another device counts as what you record."

## Notes / Current state
- This is the first step of [iphone-capture](../someday/iphone-capture.md), and it gives value alone.
- The brainstorm on 2026-10-24 added a watched folder on one computer, so that a file that a phone app saves to a synced folder becomes a note with no action on the Mac. The plan holds the decisions.
- Formats: m4a (AAC), wav and mp3. The note converts the audio to the normal audio format of a note and keeps no copy of the original.
- The computer that imports the file does the processing and the publication, so the rule that only the computer that starts a publication finishes it does not change.
