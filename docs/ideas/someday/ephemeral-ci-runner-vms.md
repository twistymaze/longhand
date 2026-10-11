---
title: A new runner VM for each CI job
description: "Start each CI job in a new clone of a clean VM image, so nothing that one job leaves can reach a later job"
area: distribution
added: 2026-10-10
source: "the self-hosted CI runners brainstorm on 2026-10-10"
related: [self-hosted-ci-runners]
---

# A new runner VM for each CI job

## Idea
Each CI job starts in a new clone of a clean VM image, and the clone is deleted after the job. A cache folder from the host can keep the builds warm.

## Why
The first runner VMs keep their state between jobs, for speed. If contributions open to people outside the collaborators, a job from one PR must not leave files or processes that a later job runs.
