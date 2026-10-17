---
title: Notes lost structure or text after a save and an open, and typed shortcuts changed text
date: 2026-10-16
category: ui-bugs
module: Note editor markdown and input rules (src/editor/markdown/parser.ts, serializer.ts, src/editor/note/keymap.ts, src/editor/widgets/slash-command.tsx)
problem_type: ui_bug
component: frontend_stimulus
severity: high
symptoms:
  - Subtasks and tasks in a bullet list were deleted when a note opened again
  - A task list after a bullet list, or two lists with an empty line between them, came back as one list
  - Text with backslashes, underscores, <br>, leading spaces or line-start syntax after a line break came back as formatting, a heading or a code block
  - Typing my_var_name, ~5 min or [ ] at the start of a bullet removed characters or text
root_cause: logic_error
resolution_type: code_fix
framework_version: markdown-it 14, prosemirror-markdown 1.13
tags:
  - markdown-round-trip
  - commonmark
  - escape
  - input-rules
  - task-list
---

# Notes lost structure or text after a save and an open, and typed shortcuts changed text

## Problem

notes.md must keep what the user typed. Each save serializes the document, and each open parses it again, so any document that the editor can make must come back the same. Several rules did not hold that:

- The task list rule converted every item between a list's open and close tokens, nested items too, so a nested list held task items and the parser dropped it.
- markdown-it trims a paragraph, so an empty task `- [ ] ` reads as `[ ]` with no space after it.
- CommonMark reads two adjacent lists with the same marker as one list, also with blank lines between them. markdown-it puts the blank lines after a list in the list's line map, so they did not count as empty paragraphs.
- `escapeText` ran its passes in order, and a later pass read neighbours from the original string after an earlier pass made it longer.
- Escapes looked only at the text node: a backslash at its end escaped the next mark, text after a hard break was not at a line start, and 1 to 4 leading spaces were removed or made an indented code block.
- Input rules for single `*`, `_` and `~` fired inside words and around spaces, where markdown makes no mark. The task rule and the slash menu replaced the whole block with an empty task.

## Solution

- Convert only the direct items of a list (`level + 1`); each nested list gets its own pass. A task prefix is `[ ]`, `[x]` or `[X]` with whitespace or the end after it.
- A list after a list of the same group takes the other marker (`-`/`*`, `.`/`)`), with one blank line between them. A top-level list ends at its last non-blank line.
- Each escape pass reads the string it changes. A trailing backslash is doubled. Text after a hard break is at a line start. At a line start, the first leading space or tab is written as `&#32;` or `&#9;` when the line has more content. `<br>` text and links with spaces (angle brackets) are escaped.
- Single-delimiter input rules follow the flanking rules (no letter or number before the opening delimiter, no space inside the ends). Backspace runs `undoInputRule` first. A new task keeps the text of its block, and `[ ] ` in a bullet converts only that item.

## Prevention

- Test the round trip as JSON: build the document the editor makes, serialize, parse, and compare; do it twice.
- For each list or block rule, test it nested and next to a block of the same kind.
- An input rule must match only what the parser would read as the same mark.
- A line of only spaces is a blank line in any form; the settings prompt fields check the markdown for empty text, so do not escape it.
