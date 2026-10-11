---
title: Type and tag filter refinements
description: "Hide the Home type and tag filters until they are useful, show General on rows when types are in use, and give the Tags menu a filter field and grouped tag labels"
area: library-and-sharing
added: 2026-10-08
source: "the user, after daily use of note types and tags (archived PR 45)"
related: [note-folders, suggested-tags]
plan: docs/plans/2026-10-08-1624-feat-type-and-tag-filter-refinements-plan.md
---

# Type and tag filter refinements

## Idea
Home shows the type filter, and a General label on General notes, only when the type list has a type other than General. Home shows the Tags menu only when a note has a tag. The Tags menu gets a filter field, groups the tags by scope, and shows each tag as its coloured label.

## Why
A user who keeps every note in General sees a type filter whose choices show the same notes. The Tags menu shows `Scope::Value` as plain text that is cut off and hard to scan with many people. A General note has no label on Home when other types exist.

## Notes / Current state
- Refines R19 and R22 of `docs/plans/2026-10-07-1940-feat-note-types-and-tags-plan.md`.
- Shipped in archived PR 46. The button that clears the type and tag filters is named "Clear Filters".
