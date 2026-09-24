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
- The UX redesign input brings a small library back in (home as a session list with previews) and asks how much of it the redesign takes on. The 16-second test sessions sit in the list with no way to remove them.
