---
title: Library and browsing
status: open
area: library-and-sharing
added: 2026-09-21
source: "Separate track named in the walking skeleton (docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md, How This Work Fits Together)"
related: [cross-machine-library, ux-redesign, local-cli]
---

# Library and browsing

## Idea
Find the session from three weeks ago: list, preview, search, rename, delete.

## Notes
- Depends on the skeleton for the on-disk shape (R24, R24a), the retained destination roots (R25a), and the rule any index must satisfy (R25). The index itself is built with this track, including what a rebuild does when a retained root is missing or unmounted.
- The UX redesign (`docs/plans/2026-09-24-1956-feat-ux-redesign-plan.md`, shipped 2026-09-25 in PR #4) built the list, the preview, rename and delete: Home lists every session by day, a pointer on a row shows its summary, and a session can be renamed (the folder follows) or moved to the Trash. [One library across computers](cross-machine-library.md) then made Home list every note in the retained storage folders, including the notes that other computers recorded. What remains for this track is search.
