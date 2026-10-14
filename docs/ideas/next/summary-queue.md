---
title: A separate queue for summaries
description: "Requests to the summary endpoint take turns in their own queue, so a slow model does not delay the transcript of a new note"
area: processing
added: 2026-10-14
source: "conversation of 2026-10-14 (desktop log and LiteLLM proxy log after PR #17)"
related: [retry-a-failed-summary-later, failed-regeneration-on-return]
plan: docs/plans/2026-10-14-1354-fix-summary-queue-plan.md
---

# A separate queue for summaries

## Idea
The runner has a second queue for the summary work: the Summarize and Save stages of a new note, Regenerate Summary and title jobs. It runs one job at a time. The audio stages keep their own permit. The reply wait goes up to 30 minutes, a second Regenerate Summary of a note is refused, and the log shows each request to the endpoint.

## Why
On 2026-10-14 a new note waited 7.5 minutes after Stop behind three regenerations, and one note was regenerated twice. A request that the proxy answered after 601 s was lost at Longhand's 600 s deadline.

## Notes / Current state
- The plan gives the design and the tests.
