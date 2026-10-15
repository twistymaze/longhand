---
title: Say where a summary error came from
description: "A failed summary says whether the error came from the endpoint (such as OpenRouter) or from the model provider behind it"
area: transcript-and-summary
added: 2026-10-15
source: "anarlog review, 2026-10-15: commit 6fe1213d"
related: [retry-a-failed-summary-later]
---

# Say where a summary error came from

## Idea
Read the error fields of the endpoint's reply and tell the user whether the endpoint or the model provider behind it failed. Read an error object in a 200 reply too.

## Why
`crates/summarize/src/client.rs` (near line 605) shows the first 200 characters of the reply body. OpenRouter can return an error inside a 200 reply, and Longhand then reports "no choices", which does not help the user. anarlog changed this in 6fe1213d. Low priority.
