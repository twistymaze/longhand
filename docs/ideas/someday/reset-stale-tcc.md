---
title: Reset a stale macOS permission entry
description: "Before Longhand asks for the microphone or audio-capture permission, reset an old entry that no longer matches the app's signature"
area: capture
added: 2026-11-15
source: "upkeep review 2026-11-15: Handy tccutil reset before the permission request"
related: [first-run-onboarding]
---

# Reset a stale macOS permission entry

## Idea
When a permission reads as not granted, run `tccutil reset` for Longhand's Microphone (and audio-capture) entry just before Longhand asks. Handy does this for its Accessibility and Microphone entries.

## Why
After a reinstall with a different code signature, the old privacy entry no longer matches, and macOS does not ask again. The user cannot grant the permission from the app.

## Notes / Current state
- Developer ID signing keeps entries across normal updates, so the risk is mostly after a development or preview build.
- It touches permissions: test only in a signed bundle.
