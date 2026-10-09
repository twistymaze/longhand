---
title: Note types and tags
description: "One note type for each note, which is also its folder on disk, plus free tags with GitLab-style scopes; Home filters by both"
area: library-and-sharing
added: 2026-09-28
source: "the user, in the idea review of 2026-09-28; changed to note types and tags in the brainstorm of 2026-10-07"
related: [summary-prompt-by-note-type, suggested-tags, library-and-browsing, cross-machine-library]
plan: docs/plans/2026-10-07-1940-feat-note-types-and-tags-plan.md
---

# Note types and tags

## Idea
Each note can have one note type and any number of tags.

- **Note type:** for example `Pathfinder`, `One on One` or `Internal Meeting`. The note type is also a folder in the storage folder, so Finder, Obsidian and agents see the grouping without Longhand.
- **Tags:** free labels. `::` splits a tag into a scope and a value, as in GitLab: `Person::Nathan`, `Customer::Mayo Clinic`, `Topic::Security`. The scope only changes how the label looks.
- **In the UI:** Home has a filter bar for one type and any tags. The note page shows the type and tags in a row under the title. The New Note menu lists the note types.

## Why
All notes are in one flat list. The user wants to browse by group on Home, keep notes in meaningful folders on disk, and label notes across folders by person, customer or topic. The first version of this idea was folders only. After one or two weeks of use, the user also wanted tags.

## Notes / Current state
- Shipped in PR #45. Changes from the plan during the manual test: the built-in type General (the top of the storage folder) replaces "no type"; types are in name order; tag scope colours are assigned across the library; the summarizer gets the type, the tags and the user's own title.
- The brainstorm of 2026-10-07 wrote the requirements in the plan above. It also changes the `STRATEGY.md` boundary that ruled out tags (R25 of the plan).
- Follow-up ideas: [suggested-tags](../someday/suggested-tags.md) and [summary-prompt-by-note-type](../next/summary-prompt-by-note-type.md).
