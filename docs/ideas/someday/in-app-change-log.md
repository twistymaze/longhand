---
title: In-app change log
description: "The app shows what changed since the last minor version, and a plain-language summary of each month, written by a model in CI"
area: interface
added: 2026-10-27
source: "a conversation on 2026-10-27"
related: [scheduled-release-batches, about-longhand-and-readme]
---

# In-app change log

## Idea
The app shows what changed, in the app and not only on GitHub. There are two possible views:

- **Since the last minor version.** The app combines the release notes of all patch releases since the last minor release (or since the version that the user had before the update) into one list.
- **Each month.** A short, plain-language summary, for example "New in October". A model writes it from the release notes of that month.

## Why
Each merged PR that changes the app makes a patch release, so one release has one or two small items. The value that the user gets is from many patches together, and a list of single patches is hard to read.

## Notes / Current state
- Each release already has notes from git-cliff (`scripts/release-version.ts`, `cliff.toml`). `scripts/publish-release.ts` writes them to the GitHub release and to `notes` in the update manifest. The About pane has a "What's new" link to the GitHub releases page.
- A summary written by a model fits best in CI, not in the app: it then needs no AI Provider on the user's computer, and the text is the same for all users. Possible flow: a scheduled job or the release job sends the notes since the last summary to a model, writes the result to a file in the repository or to the release, and the app reads that file. A person can read and edit the text before it ships, for example in a PR.
- The text that the model gets is only commit subjects and PR titles, not user data, so this does not conflict with the local-first boundary.
- Open questions:
  - Where the app gets the text: in the app bundle (known at build time, offline), or from the GitHub release (current, but needs the network).
  - When to show it: in the update dialog before an update, once after an update, or only on request from About.
  - Whether the "since the last minor version" view is still useful if `scheduled-release-batches` makes fewer, larger releases.
  - Which model and key CI uses, and where the key is kept (the `release` environment).
