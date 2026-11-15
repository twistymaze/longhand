---
title: First-run setup
description: "A short first-run flow: storage folder, microphone and audio permissions, recognition model download, optional AI provider"
area: interface
added: 2026-11-15
source: "upkeep review 2026-11-15: Handy v0.10.0 onboarding (cfcb650)"
related: [reset-stale-tcc]
---

# First-run setup

## Idea
A short flow on first start: choose the storage folder, grant the microphone and macOS audio-capture permissions, download a recognition model with a time estimate, and optionally set an AI provider.

## Why
Now a new user meets the permission prompts, the storage folder and the model download separately, and settings only write their defaults. The value increases with the number of new users, so build it before a wider public release.

## Notes / Current state
- Do it together with `reset-stale-tcc`.
- The same flow can show a screen that moves users of a retired catalog model to its replacement, as Handy does.
