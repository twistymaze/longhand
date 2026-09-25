---
title: One library across computers that share a destination
status: in-progress
area: library-and-sharing
added: 2026-09-23
source: "Session conversation, recorded in docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md (How This Work Fits Together)"
related: [library-and-browsing]
---

# One library across computers that share a destination

## Idea
When several computers save to the same synced destination (for example OneDrive), each lists only the sessions it recorded, because its session list comes from its own local store. A unified library would build its list from the session folders themselves.

## Notes
- Saving works today: both computers' folders sit side by side, and each only writes to folders it created. Names clash only if both start a session in the same minute (a `-2` suffix, or a cloud conflict copy if the other folder has not synced yet).
- Would have to settle whether one computer may resume or re-summarize a session another computer recorded.
- Picked up in `docs/plans/2026-09-25-1217-feat-one-library-across-computers-plan.md`: one person on several computers, a complete `session.md`, live folder watching, the folder as the authority on every computer, and every action (Resume included) on every computer.
