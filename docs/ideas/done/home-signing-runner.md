---
title: Home signing runner for releases
description: "Build, sign, notarize and check the macOS release in a home VM that accepts only main jobs, in place of macos-latest"
area: distribution
added: 2026-10-10
source: "the self-hosted CI runners plan, deferred during ce-work on 2026-10-10"
related: [self-hosted-ci-runners, signing-runner-on-mac-mini]
---

# Home signing runner for releases

## Idea
A separate macOS VM at home runs the `release-build` job. It is in its own runner group, restricted to `ci.yml` on `main`, and a job-started hook fails any job that is not a `main` push. `docs/plans/2026-10-10-1612-ci-self-hosted-runners-plan.md` keeps the design: R3, R11, R14, KTD7, KTD8 and U8. That plan names the job `preview`; `release-build` replaced it (`docs/plans/2026-10-11-1323-ci-releases-in-source-repository-plan.md`).

## Why
The release build on `macos-latest` takes about 10 minutes for each `main` push that is due for a release. A home VM with a warm Rust cache would make it faster. The user deferred it to keep the first work on the PR checks, and to keep the signing keys on GitHub's runners until the check runners have worked for some time. A published file must also not depend on the state of a self-hosted runner, so a home runner needs a clean build for each release.

## Outcome
Done another way in PR #60. The release build runs on the ephemeral VMs on the Mac mini (`MACOS_RELEASE_RUNNER=longhand-macos-vm`), not in a separate runner group. A new VM for each job gives the clean build that a published file needs. A signing rehearsal with a throwaway certificate passed on the VMs before the switch.
