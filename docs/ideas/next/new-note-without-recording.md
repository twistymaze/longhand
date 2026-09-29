---
title: Start a new note without recording
description: "A setting chooses whether New Note records at once; a split button gives the other choice, and the note page has a Record button"
area: capture
added: 2026-09-28
source: "the user, in the idea review of 2026-09-28"
related: [ux-redesign]
---

# Start a new note without recording

## Idea
A setting chooses what New Note does by default: start recording at once (today's behaviour), or open an empty note that does not record.

- **New Note is a split button.** A click on the button does the default. Its menu gives the other choice, so that a user can do the less usual one without a change to Settings.
- **The note page has a Record button** when the note is not recording. anarlog has one; read it in `reference-source/anarlog/` before the design.
- **The recording shortcut always does the default.** It has no menu.

## Why
Users will prefer one behaviour or the other, so a setting is simpler than a fixed choice. There are two uses for a note that does not record at once:
- **Open the note early.** Type an agenda or other context, and start recording at the right moment.
- **Notes only.** The topic is sensitive, and the user does not want a recording.

## Notes / Current state
- Today New Note starts recording at once with the defaults from Settings. Resume can record more into a saved note, so a recording that starts after the note exists can probably use the same path.
- Open questions:
  - A note that never records: does it get a summary from the typed notes only, or no summary? Where is it saved, given that Save today moves a finished recording?
  - What the Home list and the note page show for a note that has no audio.
  - The attention budget of the recording surface (U7) applies to the Record button and to the moment when recording starts.
