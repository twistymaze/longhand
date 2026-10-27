---
title: In-app change log
description: "The app shows what changed since the last minor version, and a plain-language summary of each month that a model in CI updates at each release; CHANGELOG.md keeps the full list"
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

The repository also keeps `CHANGELOG.md`, a list of all releases in date order, with the newest release first (as Release Please does). The app and the README can link to it.

The release job updates all of this at each release, with no manual step.

## Why
Each merged PR that changes the app makes a patch release, so one release has one or two small items. The value that the user gets is from many patches together, and a list of single patches is hard to read.

## Notes / Current state
- Each release already has notes from git-cliff (`scripts/release-version.ts`, `cliff.toml`). `scripts/publish-release.ts` writes them to the GitHub release and to `notes` in the update manifest. The About pane has a "What's new" link to the GitHub releases page.
- A summary written by a model fits best in CI, not in the app: it then needs no AI Provider on the user's computer, and the text is the same for all users.
- **No manual step.** At each release, the release job:
  1. Adds the release notes of the new version to the top of `CHANGELOG.md`. git-cliff can write this file (`cliff.toml` already has a `[changelog]` section).
  2. Sends the notes of the current month to a model, which writes the monthly summary again.
  3. Commits both files to `main`, or attaches them to the release.
  No person reads the text before it ships. The prompt must keep the model to the facts in the notes.
- If the summary step fails (no answer from the model), the release still publishes. The next release writes the summary again.
- The text that the model gets is only commit subjects and PR titles, not user data, so this does not conflict with the local-first boundary.
- Open questions:
  - Where the app gets the text: in the app bundle (known at build time, offline), or from the GitHub release (current, but needs the network).
  - When to show it: in the update dialog before an update, once after an update, or only on request from About.
  - Whether the "since the last minor version" view is still useful if `scheduled-release-batches` makes fewer, larger releases.
  - Which model and key CI uses, and where the key is kept (the `release` environment).
  - How the release job commits to `main`: the commit must not start a new release (a `docs` or `chore` commit makes no release) and must pass branch protection.
  - Whether `CHANGELOG.md` starts with all past releases (from the release tags) or only with new releases.
