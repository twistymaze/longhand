---
title: Audit before the repository goes public
description: "Before the repository goes public, scan the git history for secrets and private files, and check the Actions logs and docs for personal data"
area: distribution
added: 2026-10-10
source: "the self-hosted CI runners brainstorm on 2026-10-10"
related: [self-hosted-ci-runners]
---

# Audit before the repository goes public

## Idea
Before the repository goes public, check everything that becomes visible: the full git history (secrets, and the private paths in `CLAUDE.md` in case one was committed once), the logs of past Actions runs, and the docs (real names, personal details). Also set the lockdown settings: PRs only from collaborators, issues off or collaborators only, Discussions and the wiki off, and approval for workflows from external contributors.

## Why
The user plans to make the repository public in about one month. A public repository cannot be made private again in a useful way, because anyone can clone it while it is public.
