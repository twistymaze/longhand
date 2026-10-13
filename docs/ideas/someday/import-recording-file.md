---
title: Make a note from an audio file
description: "Make a new note from an audio file that another device recorded, for example an iPhone Voice Memo sent by AirDrop"
area: capture
added: 2026-10-13
source: "conversation about iPhone capture, 2026-10-13"
related: [iphone-capture]
---

# Make a note from an audio file

## Idea
The user chooses an audio file, or drops it on Home. Longhand makes a new note from it and processes it on this computer: transcript, speaker turns and summary, as for a recording. The note gets its date and time from the file. The user then sets the note type, tags and notes as usual.

## Why
Sometimes a laptop is not available, for example in a hallway or at a table where a laptop is not suitable. A phone is available. With this feature, any recorder app (Voice Memos, for example) is sufficient, and AirDrop, iCloud Drive, OneDrive or a cable moves the file. Longhand needs no phone app and no sync of its own.

`STRATEGY.md` permits this: "A recording you made on another device counts as what you record."

## Notes / Current state
- This is the first step of [iphone-capture](iphone-capture.md), and it gives value alone.
- Formats to accept: at least m4a (AAC) and wav. Decoding probably uses the same path as the audio in the note folder.
- Open question: keep the original file in the note folder, or convert it to the normal audio format of a note.
- The computer that imports the file does the processing and the publication, so the rule that only the computer that starts a publication finishes it does not change.
