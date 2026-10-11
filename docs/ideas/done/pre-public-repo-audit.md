---
title: Audit before the repository goes public
description: "Before the repository goes public, scan the git history for secrets and private files, and check the Actions logs and docs for personal data"
area: distribution
added: 2026-10-10
source: "the self-hosted CI runners brainstorm on 2026-10-10"
related: [self-hosted-ci-runners]
plan: docs/plans/2026-10-11-0917-docs-public-repository-plan.md
---

# Audit before the repository goes public

## Idea
Before the repository goes public, check everything that becomes visible: the full git history (secrets, and the private paths in `CLAUDE.md` in case one was committed once), the logs of past Actions runs, and the docs (real names, personal details). Also set the lockdown settings: PRs only from collaborators, issues off or collaborators only, Discussions and the wiki off, and approval for workflows from external contributors.

Home network details: on 2026-10-11 the history of `main` was written again to remove them, and the runner setup moved to the private repository `twistymaze/homelab-ci`. The old commits can stay on GitHub, and anyone with an old SHA can open them. Ask GitHub Support to remove the unreachable commits, or make a new public repository from a clean copy. Also delete the logs of the Actions runs of `worktree-self-hosted-ci-runners` from 2026-10-10 and 2026-10-11: their steps show the old commits. The R13 settings for self-hosted runners in a public repository are in the README of `twistymaze/homelab-ci`.

## Why
The user plans to make the repository public in about one month. A public repository cannot be made private again in a useful way, because anyone can clone it while it is public.

## Outcome
Done on 2026-10-11 with the public repository plan. The old repository is now the private archive `twistymaze/longhand-pre-public`, with Actions off and no secrets. A new `twistymaze/longhand` was made from a scanned copy of `main`, so the old commits and Actions logs are not public. [PR 1](https://github.com/twistymaze/longhand/pull/1) in the new repository added the MIT licence. The repository is public, with PRs and issues limited to collaborators, approval for workflows from all outside contributors, secret scanning and push protection on, and the wiki, Discussions and Projects off.
