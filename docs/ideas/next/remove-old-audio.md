---
title: Remove the audio of old notes
description: "An optional setting removes only the audio of a note after a delay, such as 30 days, to control the growth of disk use; off by default"
area: library-and-sharing
added: 2026-09-28
source: "the user, during the shared-settings brainstorm of 2026-09-28"
related: [one-storage-folder, shared-settings-in-storage]
---

# Remove the audio of old notes

## Idea
A setting removes the audio of each note that is older than a delay that the user sets, for example 30 days. The transcript, the summary, the notes and the title stay. The setting is off by default, and the user sets the delay.

## Why
The audio is the largest part of a note. A four-hour note makes a large file, and the storage folder grows with each note. After some weeks the user seldom needs the audio, because the transcript is the system of record.

## Notes / Current state
- Transcribe Again makes the transcript and summary again from the audio. A note with no audio cannot do this, so the menu must show that, and the confirmation of the setting must say it.
- Questions for a brainstorm:
  - Which date starts the delay: the date of the recording, or the date when the note was last opened or processed.
  - Remove the audio to the Trash or the Recycle Bin, or remove it permanently.
  - A way to keep the audio of one note, such as a "Keep audio" choice on the note.
  - Is the setting shared ([shared settings in the storage folder](../done/shared-settings-in-storage.md))? In a synced storage folder, which computer removes the audio: each computer only for the notes it recorded (the rule for automatic steps in the library), or any computer.
  - A note that is not finished (recording, processing, waiting for a folder) never loses its audio.
  - Show the space the setting will free before the user turns it on.
