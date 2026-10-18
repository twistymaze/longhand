---
title: Keep finished parts when a summary reply is cut off
description: "When the model cuts off the reply to one part, split again only the parts that are not done, keep the finished ones, and keep the progress from going back"
area: transcript-and-summary
added: 2026-10-18
source: "Manual test of PR #39 (Rig engine) with LM Studio and an 8K context, 2026-10-18"
related: [ai-provider-presets]
---

# Keep finished parts when a summary reply is cut off

## Idea

When a long transcript goes in parts and the model cuts off the reply to one part (`finish_reason: length`), Longhand doubles the reply reserve, splits the full transcript again into smaller parts, and starts again at part 1. The summaries of the finished parts are discarded.

Instead, keep the summaries of the finished parts. Split only the lines that are not done yet with the new reserve, and continue from there. The merge layer already combines summaries of different sizes.

## Why

In the manual test, a reasoning model (Gemma on LM Studio) used 1,434 of the 2,048 reply tokens to think, so the reply to part 2 was cut off. The run discarded about 2.5 minutes of finished work, and the summary progress went from 13% to 6%, because the new split has more parts. On a slow local model, the lost work is the largest cost, and a progress bar that goes back looks like a restart or a failure.

## Notes / Current state

- The recovery is in `crates/pipeline/src/summarize.rs` (the `CutOff` branch, `forget_layers_from`, `Plan::replace_from`). The same code was there before the change to Rig.
- The plan is saved as a checkpoint, so that a run can continue after the app restarts. The plan must record which parts used the earlier reserve and where the new split starts, so that a resumed run makes the same parts.
- With a larger context window (the user's fix in the test), the transcript usually goes in one request, and this path does not occur.
- Possibly also: the progress counts only increase within one run.
