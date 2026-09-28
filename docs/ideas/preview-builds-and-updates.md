---
title: Preview builds and in-app updates
status: in-progress
area: distribution
added: 2026-09-28
source: "a brainstorm on 2026-09-28"
related: []
---

# Preview builds and in-app updates

## Idea
Each push to main that passes CI publishes a signed, notarized DMG and update files to a public repository that holds only builds. The app offers each newer build in a standard macOS dialog, and never while a note records or processes.

## Why
New builds get to a Mac only by hand today. The user and a few peers want each change on main to reach them as an update offer.

## Notes / Current state
Requirements: `docs/plans/2026-09-28-1240-feat-preview-builds-and-updates-plan.md`. A stable track, release tags and Windows updates come later.
