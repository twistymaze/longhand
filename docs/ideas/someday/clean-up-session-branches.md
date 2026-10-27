---
title: Clean up pushed session branches
description: "A scheduled GitHub Actions job deletes claude/* branches that have no open pull request and whose changes are on main"
area: distribution
added: 2026-10-27
source: "a conversation on 2026-10-27, after a cloud session could not delete its own pushed branch"
related: []
---

# Clean up pushed session branches

## Idea
A scheduled GitHub Actions workflow (for example, once each day) lists the `claude/*` branches. It deletes a branch when both of these conditions are true:

- The branch has no open pull request.
- Each commit of the branch is on `main` (`git cherry main <branch>` shows only `-` lines), or the branch is older than a set age.

The job uses `GITHUB_TOKEN` with `contents: write`, so it needs no other secret.

## Why
A cloud session (claude.ai/code) starts on its own `claude/*` branch. The session's GitHub proxy rejects all branch deletions ([GitHub proxy](https://code.claude.com/docs/en/cloud-environments#github-proxy): "the proxy rejects branch deletions"). Thus a session can push a branch, but it cannot delete it. A branch that a session pushed with no pull request stays on GitHub until the user deletes it.

## Notes / Current state
- CLAUDE.md now tells agents to push a branch other than `main` only for a pull request. That prevents most of these branches. This job removes the branches that a session pushes by mistake.
- The GitHub setting "Automatically delete head branches" deletes a branch only after its pull request merges. It does not cover a branch with no pull request.
- A squash merge puts new commits on `main`, so `git cherry` shows `+` lines for a merged branch. Decide whether the job also deletes a branch whose merged pull request is closed, or only branches with no pull request.
- Decide whether the job deletes a branch that has commits not on `main`, or only reports it.
