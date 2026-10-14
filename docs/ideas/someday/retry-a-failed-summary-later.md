---
title: Retry a failed summary later
description: "When a summary failed because the endpoint could not be reached or did not answer, Longhand makes it again later without a click"
area: transcript-and-summary
added: 2026-10-14
source: "conversation of 2026-10-14 (ce-debug of a summary stub after the laptop lid closed; PR #17)"
related: [failed-regeneration-on-return]
---

# Retry a failed summary later

## Idea
A note that was saved with a summary stub because of a short endpoint problem gets its summary later, with no action by the user. Longhand records why the summary failed. When the cause was temporary (no connection, a server error, a deadline), it tries again at a later time, for example after the next wake, after a network change, or after some minutes. It stops after a few tries and keeps the stub with its reason.

## Why
On 2026-10-14 a note was saved with "No summary was produced: could not reach the endpoint". The user expected Longhand to retry after the laptop was open again for a long time. Today a stub is final: only Regenerate Summary makes the summary.

## Notes / Current state
- The summary stage retries a request only while it runs, and only after a failed connection (about 7 minutes of waits). See `docs/solutions/logic-errors/summary-in-a-wake-with-no-network-kept-the-default-budget.md`.
- A retry must never send a request again when the endpoint can have received it.
- A cause that needs the user (a rejection for length, a missing key) is not retried.
- Only one computer should retry a note. The note's in-use marker decides.
