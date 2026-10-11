---
title: Signing runner on the Mac mini
description: "Move the macOS signing and preview runner to the Mac mini M6 on the agents VLAN, so the Mac Studio can run two check VMs"
area: distribution
added: 2026-10-10
source: "the self-hosted CI runners brainstorm on 2026-10-10"
related: [self-hosted-ci-runners]
---

# Signing runner on the Mac mini

## Idea
The Mac mini M6 runs the signing and preview runner, alone on the agents VLAN. The Mac Studio then uses both of its macOS VM slots for checks.

## Why
Apple allows 2 macOS VMs at the same time on one Mac. The first setup uses one slot for checks and one for signing, so macOS check jobs run one at a time. Separate hardware also keeps the signing keys further from PR code. The Mac mini is not set up yet.
