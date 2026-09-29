---
title: Before a PR, rebase when main has new code, then check what the rebase cannot show
date: 2026-09-28
category: workflow-issues
module: PR creation from parallel worktrees (docs/development-workflow.md, before a PR)
problem_type: workflow_issue
component: development_workflow
related_components:
  - testing_framework
  - tooling
severity: medium
applies_when:
  - "A branch in a worktree is ready for a PR, and PRs from other worktrees merged into main after the branch started"
  - "Right before ce-commit-push-pr or gh pr create"
  - "The branch removes or renames an identifier (a model id, an enum variant, a function) that new code on main can use"
symptoms:
  - "git log HEAD..origin/main shows commits that change more than docs"
  - "After the rebase, no conflict markers remain, but the build or tests fail on a name that the branch removed"
  - "New code on main compiles and passes its tests, but its text or logic is wrong after the change of the branch"
root_cause: missing_workflow_step
resolution_type: workflow_improvement
tags:
  - rebase
  - parallel-worktrees
  - pull-request
  - semantic-conflict
  - git-grep
---

# Before a PR, rebase when main has new code, then check what the rebase cannot show

## Context

The user often works on two or three ideas at the same time, each in its own worktree. PR #21 (keep five transcription models) was ready for a PR. During its work, PR #19 (model bundles) and PR #20 (Transcribe Again from Settings) merged into main: 21 commits. The user saw that main had moved and asked for a rebase before the PR.

The rebase stopped with conflicts in three files: `crates/pipeline/src/reprocess.rs` (main rewrote a test), `src/settings/Models.test.tsx`, and `src/session/SessionPage.test.tsx`. The resolution of the conflict markers was not sufficient. Two more problems had no conflict marker:

- New tests on main used model ids that the branch removed: `granite-speech-4.1-2b-plus-q5km` and `parakeet-tdt-0.6b-v2-q80` in `crates/pipeline/src/reprocess.rs` and `crates/pipeline/tests/runner.rs`, and `moss-transcribe-diarize-q80` in `crates/models/tests/recognition.rs`.
- `models::recognition::bundle()` (`crates/models/src/recognition.rs:306`), which PR #19 added, listed `SpeakerSource::Integrated`. The branch deleted that variant, so the build failed.

A `git grep` of the tree for the removed identifiers, after the rebase, found all of them.

A third problem did not fail the build. PR #20 added a refusal for a note with too many speakers. Its text ended "Choose another speaker source or model in Settings." After PR #21, only the speaker source sets the speaker limit, so another model cannot fix it. A read of the new Transcribe Again code on main found the stale text. It now ends "Choose another speaker source in Settings." (`crates/pipeline/src/reprocess.rs:129-132`).

After the PR opened, main got one more commit. It only moved files under `docs/ideas/`, so no rebase was necessary.

## Guidance

Right before you create a pull request from a worktree branch:

1. Fetch main (`git fetch origin main`) and list its new commits (`git log --stat HEAD..origin/main`).
2. Find out if a new commit changes more than docs. Use the docs definition of CI in `docs/development-workflow.md` ("Checks and CI"): all Markdown files except `PROVENANCE.md`, plus `docs/`, `.claude/`, and `.compound-engineering/`. Docs-only commits need no rebase.
3. If a commit changes more than docs, rebase onto `origin/main` and resolve the conflicts.
4. Search the tree (`git grep`) for each identifier that the branch removed or renamed. New code on main can use a removed name without a conflict, because the two sides changed different lines.
5. Read the new code on main in the areas that the branch changed. Look for text, assumptions, or behaviour that the change of the branch made incorrect. The build and the tests do not find this type of problem.
6. Run the local checks again. Results from before the rebase are not valid for the new tree. Manual test and real-audio results from before the rebase apply only to the code that they tested; say so in the PR.
7. Create the PR.

## Why This Matters

A rebase with no remaining conflict markers does not prove that the branch works with main. Git finds only changes to the same lines. It does not find:

- **Use of a removed identifier.** New code on main can use a name that the branch deleted in another file. The failure shows only in the build or in the tests, and without the local checks it shows first in CI.
- **A semantic conflict.** New code on main can compile and pass its tests but be incorrect after the change of the branch, for example a message that names an option that the branch removed. Only a read of the new code finds it.

A related problem with parallel worktrees is two branches that raise the same settings schema version: see [parallel-branches-bump-same-schema-version.md](parallel-branches-bump-same-schema-version.md).

## When to Apply

- Before each PR from a worktree branch, when other PRs merged into main during its work.
- The effort is small when main has only docs changes: steps 1 and 2 are sufficient.
- The risk is higher when the branch removes or renames identifiers, catalog entries, enum variants, or user-facing options.

## Examples

The check before the PR for PR #21:

```sh
git fetch origin main
git log --stat HEAD..origin/main    # 21 commits, with code changes: rebase
git rebase origin/main              # conflicts in 3 files
git grep -n -E "granite|moss-transcribe|tdt-0.6b-v2|SpeakerSource::Integrated"
# Found removed ids in main's new tests and SpeakerSource::Integrated in bundle().
# Read main's new reprocess.rs code: the speaker-limit text named "model".
# Ran all local checks again, then created the PR.
```

The commit on main after the PR opened only moved files under `docs/ideas/`, so it needed no rebase.
