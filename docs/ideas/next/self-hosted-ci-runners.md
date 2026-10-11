---
title: Self-hosted CI runners
description: "Run the macOS and Windows CI checks in home VMs on the agents VLAN, for faster CI and fewer billed minutes"
area: distribution
added: 2026-10-10
source: "a brainstorm on 2026-10-10 about slow CI and runner cost"
related: [windows-preview-builds-and-updates, ephemeral-ci-runner-vms, home-signing-runner, signing-runner-on-mac-mini, pre-public-repo-audit]
plan: docs/plans/2026-10-10-1612-ci-self-hosted-runners-plan.md
---

# Self-hosted CI runners

## Idea
Run the CI checks in VMs on the home network instead of on GitHub's runners: a macOS VM on the Mac Studio and a Windows VM on the Proxmox host. The preview build stays on GitHub's runners for now (see home-signing-runner). All VMs are on the agents VLAN, which has access to the internet only.

## Why
A change takes about 20 minutes of PR checks, 22 minutes of checks on `main` and a 10-minute preview build. Actions cost about $250 a month in October 2026. The Windows job is the slowest, which blocks a Windows preview build on `main`.
