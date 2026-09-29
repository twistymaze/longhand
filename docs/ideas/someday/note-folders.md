---
title: Folders for notes
description: "A few flat folders, real on disk, shown on Home as a filter like tags; no tree browser"
area: library-and-sharing
added: 2026-09-28
source: "the user, in the idea review of 2026-09-28"
related: [summary-prompt-by-note-type, library-and-browsing, cross-machine-library]
---

# Folders for notes

## Idea
The user can put each note in one folder. The folders are flat (one level) and few, for example `Pathfinder`, `1:1s`, `Customer calls` and `Other work`.

- **On disk,** a folder is a real directory in the storage folder, so Finder, Obsidian and agents see the grouping without Longhand.
- **In the UI,** a folder looks like a tag, not a tree. Home has a short filter (All and one entry for each folder). The note page shows the note's folder as one label, with a menu to move the note.
- **Behaviour:** a folder can be the "type of note" that chooses the summary prompt ([summary-prompt-by-note-type](summary-prompt-by-note-type.md)).

## Why
All notes are in one flat list today. The user wants to browse by group on Home, have the notes in meaningful folders on disk, and let the group choose defaults such as the summary prompt. The user expects few folders and no deep hierarchy, so a tree browser would be more UI than the need.

## Notes / Current state
- Nothing is built. Every note goes into the storage folder, and Home lists every note by day.
- `STRATEGY.md` allows folders and rules out tags ("Folders, and that's it"). The user first thought of tags because the current UI suits tags better than a tree. This idea keeps folders on disk and borrows the tag-like UI.
- Questions for a brainstorm:
  - Which folder a new note goes to: none, the last one used, or a choice in the New Note menu ([new-note-without-recording](../next/new-note-without-recording.md) adds that menu).
  - How a move works with one library across computers: a move is a rename of the home folder, which other computers see as a sync change.
  - Whether a folder can hold other folders. The current answer is no.
  - How a note that is recording or processing moves.
