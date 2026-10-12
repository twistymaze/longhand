---
title: Release Versioning - Plan
type: ci
date: 2026-10-12
topic: release-versioning
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Release Versioning - Plan

## Goal Capsule

- **Objective:** A minor release of Longhand occurs only when the user chooses it. Agents choose `feat` or `fix` only by what changed for the user, and that choice does not decide the size of the release.
- **Product authority:** The user decides each minor and each major release. Agents recommend; they do not decide.
- **Open blockers:** None.

---

## Product Contract

### Summary

Each release-type commit makes a patch release by default. A `+` after the type in a PR title (`feat+:` or `feat(scope)+:`) makes a minor release. Before each `feat` PR, the agent asks the user "patch or minor?" and gives a recommendation. git-cliff stays the tool that parses commits and writes the release notes. The release script makes the final choice of version.

### Problem Frame

Today `cliff.toml` follows strict semver: each `feat` makes a minor release (`docs/releases.md`, When a release is due). So `CLAUDE.md` makes agents use `feat` only for a new capability, and it gives PR #8 (new text and links in Settings → About) as an example of a `fix`. Agents must make that judgement for each PR. They put real improvements under `fix` so that no minor release occurs. Then the release notes list improvements under the wrong heading, and the minor number does not mark the releases that the user thinks are important.

Longhand is an app, not a library. No other software depends on its version number for compatibility. So strict semver gives no value here, but the user wants the minor number to mark the releases that they choose.

### Key Decisions

- **Patch by default; a minor release only when the user chooses it.** Governs R1, R2, R10. (session-settled: user-directed — chosen over an agent decision by a written test, and over no minor marker at all with `feat` keeping its minor release: the user wants control of the minor number, with the agent's recommendation.)
- **The agent asks at each `feat` PR, and only then.** Governs R10, R11. Hands-off runs (lfg, babysit) wait at each `feat` PR until the user answers; the user accepted that cost. (session-settled: user-directed — chosen over asking only when the agent recommends a minor, and over a recommendation in the PR description that never blocks: the user wants to see each decision.)
- **`feat` means any visible gain, and the headings follow.** Governs R6, R8. (session-settled: user-directed — chosen over "new things only" and over a broad `feat` with the current headings: `feat` no longer has a release cost, so it can describe improvements correctly.)
- **The minor marker is `+` after the type or scope.** Governs R2, R9. It is one character in the same place as `!`, and it has no other meaning. (session-settled: user-directed — chosen over `!` for minor, a `Release: minor` line in the squash commit body, and a `[minor]` word at the end of the title: the user wanted an unobtrusive marker, no new Conventional Commit type, and no change to the standard meaning of `!`.)
- **No new Conventional Commit types.** Governs R2, R9. (session-settled: user-directed — chosen over a new `minor:` type: the user does not want invented prefixes.)
- **git-cliff gives the next patch; the release script raises it.** Governs R5. With `bump_type = "patch"`, git-cliff returns only the latest tag (no release due) or the next patch. The script raises that version to a minor or a major from the markers, and it never lowers a version. git-cliff only reads the history and prints; it never creates a tag. (session-settled: user-directed — chosen over standard semver rules with a script that lowers a minor to a patch, and over a `cliff.toml`-only rule that renames `feat` before the version calculation.)
- **A major release stays a user request with `!`.** Governs R3. The user said a major release can be a manual operation. The current rule already is one: an agent writes `!` only when the user asks. So no separate manual workflow is needed. (session-settled: user-approved — proposed with that tradeoff in the summary, and the user confirmed it.)
- **The squash commit body stays empty.** Governs R12. The marker is in the title, so no repository setting changes.

### Requirements

**Version rule**

- R1. Each release-type commit (`feat`, `fix`, `perf`, `refactor`, `revert`, `build`) since the latest release tag makes a patch release by default.
- R2. A commit whose subject has `+` directly after the type or scope and before the colon (`feat+:`, `feat(capture)+:`) makes the release a minor release. `+` is valid only on `feat`.
- R3. A commit whose subject has `!` after the type or scope makes a major release. A `BREAKING CHANGE` footer has no effect.
- R4. When the commits since the latest release tag have different effects, the highest effect applies. `docs`, `ci`, `test`, `chore` and `style` commits make no release, as now.
- R5. git-cliff decides whether a release is due, gives the next patch version, and writes the release notes. The release script raises that version to a minor (R2) or a major (R3) from the original commit subjects.

**Release notes**

- R6. The release notes headings are "New features and improvements" (`feat`), "Fixes" (`fix`, `perf`) and "Other changes" (`refactor`, `revert`, `build`). A heading with no commits does not appear, as now.
- R7. A `feat+` commit appears in the notes in the same way as a `feat` commit, under its heading, with its scope and without the `+`.

**PR titles and agent guidance**

- R8. `feat` is for any gain that the user can see: a new capability, an option, information, or a clear improvement to how something works. `fix` is for a defect or small polish (wording, spacing). `CLAUDE.md` states this rule. The strict "new capability only" test and the PR #8 example are removed.
- R9. The required `PR title` check accepts `+` after the type or scope of a `feat` title and rejects it on all other types.
- R10. Before an agent opens a PR whose type is `feat`, it asks the user "patch or minor?" with a blocking question, and gives its recommendation and one reason. When the user chooses minor, the title gets `+`. When a PR title becomes `feat` later (a retitle), the agent asks then. When the user already said patch or minor for this change in the conversation, the agent does not ask again.
- R11. The agent does not ask for PRs of other types.
- R12. The squash merge settings and the empty squash commit body do not change.

**Documentation and checks**

- R13. `docs/releases.md` describes the new version rule, the marker, the headings and the role of the release script. `CONCEPTS.md` (Release) agrees with it.
- R14. Automated tests run the real git-cliff on `feat+:` and `feat(scope)+:` subjects, so that a later git-cliff upgrade cannot remove a `+` commit from the notes or the version decision without a test failure.

### Acceptance Examples

- AE1. **Covers R1, R6.** **Given** latest tag v1.1.0, **when** the commits since it are `feat(about): show the version` and `fix: typo`, **then** the release is v1.1.1, with one item under "New features and improvements" and one under "Fixes".
- AE2. **Covers R2, R7.** **Given** latest tag v1.1.0, **when** the commits are `feat(capture)+: record system audio` and `fix: typo`, **then** the release is v1.2.0, and the notes show `capture: Record system audio` under "New features and improvements".
- AE3. **Covers R3.** **Given** the user asked for a major release, **when** the PR title is `feat!: change the storage format`, **then** the release goes from v1.4.2 to v2.0.0.
- AE4. **Covers R4.** **Given** only `docs` and `chore` commits since the latest tag, **then** no release occurs.
- AE5. **Covers R10.** **Given** an agent prepares a PR that adds a new export option, **when** it chooses the type `feat`, **then** it asks "patch or minor?" with its recommendation before it opens the PR. If the user chooses minor, the title is `feat(export)+: …`; if the user chooses patch, it is `feat(export): …`.
- AE6. **Covers R10.** **Given** an open PR titled `fix: …`, **when** the agent changes the title to `feat` during babysit, **then** it asks "patch or minor?" before it changes the title.
- AE7. **Covers R9.** **When** a PR title is `fix+: typo`, **then** the `PR title` check fails.

### Scope Boundaries

- No repository setting changes, and no marker in PR labels or in the squash commit body.
- No new Conventional Commit types.
- No separate manual workflow for a major release (see the Key Decision on `!`).
- Past releases and tags do not change.
- No changelog file in the repository.
- `release-windows.yml` does not change.

### Outstanding Questions

**Deferred to Planning**

- Where the R10 question goes so that each agent flow that opens or retitles a PR asks it: `CLAUDE.md` alone, or also repository-level Compound Engineering configuration.
- How the release script reads the original commit subjects: `git log` since the latest tag, or git-cliff's context output.
- The exact `headerPattern` for the `PR title` check (`amannn/action-semantic-pull-request` v6.1.1), including how it handles `!`.

### Sources / Research

All results below came from git-cliff 2.14.2, the version that CI pins (`.github/workflows/ci.yml`), in a scratch repository.

- `features_always_bump_minor = false` has no effect at 1.x: a `feat` still makes a minor.
- `bump_type = "patch"` gives the next patch for `feat`, `fix` and also `feat!`, and gives the latest tag (no release) when only skipped types follow it. So the script must detect `!` itself (R3, R5).
- With the standard rules, a `BREAKING CHANGE:` line in a commit body makes git-cliff return a major (v2.0.0). With `bump_type = "patch"`, git-cliff never returns a major, so that line cannot start one.
- Without a preprocessor, git-cliff skips `feat(capture)+:` with a parse warning (no release, not in the notes), but it parses `feat+:` as an unknown type `feat+`. A `commit_preprocessors` entry that removes the `+` before parsing makes both forms a normal `feat`, and the notes show them without the `+`.
- The notes template uses only the commit subject, so a commit body does not appear in the notes.
- Current behaviour: `cliff.toml`, `scripts/release-version.ts` (it takes git-cliff's version as it is), `.github/workflows/pr-title.yml`, `docs/releases.md`, `CLAUDE.md` (commit type rules).
