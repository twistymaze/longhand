---
title: "Patch by default with git-cliff: bump_type patch, a + marker, and a script that raises the version"
date: 2026-10-12
category: tooling-decisions
module: "Release version and PR title check (cliff.toml, scripts/release-version.ts, scripts/check-pr-title.sh, .github/workflows/pr-title.yml)"
problem_type: tooling_decision
component: development_workflow
severity: medium
applies_when:
  - "A change touches how the release version is computed, or the git-cliff bump settings in cliff.toml"
  - "A change adds, removes, or renames a release marker in a PR title (+ or !)"
  - "A change edits the PR title workflow or the semantic PR title action settings"
  - "A release has a version that the PR titles since the latest tag do not explain"
related_components:
  - ci
tags: [git-cliff, release, semver, conventional-commits, pr-title, github-actions, semantic-pull-request]
---

# Patch by default with git-cliff: bump_type patch, a + marker, and a script that raises the version

## Context

Before PR #12, git-cliff chose the version from the Conventional Commit types: `feat` made a minor release. Agents then had to choose between `feat` and `fix` by the release size, so small visible improvements went out as `fix`. The goal was: each release is a patch by default, `feat` stays usable for any visible gain, and a minor release happens only when the user chooses it. No new Conventional Commit types were allowed.

The first setup (2026-10-11) used standard semver on purpose. A "patch by default" rule was proposed then, for the Windows MSI version limit (minor at most 255), but the installer is NSIS, so that reason went away and the user asked for typical practice (session history). That setup also tried to make git-cliff remove `!` from messages before it computed the bump; the user rejected it as too complex (session history). PR #12 brings back patch by default for a different reason: the `feat`/`fix` choice, not a version limit.

git-cliff has no setting that makes a commit raise the minor version only when it carries an extra mark. `bump_type` fixes the bump level for all commits, and the `features_always_bump_minor` / `breaking_always_bump_major` settings are all-or-nothing by type. So the version decision has two parts.

## Guidance

**1. git-cliff gives the next patch; the release script raises it.** `cliff.toml:59` sets `bump_type = "patch"`, so `git-cliff --bumped-version` always gives the next patch when a release is due. `scripts/release-version.ts:59-68` (`raise`) reads the raw subjects since the latest tag with `git log --format=%s <tag>..HEAD` (`scripts/release-version.ts:96`) and raises the version:

```ts
const MINOR_MARKER = /^feat(\([^)]*\))?\+:/;                                   // feat+:, feat(scope)+:
const MAJOR_MARKER = /^(feat|fix|perf|refactor|revert|build)(\([^)]*\))?!:/; // ! on a release type
...
if (bumped !== nextPatch) throw new Error(`git-cliff gave ${bumped} after ${latestTag}, not the next patch ...`);
if (subjects.some((s) => MAJOR_MARKER.test(s))) return `${major + 1}.0.0`;
if (subjects.some((s) => MINOR_MARKER.test(s))) return `${major}.${minor + 1}.0`;
return nextPatch;
```

The throw is the guard: if someone changes `bump_type` or adds a bump setting, the release fails loudly instead of publishing a wrong version. git-cliff still decides whether a release is due and writes the notes.

Why this split and not the reverse (git-cliff gives semver, the script lowers a minor to a patch): to lower a minor, the script must find the correct patch number again, which repeats git work that git-cliff already did. Raising from a known next patch is arithmetic.

**2. A git-cliff preprocessor removes `+` before parsing.** `cliff.toml:25-27`:

```toml
commit_preprocessors = [
  { pattern = '^(\w+(?:\([^)]*\))?)\+:', replace = "${1}:" },
]
```

`feat+:` is not Conventional Commit syntax, so git-cliff must see `feat:` before its commit parsers run; the tests check that the notes show no `)+:` text. The release script reads the raw subjects with git log, not git-cliff's processed ones, so it still sees the `+`.

**3. The semantic PR title action cannot check the markers alone.** `.github/workflows/pr-title.yml:50-51` gives `amannn/action-semantic-pull-request` a custom `headerPattern` (`^(\w*)(?:\((.*)\))?\+?: (.*)$`) so `+` titles pass. But the action accepts `!` through its preset's breaking-change pattern, which no input changes (in v6.1.1 its conventional-commits-parser tries that pattern first, per this session's reading of the action source), so `docs!: …` passes even with a custom pattern, and the action has no option to reject `!` on a type that makes no release. A separate step runs `scripts/check-pr-title.sh` after a checkout (`pr-title.yml:53-61`). It rejects `+` on any type other than `feat`, `+` together with `!`, and `!` on `docs`, `ci`, `test`, `chore` and `style`. The title comes in through the `PR_TITLE` environment variable, never through `${{ }}` inside the script, so shell characters in a title are only text.

A PR can change its own copy of `check-pr-title.sh`. That does not change a release, because the release script on `main` reads the markers again.

**4. The marker grammar lives in four places.** `cliff.toml` (preprocessor), `scripts/release-version.ts` (`MINOR_MARKER`, `MAJOR_MARKER`), `scripts/check-pr-title.sh` (marker regex and no-release types), and `.github/workflows/pr-title.yml` (`headerPattern`). Each place has a comment about the marker rules, and `scripts/release-version.ts` names the other three. No test compares the four copies; change all four together.

## Why This Matters

- Squash merges make the PR title the commit subject, so the title is the only input the version can come from. A marker in the title keeps the choice visible in the PR and in `main`'s history.
- Without the throw in `raise`, a later `bump_type` change would let git-cliff's minor and the script's raise add up to a wrong version with no error. The throw fails the release step only because that step runs with `pipefail` ([the default run shell hides pipe failures](../workflow-issues/default-run-shell-hides-pipe-failures.md)).
- Without the separate check step, a `docs!:` title passes the required check. That is harmless for the version (docs makes no release), but it leaves a misleading marker in history.

- A release covers all commits since the latest tag, and two merges that queue together go into one release (session history). `raise` checks every subject, so the highest marker in the combined commits wins, the same rule git-cliff uses.

## When to Apply

- Before you change any bump setting in `cliff.toml`: keep `bump_type = "patch"` or change `raise` in the same PR.
- Before you add a new marker or a new release type: update all four places and the tests in `scripts/release-version.test.ts` and `scripts/check-pr-title.test.ts`.
- When a release version looks wrong: list the subjects since the previous tag with `git log --format=%s <tag>..<release commit>` and apply the rules above.

## Examples

| PR titles since the latest tag v1.4.2 | git-cliff gives | Release |
|---|---|---|
| `fix: …`, `feat: …` | 1.4.3 | 1.4.3 |
| `feat(capture)+: …`, `fix: …` | 1.4.3 | 1.5.0 |
| `feat+: …`, `fix!: …` | 1.4.3 | 2.0.0 |
| `docs: …`, `ci: …` | (no release) | none |
| `revert: "feat+: …"` | 1.4.3 | 1.4.3 (the subject starts with `revert`, so MINOR_MARKER does not match) |

The tests in `scripts/release-version.test.ts` run real git-cliff 2.14.2 for these cases.
