---
title: Keep the chosen tab when processing ends
description: "Do not move a user who reads the Transcript tab to Summary when processing ends"
area: interface
added: 2026-10-15
source: "anarlog review, 2026-10-15: commit 5b1e7a37"
related: []
---

# Keep the chosen tab when processing ends

## Idea
When processing ends while the user reads a tab other than Summary, stay on that tab.

## Why
`src/session/SessionPage.tsx` (near line 286) moves the user to Summary when processing ends, unless they are typing notes. A user who reads the transcript loses their place. anarlog changed this in 5b1e7a37.

## Notes / Current state
- Conflicts with flow F2 of the walking-skeleton plan ("a completed session shows Summary"). Decide which rule wins before work starts. One option: move to Summary only when the user is still on the tab that was open at the start of processing.
