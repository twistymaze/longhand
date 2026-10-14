---
title: Built-in prompt text in the agent skill
description: "The longhand-notes skill holds the text of the two built-in summary prompts, so an agent can read them and make changed copies"
area: transcript-and-summary
added: 2026-10-14
source: "a conversation: an agent could name the built-in prompts but not read their text"
related: [summary-prompts, app-command-line-tool]
plan: docs/plans/2026-10-14-1345-docs-built-in-prompts-in-skill-plan.md
---

# Built-in prompt text in the agent skill

## Idea
Add a reference file to the `longhand-notes` skill with the full text of the two built-in prompts, "Topic summary" (`topic-summary`) and "Play-by-play summary" (`play-by-play`), and their title instructions. `references/prompts.md` tells the agent to read this file, and to copy a built-in prompt into a new prompt file with a new id when the user wants a changed version. A Rust test fails when the file and the text in `crates/summarize/src/prompts.rs` are different.

## Why
The built-in prompts have no file in the storage folder. An agent can see their names and ids, but not their text. It cannot explain what the default summary does, and it cannot make a changed version of a built-in prompt. The user must open Settings and use Duplicate.

## Notes / Current state
- Agent skill change only: no app release. The PR title type is `docs`.
- The skill updates from `main`, so a user with an older app can see newer text. The built-in prompts seldom change. Accepted for now (user decision, 2026-10-14). [app-command-line-tool](../someday/app-command-line-tool.md) is the later alternative.
- `crates/library/tests/skill_drift.rs` already checks that the skill and the app agree. The new test can go there.
- The fixed rules that Longhand adds to each prompt stay described in `references/prompts.md`, not copied.
