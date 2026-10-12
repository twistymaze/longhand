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
- **Means:** git-cliff gives the next patch, and the release script raises it from a `+` or `!` marker in the commit subjects (Key Decision on git-cliff and the script; KTD1).
- **Product authority:** The user decides each minor and each major release. Agents recommend; they do not decide. The Product Contract wins on behavior; the KTDs win on mechanism.
- **Execution profile:** One PR with a `ci:` title, so it makes no release itself. Implement in a worktree from `origin/main`.
- **Stop conditions:** Stop and ask when git-cliff 2.14.2 behaves differently from Sources / Research, or when a requirement cannot be met without a repository setting change.
- **Open blockers:** None.

---

## Product Contract

### Summary

Each release-type commit makes a patch release by default. A `+` after the type in a PR title (`feat+:` or `feat(scope)+:`) makes a minor release. Before each `feat` PR, the agent asks the user "patch or minor?" and gives a recommendation. git-cliff stays the tool that parses commits and writes the release notes. The release script makes the final choice of version.

Product Contract preservation: changed R10, R11 and AE6, and added AE11 — the plan's scope summary, which the user confirmed, added three agent rules (an agent asks before it removes a `+` by changing the type; an unattended babysit counts as hands-off; a hands-off report names the patch). No other scope change.

### Problem Frame

Today `cliff.toml` follows strict semver: each `feat` makes a minor release (`docs/releases.md`, When a release is due). So `CLAUDE.md` makes agents use `feat` only for a new capability, and it gives PR #8 (new text and links in Settings → About) as an example of a `fix`. Agents must make that judgement for each PR. They put real improvements under `fix` so that no minor release occurs. Then the release notes list improvements under the wrong heading, and the minor number does not mark the releases that the user thinks are important.

Longhand is an app, not a library. No other software depends on its version number for compatibility. So strict semver gives no value here, but the user wants the minor number to mark the releases that they choose.

### Key Decisions

- **Patch by default; a minor release only when the user chooses it.** Governs R1, R2, R10. (session-settled: user-directed — chosen over an agent decision by a written test, and over no minor marker at all with `feat` keeping its minor release: the user wants control of the minor number, with the agent's recommendation.)
- **The agent asks at each `feat` PR, and only then.** Governs R10, R11. (session-settled: user-directed — chosen over asking only when the agent recommends a minor, and over a recommendation in the PR description that never blocks: the user wants to see each decision.)
- **Hands-off runs make only patch releases.** Governs R11. A hands-off run does not ask and never writes `+`. (session-settled: user-directed — chosen over hands-off runs that stop and wait at each `feat` PR: hands-off runs suit small, safe changes, and a minor release needs the user to drive the work.)
- **`feat` means any visible gain, and the headings follow.** Governs R6, R8. (session-settled: user-directed — chosen over "new things only" and over a broad `feat` with the current headings: `feat` no longer has a release cost, so it can describe improvements correctly.)
- **The minor marker is `+` after the type or scope.** Governs R2, R9. It is one character in the same place as `!`, and it has no other meaning. (session-settled: user-directed — chosen over `!` for minor, a `Release: minor` line in the squash commit body, and a `[minor]` word at the end of the title: the user wanted an unobtrusive marker, no new Conventional Commit type, and no change to the standard meaning of `!`.)
- **`+` is valid only on `feat`.** Governs R2, R9. A release with only fixes is never a minor. (session-settled: user-directed — chosen over `+` on any release type at the user's request: a minor release follows a feature.)
- **No new Conventional Commit types.** Governs R2, R9. (session-settled: user-directed — chosen over a new `minor:` type: the user does not want invented prefixes.)
- **git-cliff gives the next patch; the release script raises it.** Governs R5. With `bump_type = "patch"`, git-cliff returns only the latest tag (no release due) or the next patch. The script raises that version to a minor or a major from the markers, and it never lowers a version. git-cliff only reads the history and prints; it never creates a tag. (session-settled: user-directed — chosen over standard semver rules with a script that lowers a minor to a patch, and over a `cliff.toml`-only rule that renames `feat` before the version calculation.)
- **A major release stays a user request with `!`.** Governs R3. The user said a major release can be a manual operation. The current rule already is one: an agent writes `!` only when the user asks. So no separate manual workflow is needed. (session-settled: user-approved — proposed with that tradeoff in the summary, and the user confirmed it.)
- **The squash commit body stays empty.** Governs R12. The marker is in the title, so no repository setting changes.

### Requirements

**Version rule**

- R1. Each release-type commit (`feat`, `fix`, `perf`, `refactor`, `revert`, `build`) since the latest release tag makes a patch release by default.
- R2. A commit whose subject has `+` directly after the type or scope and before the colon (`feat+:`, `feat(capture)+:`) makes the release a minor release. `+` is valid only on `feat`.
- R3. A release-type commit (R1) whose subject has `!` after the type or scope makes a major release. `!` on another type has no effect. A `BREAKING CHANGE` footer has no effect.
- R4. When the commits since the latest release tag have different effects, the highest effect applies. `docs`, `ci`, `test`, `chore` and `style` commits make no release, as now.
- R5. git-cliff decides whether a release is due, gives the next patch version, and writes the release notes. The release script raises that version to a minor (R2) or a major (R3) from the original commit subjects.

**Release notes**

- R6. The release notes headings are "New features and improvements" (`feat`), "Fixes" (`fix`, `perf`) and "Other changes" (`refactor`, `revert`, `build`). A heading with no commits does not appear, as now.
- R7. A `feat+` commit appears in the notes in the same way as a `feat` commit, under its heading, with its scope and without the `+`.

**PR titles and agent guidance**

- R8. `feat` is for any gain that the user can see: a new capability, an option, information, or a clear improvement to how something works. `fix` is for a defect or small polish (wording, spacing). `CLAUDE.md` states this rule. The strict "new capability only" test and the PR #8 example are removed.
- R9. The required `PR title` check accepts `+` after the type or scope of a `feat` title and rejects it on all other types. It rejects `!` on `docs`, `ci`, `test`, `chore` and `style`.
- R10. Before an agent opens a PR whose type is `feat`, it asks the user "patch or minor?" with a blocking question, and gives its recommendation and one reason. When the user chooses minor, the title gets `+`. When a PR title becomes `feat` later (a retitle), the agent asks then. When the user already said patch or minor for this change in the conversation, the agent does not ask again. When an agent writes the title of a PR with `+` again, it keeps the `+` unless the user says patch. Before an agent changes the type of a `feat+` title, it asks the user.
- R11. The agent does not ask for PRs of other types. A hands-off run (lfg, an unattended babysit, or another flow that runs while the user is not present) does not ask and never writes `+`, so its release is a patch. Its final report says that the PR makes a patch release and that the user can add `+` to the title before merge.
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
- AE6. **Covers R10.** **Given** an open PR titled `fix: …` and the user present, **when** the agent changes the title to `feat` during babysit, **then** it asks "patch or minor?" before it changes the title.
- AE7. **Covers R9.** **When** a PR title is `fix+: typo`, **then** the `PR title` check fails.
- AE8. **Covers R3, R9.** **When** a PR title is `docs!: rewrite the guide`, **then** the `PR title` check fails. If such a commit reaches `main`, it makes no release.
- AE9. **Covers R10.** **Given** an open PR titled `feat(capture)+: record system audio`, **when** an agent writes its description and title again, **then** the title keeps the `+`.
- AE10. **Covers R11.** **Given** an lfg run that prepares a new export option, **then** it opens the PR as `feat(export): …` without a question, the release is a patch, and the final report says so.
- AE11. **Covers R11.** **Given** an unattended babysit run, **when** it changes a `fix: …` title to `feat`, **then** it writes no `+`, does not ask, and reports the change.

### Scope Boundaries

- No repository setting changes, and no marker in PR labels or in the squash commit body.
- No new Conventional Commit types.
- No separate manual workflow for a major release (see the Key Decision on `!`).
- No minor release from a hands-off run (R11).
- Past releases and tags do not change.
- No changelog file in the repository.
- `release-windows.yml` and `scripts/publish-release.ts` do not change. The publish step only checks that the version is higher than the latest release.
- Considered and not built: a rule for a `feat+` PR that is reverted before its release goes out. The release is still a minor, because the original commit is still in the range. This is rare, and the user can see the version in the release. Evidence that it happens would change this.
- Considered and not built: revert title guidance in `CLAUDE.md`. The anchored markers (KTD2) ignore a `revert:` title that quotes a `feat+` title, so no rule is needed.

### Sources / Research

All results below came from git-cliff 2.14.2, the version that CI pins (`.github/workflows/ci.yml`), in a scratch repository.

- `features_always_bump_minor = false` has no effect at 1.x: a `feat` still makes a minor.
- `bump_type = "patch"` gives the next patch for `feat`, `fix` and also `feat!`, and gives the latest tag (no release) when only skipped types follow it. So the script must detect `!` itself (R3, R5).
- With the standard rules, a `BREAKING CHANGE:` line in a commit body makes git-cliff return a major (v2.0.0). With `bump_type = "patch"`, git-cliff never returns a major, so that line cannot start one.
- Without a preprocessor, git-cliff skips `feat(capture)+:` with a parse warning (no release, not in the notes), but it parses `feat+:` as an unknown type `feat+`. A `commit_preprocessors` entry that removes the `+` before parsing makes both forms a normal `feat`, and the notes show them without the `+`.
- The preprocessor also changes git-cliff's own record of the commit (`raw_message` has no `+`). So the script reads the subjects with `git log`, which keeps the `+`.
- With no release tag, git-cliff returns `initial_tag` v1.0.0 even for `feat+:` or `feat!:` subjects.
- The notes template uses only the commit subject, so a commit body does not appear in the notes.
- `amannn/action-semantic-pull-request` v6.1.1 supports `headerPattern` and `headerPatternCorrespondence`. But its parser tries a built-in breaking-change pattern first, and no input changes it. So any listed type with `!` passes the action, including `docs!:`.
- Repository settings (checked with the GitHub API): the squash commit title is the PR title, the squash commit body is empty, and auto-merge is on.
- `AGENTS.md` is a link to `CLAUDE.md`, and the Compound Engineering configuration has no PR title setting. So the agent rules go only in `CLAUDE.md`.
- `docs/solutions/workflow-issues/default-run-shell-hides-pipe-failures.md`: the release version step pipes into `tee`, and `release-build` sets `shell: bash` so a script failure fails the step.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **The script computes the raised version from the latest tag, and checks git-cliff's answer first.** Covers R5 (and the Key Decision on git-cliff and the script). When a release is due and a tag exists, git-cliff's version must be the tag's patch plus 1; otherwise the script fails. Then the script computes the minor or major from the tag (1.1.0 → 1.2.0 or 2.0.0), not from git-cliff's output. Rationale: if `bump_type` is lost later, git-cliff could return a minor, and a raise on top of it would publish a wrong version. A published tag cannot be taken back.
- KTD2. **Markers are anchored, case-sensitive matches on raw subjects.** Covers R2, R3. Minor: the subject starts with `feat`, an optional `(scope)` with no `)`, then `+:`. Major: the subject starts with a release type, an optional `(scope)`, then `!:`. The script reads the subjects with `git log --format=%s <latest tag>..HEAD`, using the same tag that it chose as the latest release tag. Rationale: the anchor makes `revert: feat(capture)+: …` and `Revert "…"` a patch, and the same tag keeps queued, cancelled and re-run CI runs correct, because each run computes from all commits since the latest tag.
- KTD3. **No raise without a tag or without a due release.** Covers R5. With no release tag, the version is git-cliff's initial 1.0.0, and the script does not read subjects. When no release is due, the script does not read subjects either.
- KTD4. **`cliff.toml` uses `bump_type = "patch"` and one preprocessor.** Covers R5, R7. Remove `features_always_bump_minor` and `breaking_always_bump_major`. Add a `[git] commit_preprocessors` entry that replaces a type, optional scope and `+:` at the start of the message with the same text without the `+`.
- KTD5. **The PR title check keeps the action and adds one script step.** Covers R9. The action gets a `headerPattern` that accepts an optional `+` before the colon on any listed type, so `feat+:` parses. A new step runs `scripts/check-pr-title.sh` with the title in an environment variable. The script rejects `+` on any type except `feat`, and `!` on `docs`, `ci`, `test`, `chore` and `style`, each with a clear message. Rationale: the action cannot reject `docs!:` (Sources / Research), and its error for `fix+:` does not name the `+`. A script file can be tested, and an inline `${{ }}` in a `run:` block is a script-injection risk.
- KTD6. **The agent rules live only in `CLAUDE.md`.** Covers R8, R10, R11. `AGENTS.md` links to it, so Codex reads the same rules. The wording tells the agent to ask, then stop and wait, and not to open or retitle the PR in the same turn.

### Sequencing

U1 first, because the docs (U4) describe its behavior. U2 and U3 are independent of U1 and of each other. U4 last.

---

## Implementation Units

### U1. Release rule in `cliff.toml` and the release script

**Goal:** git-cliff gives the next patch, and the release script raises it from the markers.

**Requirements:** R1–R7, R14; AE1–AE4, AE8; KTD1–KTD4.

**Dependencies:** None.

**Files:**
- `cliff.toml`
- `scripts/release-version.ts`
- `scripts/release-version.test.ts`

**Approach:**
1. `cliff.toml`: apply KTD4. Rename the group texts to the R6 headings, keeping the `<!-- N -->` sort prefixes. Rewrite the comments that name the old rule and the old requirement IDs.
2. `scripts/release-version.ts`: add a subjects function to the `Tools` seam and to `systemTools`. Change the release decision to: get git-cliff's version; when due and a tag exists, check it (KTD1), read subjects (KTD2), and raise. Get the notes with the final version, as now. Update the header comment.
3. Keep the CLI output (`due=`, `version=`) unchanged, so `ci.yml` needs no change.

**Patterns to follow:** the existing `Tools` seam and `fakeTools`; the scratch-repo helpers `repository(steps)` and `facts(steps)` in `scripts/release-version.test.ts`; full-sentence test names.

**Test scenarios:**
- Covers AE1. Tag v1.1.0, then `feat(about): show the version` and `fix: typo`: due, 1.1.1, both headings in order.
- Covers AE2. Tag v1.1.0, then `feat(capture)+: record system audio` and `fix: typo`: 1.2.0, and the notes contain `capture: Record system audio` and no `+`.
- Tag v1.1.0, then `feat+: plain plus`: 1.2.0, and the notes contain `Plain plus`.
- Covers AE3. Tag v1.4.2, then `feat!: change the storage format`: 2.0.0.
- Tag v1.4.2, then `feat(capture)+:` and `fix!: …`: 2.0.0.
- Covers AE4. Tag v1.0.3, then only `docs` and `chore` commits: not due, notes null.
- Covers AE8. Tag v1.1.0, then only `docs!: rewrite the guide`: not due.
- Tag v1.1.0, then `feat(capture)+:`, then `fix: …` and `chore: …`: still 1.2.0.
- `feat+` before tag v1.1.1, and only `fix` after it: 1.1.2.
- Tag v1.1.0, then `revert: feat(capture)+: record system audio`: 1.1.1.
- Tag v1.1.0, then `fix+: typo` (a title that bypassed the check): 1.1.1.
- No tag, history includes `feat+:` and `feat!:`: 1.0.0, and the fake records no subjects call.
- The existing tests that expected a minor for a plain `feat` now expect a patch (a feat and a fix after v1.0.4 give 1.0.5).
- The heading-order test uses the new headings, and the fake's notes text uses "### Fixes".
- Fake: git-cliff returns a version that is not the tag's patch plus 1: the script throws.
- Fake: the subjects call fails: the script throws.
- Unchanged: the preview tag test, the build/refactor patch test, the CLI output test, and the missing git-cliff test still pass.

**Verification:** The release script tests pass with git-cliff 2.14.2, and the CLI output for a fix-only history is unchanged.

### U2. PR title check

**Goal:** The required `PR title` check accepts `feat+:` titles and rejects misplaced markers with clear messages.

**Requirements:** R9; AE7, AE8; KTD5.

**Dependencies:** None.

**Files:**
- `.github/workflows/pr-title.yml`
- `scripts/check-pr-title.sh` (new)
- `scripts/check-pr-title.test.ts` (new)
- `.github/workflows/ci.yml` (add the new test file to the release script tests step)

**Approach:**
1. Add `headerPattern` and `headerPatternCorrespondence` (`type, scope, subject`) to the action, per KTD5.
2. Add a checkout step (pinned to the same SHA as `ci.yml`) and a step that runs `scripts/check-pr-title.sh` with the title in `env:`. Add `contents: read` next to `pull-requests: read` in the workflow permissions, because the checkout needs it.
3. The script exits non-zero with one message line for each rule it fails, and exits zero otherwise.
4. Keep the job name `PR title`. Update the header comment.

**Patterns to follow:** SHA-pinned actions as in `ci.yml`; the `env:`-passed inputs in `ci.yml`; test style of `scripts/release-version.test.ts` (run the real script, check exit code and output).

**Test scenarios:**
- `feat: add export`, `feat(capture)+: record system audio`, `feat+: x`, `feat!: x`, `fix!: x`, `fix(capture): x` pass.
- Covers AE7. `fix+: typo` fails with a message that names the `+` and `feat`.
- `chore(x)+: y` and `feat+!: x` fail.
- Covers AE8. `docs!: rewrite the guide` and `ci!: x` fail with a message that names the `!`.
- A title with a quote, a dollar sign or a backtick is read as text, not run.

**Verification:** The new tests pass, `actionlint` reports nothing for both workflows, and this PR's own `ci:` title passes the check.

### U3. Agent rules in `CLAUDE.md`

**Goal:** Agents choose `feat` or `fix` by what changed for the user, ask about the release size at each `feat` PR when the user is present, and keep the user's choice.

**Requirements:** R8, R10, R11; AE5, AE6, AE9–AE11; KTD6.

**Dependencies:** None.

**Files:**
- `CLAUDE.md`

**Approach:**
1. Rewrite the `feat` bullet per R8. Remove "It makes a minor release", the "write the one new thing" test and the PR #8 example.
2. State that each release type makes a patch release, and that `+` on a `feat` title makes a minor.
3. Add the R10 question and its rules, with the KTD6 wording. Add the R11 hands-off rule and its report line.
4. Keep the existing `!` rule (major only when the user asks).

**Patterns to follow:** the existing bullet list of commit-type rules; ASD-STE100 wording.

**Test expectation:** none -- agent guidance text. Review the wording against R8, R10 and R11.

**Verification:** Each of AE5, AE6 and AE9–AE11 can be answered from the new text alone.

### U4. Release docs and glossary

**Goal:** The docs describe the new rule.

**Requirements:** R13.

**Dependencies:** U1, U2.

**Files:**
- `docs/releases.md`
- `CONCEPTS.md`

**Approach:**
1. `docs/releases.md`: replace the sentence that says no person chooses a minor release. Update release flow step 1 for the script's raise. Replace the version table and the headings table. In "Major releases", replace the empty-body reason with R3's rule. In "PR titles", describe `+` and the new check step, and add that a squash title edited in the merge dialog wins and no check sees it.
2. `CONCEPTS.md`, Release: say that git-cliff and the release script compute the version from the PR titles.
3. Search the repository for the old headings and "minor" wording, and fix what remains outside historical plans and done ideas.

**Test expectation:** none -- documentation.

**Verification:** No current doc says that `feat` makes a minor release, and the docs agree with `cliff.toml` and `scripts/release-version.ts`.

---

## Verification Contract

| Check | Command or action | Applies to |
|---|---|---|
| Release and title script tests | `bun test scripts/release-version.test.ts scripts/publish-release.test.ts scripts/check-pr-title.test.ts` (git-cliff 2.14.2 on PATH; sandbox off for git writes in temporary directories; never bare `bun test`) | U1, U2 |
| Workflow lint | `actionlint .github/workflows/pr-title.yml .github/workflows/ci.yml` | U2 |
| Provenance | `scripts/check-provenance.sh` | all |
| Full CI | the PR's checks, including `PR title` on this PR's own `ci:` title | all |
| After merge | the next `feat` PR: its release is a patch, or a minor when its title has `+` | U1, U2 |

Read [Checks and CI](docs/development-workflow.md#checks-and-ci) before validation and before the push. No Rust code changes, so cargo checks are not needed unless CI requires them.

---

## Definition of Done

- U1–U4 are complete, and their verification passes.
- No abandoned or experimental code stays in the diff.
- The PR title is a `ci:` Conventional Commit, so the change makes no release.
- The PR is open, and `ce-babysit-pr` has run until it is ready to merge.
