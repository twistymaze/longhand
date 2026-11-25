---
title: Run an evaluation tool on real data before a PR, and ask before the PR
date: 2026-11-22
category: workflow-issues
module: ce-work shipping steps (code review, PR, babysit)
problem_type: workflow_issue
component: development_workflow
related_components:
  - tooling
severity: medium
applies_when:
  - "The work is a local tool that measures or compares approaches, for example crates/summary-lab"
  - "Implementation and local checks are done, and the next default step is ce-commit-push-pr"
symptoms:
  - "A PR is ready to merge, but the tool has run only on test fixtures or in a dry run"
  - "The report to the user gives CI results and no measurements from the tool"
root_cause: missing_workflow_step
check: "none: the difference between an evaluation tool and a product change is a judgment about the purpose of the work; no hook or lint can see it"
---

# Run an evaluation tool on real data before a PR, and ask before the PR

## What occurred

The summary agent plan (`docs/plans/2026-11-20-1211-chore-summary-agent-foundation-plan.md`) added `summary-lab`, a local tool that compares agent summaries with the current summaries on the user's notes. After the implementation, the agent did the default ce-work shipping steps: code review, simplify, PR #113, and babysit until the PR was ready to merge. The lab had run only as a dry run on the test fixture.

The user stopped the merge. The purpose of the work was to run the tool locally, compare the approaches, and choose one. None of that needs a merged PR.

## Why it occurred

ce-work and CLAUDE.md make a PR the default end of implementation work. The plan had a Conventional Commit type (`chore`) and units, so it looked like a normal change to ship. The agent did not ask what the user does with the result of the work.

## What to do

- Before the shipping steps, ask: is the value of this work the code, or the result of running the code? For a measurement or evaluation tool, it is the result.
- For such a tool, run it on real data after the local checks. Give the user the measurements. Do the code review on the branch if it helps, but do not open a PR.
- Ask the user before a PR for such a tool. Open one only when the user wants the tool on `main`, for example after an approach is chosen.
- A pushed branch with no PR stays on GitHub. Do not push the branch only to keep the work: it runs from the worktree.
