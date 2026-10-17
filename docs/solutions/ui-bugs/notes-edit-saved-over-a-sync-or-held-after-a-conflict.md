---
title: A My Notes edit was saved over a sync, or stayed only in memory after a save conflict
date: 2026-10-16
category: ui-bugs
module: My Notes saves (src/session/NotesTab.tsx, src/editor/NoteEditor.tsx, src/lib/notesSaves.ts)
problem_type: ui_bug
component: frontend_stimulus
severity: high
symptoms:
  - A click edit (task checkbox, toolbar, slash menu) made just before notes from another computer arrived was written over them with no conflict draft
  - After a save conflict with unchanged notes (notes.md could not be read), later edits were never saved and were lost at a quit
  - A leave of the note ended before an edit that a conflict held was saved
root_cause: logic_error
resolution_type: code_fix
framework_version: react 19, @handlewithcare/react-prosemirror 3.2.6
tags:
  - fingerprint
  - sync-reload
  - save-conflict
  - flushSync
  - unmount-flush
---

# A My Notes edit was saved over a sync, or stayed only in memory after a save conflict

## Problem

Each notes save sends the fingerprint of the notes.md that the edit was made on (KTD10 of the walking-skeleton plan). The editor keeps a change for up to 500 ms before it sends it.

1. A sync read checked "idle" with the key and input events only. A click edit gave no such event, so the read applied the new notes, set the new fingerprint, and gave the editor a new key. The old editor's unmount flush then sent the old text with the new fingerprint, and the backend accepted it over the other computer's text.
2. After a conflict, edits wait in `held`. When the read found the same fingerprint (the conflict came from a read failure, not a change), it returned early and `held` never ended: every later edit went into memory only.
3. Held edits went out at a leave only through the tab's unmount, and `notesSaved` waited only for saves that were already running. A conflict that moved the next edit into `held` during a leave was not awaited.

## Solution

- The read flushes the editor (a `flush()` handle on `NoteEditor`) before its idle check. A pending change then starts a save with the old fingerprint, the tab is not idle, and the read tries again after that save.
- A held edit makes the read not idle too, so it goes into a draft before the notes are replaced. An unchanged read ends the held state.
- The read replaces the editor with `flushSync`, in the same task as the flush. react-prosemirror records a change only when React renders the editor (a plugin view update), and its own dispatch also uses `flushSync`, so no change can reach the old editor between the flush and its unmount.
- NotesTab registers a leave send (`addNotesSend`): flush the editor, then send the held edit. `notesSaved` runs the sends again after each round of saves, until no save runs.

## Prevention

- "Idle" cannot rest on key events: clicks, the toolbar and menu commands change the document too. Flush the pending change and look at the save state.
- A state that holds edits needs an end at every exit: a changed read, an unchanged read, a leave, an unmount.
- A leave that waits for saves must send again after each save, because a save result can make an edit wait again.
- happy-dom and act render a click and a state update together, so a window between two React renders cannot be tested there. Close the window (`flushSync`) instead of guarding it with code that no test reaches.
