---
title: A gitleaks step in CI
description: "Run gitleaks on each PR and push, so a secret that GitHub push protection does not know is found before it is public"
area: distribution
added: 2026-10-11
source: "the public repository plan on 2026-10-11 (U4 and U8)"
related: [pre-public-repo-audit]
---

# A gitleaks step in CI

## Idea
Add a CI job that runs `gitleaks` with `--redact` on the commits of each PR and each push to `main`. A finding fails the job.

## Why
The repository is public. GitHub secret scanning and push protection are on, but they find only the token formats that GitHub knows. The scan before the switch used gitleaks and found no secret; a CI step keeps that check after the switch.

## Notes / Current state
- Decide the job's runner: a GitHub runner is free for a public repository.
- A `.gitleaks.toml` allowlist may be necessary for the test fixtures and sample keys that the first scan accepted as harmless.
- The pull request trigger has no paths filter, so a job there also scans docs-only PRs.
