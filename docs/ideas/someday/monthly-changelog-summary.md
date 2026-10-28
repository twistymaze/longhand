---
title: Monthly changelog summary
description: "A plain-language summary of each month's changes that a model in CI writes at each release, and a view of the changes since the last minor version"
area: interface
added: 2026-10-28
source: "the open part of in-app-change-log after PR #74"
related: [in-app-change-log, scheduled-release-batches]
---

# Monthly changelog summary

## Idea
PR #74 shipped the changelog by month: the same lines as the release notes, in Recent changes, in the update window and on the changelog page. Two parts of [in-app-change-log](../done/in-app-change-log.md) are still open:

- **A summary of each month.** A short, plain-language text, for example "New in October", that a model in CI writes from that month's changelog entries at each release.
- **Since the last minor version.** One list of the changes since the last minor release, or since the version that the user had before the update.

## Why
Each merged PR makes a patch release, so the monthly list is many small lines. A short summary tells a user what matters without reading every line.

## Notes / Current state
- The changelog data is `changelog.json`, which `scripts/lib/changelog.ts` writes from `git-cliff --context` at each release. A summary step can read the same data.
- A summary written in CI needs no AI Provider on the user's computer, and every user sees the same text. The text the model gets is only PR titles, not user data.
- If the summary step fails, the release must still publish; the next release writes the summary again.
- Open questions: which model and key CI uses, where the summary is stored (a field in `changelog.json` keeps the "commit nothing" rule), and whether "since the last minor version" is still useful if `scheduled-release-batches` makes fewer, larger releases.
