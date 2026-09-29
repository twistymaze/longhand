---
title: One storage folder
description: "When the user changes the storage folder, offer to move the notes there or leave them; the library then has only one storage folder"
area: library-and-sharing
added: 2026-09-28
source: "the user, after the idea review of 2026-09-28"
related: [shared-settings-in-storage, cross-machine-library]
---

# One storage folder

## Idea
Longhand has exactly one storage folder. When the user chooses a different one, the app asks what to do with the notes in the old folder:

- **Move:** the notes move to the new folder, and the store follows them to their new places. This is like OneDrive when the user picks a different location for its folder.
- **Leave them:** the notes stay in the old folder, and the library forgets them. The files are not deleted.

## Why
Today the settings keep every storage folder that the user has ever chosen (`destination_roots`, R25a), and the library lists the notes of all of them. That rule was made so that a change of folder could never lose notes from the library. The user thinks it is complexity that nobody needs: there is no good reason to use several storage folders at the same time. One storage folder also makes [shared settings in the storage folder](shared-settings-in-storage.md) simpler, because there is only one place for them.

## Notes / Current state
- Do it at about the same time as [shared settings in the storage folder](shared-settings-in-storage.md); a brainstorm can cover both.
- R25a (walking skeleton plan) keeps each chosen root, and the index is rebuilt by a scan of all of them. Settings > Storage and the "Choose Folder…" action of a note that waits for a folder use this.
- Questions for a brainstorm:
  - A move between volumes or sync services is a copy and then a delete, and can be large and slow. Show progress, and make it safe after a crash or a full disk, as Save is.
  - Notes that record or process during the move.
  - Other computers that share the old folder: a move removes the notes from the old folder, so those computers forget them until they choose the new folder too.
  - What a computer does when it opens a storage folder that another computer moved away from.
  - What happens to existing installs with several roots. No installs need migration yet, so a simple rule is enough.
