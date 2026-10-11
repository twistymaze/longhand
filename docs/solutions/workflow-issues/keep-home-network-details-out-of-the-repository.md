---
title: Keep home network details out of the repository
date: 2026-10-11
category: workflow-issues
module: CI runner setup (docs, scripts and plans for self-hosted runners)
problem_type: workflow_issue
component: development_workflow
related_components:
  - infrastructure
  - documentation
severity: high
applies_when:
  - "Work sets up or changes something on the user's own machines or network: runner VMs, hosts, routers, VLANs, storage"
  - "A plan, idea, solution doc, script or commit message names a host, an IP address, a VLAN, a firewall rule, a storage pool or a hardware detail of the user's home"
  - "Before a commit or a push of docs or scripts that were written during host setup"
symptoms:
  - "Setup scripts and a setup guide in the repository named the home hosts, their addresses, the VLAN number and the router rules"
  - "The plan and idea files on main had the same details, so removing them needed a history rewrite of main"
root_cause: missing_workflow_step
resolution_type: workflow_improvement
tags:
  - privacy
  - self-hosted-runners
  - history-rewrite
  - private-repository
  - public-repository
---

# Keep home network details out of the repository

## Context

The repository is private now, and the user plans to make it public. During the self-hosted CI runners work (archived PR 54), the agent wrote the runner setup scripts, a full setup guide, and the plan results into the repository. These files named the home hosts, IP addresses, the VLAN number, the router rules, the storage names and the hardware. The agent treated them as normal project docs, because the repository rules list only some private paths (`sample.m4a`, `fixtures/`, `manual-tests/`, `.env`, idea screenshots). The user saw the files after many commits and said that they did not want these details in the repository.

The fix took much more work than writing the files correctly at the start:

- The setup scripts and the guide moved to a separate private repository (`twistymaze/homelab-ci`). The Longhand repository keeps only the workflow changes and a short note in `docs/development-workflow.md`.
- A regex substitution list replaced each detail in the plan and idea files with a generic term ("the Proxmox host", "the agents VLAN", "the router"). A manual pass then fixed the grammar that the substitutions broke and removed details that the list did not catch (a VLAN number in a sentence, DNS server addresses, router port numbers, a network interface name).
- The plan and idea files were already on `main`, so `main`'s history was written again. The `non_fast_forward` rule of the main ruleset was removed for the push and then restored. The push used `--force-with-lease` with the expected old SHA.
- The feature branch was built again on the new `main` with only the scrubbed files.

The old commits can still be opened on GitHub by their SHA, and the logs of old Actions runs still exist. `docs/ideas/someday/pre-public-repo-audit.md` lists both for the audit before the repository becomes public.

## Guidance

Treat each detail of the user's home network and machines as private data, the same as the private paths in `CLAUDE.md`:

- Host names, IP addresses and subnets, VLAN numbers and bridge names, router and firewall rules, DNS servers, storage pool names, VM IDs, and hardware specifications.
- Put scripts and guides that need these details in a private repository, or in local memory when they are only for the agent.
- In the Longhand repository, describe the setup in generic terms: "a macOS VM on the Mac Studio", "a Windows VM on a Proxmox host", "a VLAN that has internet access only". Name the switch that selects the runners (repository variables, the `hosted` input), not the network.
- Before a commit of docs or scripts that came from host work, search the staged files for the details. For example: `git diff --cached | grep -nE '\b(10|172|192)\.[0-9]+\.[0-9]+\.[0-9]+\b|VLAN ?[0-9]+'`, plus the host names of the session.
- Find a detail before it is on `main`. On a feature branch, a scrub and a force-push are enough. On `main`, the fix needs a history rewrite, a ruleset change, and a reset of every checkout.

## Why This Matters

A public repository cannot be made private again in a useful way: anyone can clone it while it is public. A map of the home network helps an attacker: it shows which hosts exist, which addresses they have, and which rules isolate the CI VMs. A history rewrite removes the details from branches, but GitHub keeps the old commits available by SHA, and Actions logs can show them.

## When to Apply

- During any host or network setup that a task does from this repository: runners, VMs, signing machines, build hosts.
- When a plan or idea records facts about the user's machines.
- At each commit of files that a host setup step produced.

## Examples

Before (a line in the plan on `main`):

```text
- R2. ... On <host name>, the VM's network adapter has VLAN tag <number>, as for <VM name>.
```

After:

```text
- R2. ... On the Proxmox host, the VM's network adapter has the tag of the agents VLAN, as for an existing agent VM.
```

Before (`docs/development-workflow.md` linked a guide in the repository with the full setup). After:

```text
The runner setup is in a private repository, because it contains details of the home network.
The repository variables `MACOS_RUNNER` and `WINDOWS_RUNNER` select the home runners.
```
