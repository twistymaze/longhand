---
title: Weekly upkeep as a local routine
description: "Run the weekly dependency and upstream upkeep on the Mac Studio from a private repository, with a narrow automatic update policy and one risk policy for all PRs"
area: distribution
added: 2026-10-18
source: "conversation about the cloud upkeep routine and GitHub security features (2026-10-18)"
related: [move-javascript-packages-to-npm]
plan: docs/plans/2026-10-18-2151-chore-local-upkeep-routine-plan.md
---

# Weekly upkeep as a local routine

## Idea

Replace the cloud upkeep routine and its workflows with one Claude Desktop scheduled task on the Mac Studio. Its instructions and scripts are in the private repository `twistymaze/longhand-upkeep`. It opens PRs only for fix-only patch versions and high or critical security fixes, and writes everything else into the tracking issue as proposals. It reads Dependabot and code scanning alerts directly. Every PR gets the same dependency-age and new-advisory checks in the required `cargo deny` job.

## Why

The cloud session could not read alerts, other repositories' advisories or most GitHub Actions APIs, so each gap became a workflow. The cloud design also updated more than the maintainer wants, and applied stricter rules to the routine than to interactive sessions.

## Notes / Current state

The plan replaces `docs/plans/2026-10-18-1454-chore-dependency-and-upstream-upkeep-plan.md`. PR #43 closes without a merge.
