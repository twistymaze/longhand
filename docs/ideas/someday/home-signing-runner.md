---
title: Home signing runner for preview builds
description: "Build, sign, notarize and publish the preview build in a home VM that accepts only main jobs, in place of macos-latest"
area: distribution
added: 2026-10-10
source: "the self-hosted CI runners plan, deferred during ce-work on 2026-10-10"
related: [self-hosted-ci-runners, signing-runner-on-mac-mini]
---

# Home signing runner for preview builds

## Idea
A separate macOS VM at home runs the `preview` job. It is in its own runner group, restricted to `ci.yml` on `main`, and a job-started hook fails any job that is not a `main` push. `docs/plans/2026-10-10-1612-ci-self-hosted-runners-plan.md` keeps the design: R3, R11, R14, KTD7, KTD8 and U8.

## Why
The preview build on `macos-latest` takes about 10 minutes and uses billed minutes on each `main` push. A home VM would make it faster and free. The user deferred it to keep the first work on the PR checks, and to keep the signing keys on GitHub's runners until the check runners have worked for some time.
