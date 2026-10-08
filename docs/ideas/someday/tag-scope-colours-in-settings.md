---
title: Choose the colour of a tag scope
description: "Settings lets the user choose the colour of each tag scope (Person, Vendor), saved in the shared settings so it never changes"
area: interface
added: 2026-10-08
source: "Manual test of the note types and tags build, conversation 2026-10-08"
related: [suggested-tags]
---

# Choose the colour of a tag scope

## Idea
The Types and Tags pane in Settings shows each tag scope (the part before `::`, such as `Person` or `Vendor`) with its colour. The user can choose another colour from the label palette. Longhand saves the choice in the shared settings, so every computer shows the same colour.

## Why
The first version gives the scopes their colours from the scopes in the library: it sorts them, and each scope takes its usual colour, or the next free one if another scope has it. Two scopes get the same colour only when there are more scopes than colours. A new scope can change the colour of an existing scope in rare cases. A colour that the user chose never changes. The user asked for this choice.

## Notes / Current state
- The palette is `scopeTones` in `src/lib/Labels.tsx` (11 colours, light and dark). The automatic rule stays the default for a scope with no choice.
- Shared settings rules apply (`docs/solutions/integration-issues/shared-settings-in-a-synced-folder-rules.md`): a read never writes, and the last change wins.
- A rename of a scope (a tag rename that changes the part before `::`) should keep the colour, or ask.
