---
title: Search the library
description: "Find a note by words in its title, summary, transcript or notes"
area: library-and-sharing
added: 2026-09-21
source: "Separate track named in the walking skeleton (docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md, How This Work Fits Together)"
related: [cross-machine-library, ux-redesign]
---

# Search the library

## Idea
Find the note from three weeks ago: search the titles, summaries, transcripts and notes of every note in the library.

## Notes
- The rest of the original library track shipped. The UX redesign (`docs/plans/2026-09-24-1956-feat-ux-redesign-plan.md`, PR #4) built the list, the preview, rename and delete. [One library across computers](../done/cross-machine-library.md) made Home list every note in the retained storage folders, including the notes that other computers recorded.
- Search needs an index. It must follow the rule for any index (R25) and the retained destination roots (R25a), including what a rebuild does when a retained root is missing or unmounted.
- The organization idea (tags or folders) is close to this. Design the two together if both are picked up.
