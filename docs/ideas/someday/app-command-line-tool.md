---
title: Command-line tool in the app for agents
description: "A command in the installed app gives agents data that only the app has, such as the built-in prompt text of that app version"
area: library-and-sharing
added: 2026-10-14
source: "a conversation about built-in prompt text in the agent skill"
related: [built-in-prompts-in-the-skill]
---

# Command-line tool in the app for agents

## Idea
The installed Longhand app has a command that the agent skill can run, for example to print the text of the built-in prompts. The output always matches the installed app version.

## Why
The skill updates from `main`, but the app updates by release. A copy of app data in the skill can be newer or older than the user's app. See [built-in-prompts-in-the-skill](../done/built-in-prompts-in-the-skill.md).

## Notes / Current state
- The safe agent edits plan rejected this on purpose: "There is no command-line tool in the app, and the skill never looks for the app" (`docs/plans/2026-10-12-1827-feat-safe-agent-edits-plan.md`, user-directed). A skill that must find the installed app is fragile. Do this only when that reason changes.
