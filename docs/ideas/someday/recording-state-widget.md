---
title: Always-visible recording state widget
description: "A small widget that is always on screen while a note records or is paused, like Handy's overlay, so the user sees a forgotten pause or a recording left running after the meeting"
area: interface
added: 2026-10-09
source: "pause-recording brainstorm (conversation, 2026-10-09)"
related: [pause-recording]
---

# Always-visible recording state widget

## Idea
A small widget stays on screen, above other windows, while a note records or is paused. It shows the state (recording or paused) and the elapsed time, like Handy's recording overlay (`reference-source/Handy/src/overlay/RecordingOverlay.tsx`). The main window does not need to be in front.

## Why
The user has two failures that the main window and the tray do not prevent:

- The meeting starts again after a break, and nobody un-pauses the note.
- The meeting ends, and the note records for a long time after it. The user has done this several times.

This replaces a reminder that listens for speech while paused. The user rejected that design during the pause-recording brainstorm.

## Notes / Current state
- U7's two-tier rail and attention budget apply: the widget must not draw participants' attention to the screen.
- The tray already shows a state ("Interrupted") for a failed capture.
- Open questions: position, size, whether it has controls (Pause, Stop), whether it shows on all displays, and whether it can be turned off.
