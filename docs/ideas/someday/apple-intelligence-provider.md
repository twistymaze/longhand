---
title: Apple Intelligence as an AI provider on Macs
description: "An AI Provider preset that summarizes with Apple's on-device model, so a Mac with Apple Intelligence needs no key, server or download"
area: transcript-and-summary
added: 2026-10-18
source: "User question during the manual test of PR #39 (Rig engine), 2026-10-18"
related: [ai-provider-presets]
---

# Apple Intelligence as an AI provider on Macs

## Idea

Add an "Apple Intelligence" preset to the AI Provider pane on Macs. The summary and the title use Apple's on-device model through the Foundation Models framework. Nothing leaves the Mac, and the user enters no key or address.

## Why

The user asked for it. It gives a private, free summary with no setup on a Mac that has Apple Intelligence turned on.

## Notes / Current state

- Rig (rig-core 0.44) has no Apple provider. The Foundation Models framework is Swift only (macOS 26 or later, Apple silicon, Apple Intelligence on), so the app needs a Swift bridge (for example a Tauri plugin) and a custom Rig `CompletionModel` that calls it. The summary code stays the same.
- The on-device model's context is small (about 4K tokens, to verify), so a long note always goes in many parts. The step names of PR #39 show those parts.
- Open: how the preset shows when the Mac cannot use it (older macOS, Intel, Apple Intelligence off); the context limit to report to the planner; the quality of a summary from a small model; Windows has no such preset.
