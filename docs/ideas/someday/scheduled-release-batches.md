---
title: Scheduled release batches
description: "Collect the commits on main into a release once or twice a day, in place of one release for each push to main"
area: distribution
added: 2026-10-13
source: "conversation after the safe agent edits PR (#13), 2026-10-13"
related: [releases-in-source-repository]
---

# Scheduled release batches

## Idea

Keep two streams. Each push to `main` still runs CI and can make a build. A separate scheduled job makes the release: once or twice a day, it collects the commits on `main` since the last release into one version, with one set of release notes and one update notice.

## Why

Today each push to `main` that changes the app makes a release ([Releases](../../releases.md)). When several PRs merge close together, each one makes an update notice, although the user gets the value only from all of them together. The safe agent edits work was first split into three PRs, which would have made three fast releases.

## Notes / Current state

- `scripts/release-version.ts` already finds the highest effect of all commits since the last release tag, so a batch gets the correct version size (patch, `+` minor, `!` major).
- Open questions:
  - The schedule (times, time zone), and whether a release can still start by hand (`workflow_dispatch`).
  - Whether a day with only `docs`, `ci` or `chore` commits makes no release, as now.
  - Where the Windows installer job hooks in, because it now follows the CI run of the released commit.
  - Whether a per-push build is still useful for testing when it is not a release.
