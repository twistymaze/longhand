---
title: Show a failed regeneration when the user comes back to the note
description: "A Regenerate Summary that fails while the user is on another page shows why when the user opens the note again"
area: transcript-and-summary
added: 2026-10-14
source: "code review of PR #17 (correctness reviewer and the cross-model review), 2026-10-14"
related: [retry-a-failed-summary-later]
---

# Show a failed regeneration when the user comes back to the note

## Idea
Longhand keeps the error of the last failed Regenerate Summary of each note. When the user comes back to a note whose regeneration failed while they were away, the note shows the reason once, as it does when the user stays on the page.

## Why
Since PR #17 the note page shows a regeneration that still runs after the user comes back. When that regeneration fails, the page shows the previous summary again with no message, so the user does not know that it failed or why.

## Notes / Current state
- The page learns that a regeneration runs from `SessionDetail.regenerating`. The error goes only to the page that started the run.
- Clear the stored error when the next regeneration starts or succeeds.
