---
title: "The note editor prevents the default of every Return and Escape, so a dialog cannot use defaultPrevented"
date: 2026-10-26
category: ui-bugs
module: "Import sheet Notes field (src/home/ImportSheet.tsx)"
problem_type: design_pattern
component: frontend
severity: medium
applies_when:
  - "A dialog or page puts NoteEditor next to its own window keydown handler for Return, Cmd+Return or Escape"
  - "A handler outside the editor must know whether the editor used a key"
root_cause: unverified_assumption
resolution_type: code_fix
framework_version: "prosemirror-view 1.42.3 / React 19 / WKWebView on macOS"
related_components:
  - testing_framework
tags:
  - prosemirror
  - keyboard
  - escape
  - dialog
  - note-editor
---

# The note editor prevents the default of every Return and Escape, so a dialog cannot use defaultPrevented

## Context

The import sheet got a Notes field that is the My Notes editor (`NoteEditor`). The sheet has a window `keydown` handler: Escape cancels the sheet, and Cmd+Return (Ctrl+Return on Windows) starts the import. The editor uses some of the same keys: Escape closes the `/` menu, Return runs a `/` menu command, and Cmd+Return checks a task item (`Mod-Enter` in `src/editor/note/keymap.ts`).

The first plan said: "the sheet's handler checks `event.defaultPrevented` first", because the `/` menu and the task keymap prevent the default when they use a key. The feasibility review of the plan (PR #72) found that this cannot work. Escape would never cancel the sheet from the Notes field, and Cmd+Return would never import from it.

## Guidance

**Do not use `event.defaultPrevented` to learn whether the note editor used a key.** prosemirror-view calls `preventDefault()` when `handleKeyDown` returns true **or** when its own `captureKeyDown` returns true, and `captureKeyDown` returns true for every Enter (key code 13) and every Escape (27), with or without modifiers (`node_modules/prosemirror-view/dist/index.js`, `captureKeyDown`, and the `keydown` handler that calls it). Every Return and Escape that starts in the editor therefore reaches the window with `defaultPrevented == true`.

**Record the editor state in the capture phase instead.** A capture-phase window listener runs before the editor sees the key, so it can read the state that decides what the editor will do:

```ts
// src/home/ImportSheet.tsx
let editorUsesKey = false;
const before = (e: KeyboardEvent) => {            // capture phase
  editorUsesKey = false;
  if (!inEditor(e.target)) return;
  const slashMenuOpen = document.querySelector(SLASH_MENU) !== null;
  if (e.key === "Escape") editorUsesKey = slashMenuOpen;
  else if (isSubmitKey(e)) editorUsesKey = slashMenuOpen || selectionInTask();
};
const onKey = (e: KeyboardEvent) => {             // bubble phase
  if (e.isComposing) return;
  if (inEditor(e.target) ? editorUsesKey : e.defaultPrevented) return;
  // Escape cancels; Cmd+Return submits
};
window.addEventListener("keydown", before, true);
window.addEventListener("keydown", onKey);
```

- `SLASH_MENU` is `[data-editor-escape-consumer]:not([role=toolbar])`. The format toolbar has the same attribute but does not use Escape.
- `selectionInTask()` checks whether the selection anchor is inside `li[data-type="taskItem"]`.
- A bubble-phase check of the DOM is too late: the `/` menu's Escape handler closes the menu, and the menu can leave the page before the key reaches the window.
- Outside the editor, `defaultPrevented` still works. The toolbar's link field (`src/editor/widgets/format-toolbar.tsx`) prevents the default of its own Escape, and the TagPicker stops the propagation of its Escape.
- Skip a key whose `isComposing` is true. Escape that ends an input-method composition must not also cancel the dialog and lose its text. The code review of PR #72 found this, and Home's arrow-key handler already does the same.

## Why This Matters

The obvious check makes the dialog's shortcuts dead in the editor field and nowhere else. Tests that press the key in another field still pass, so the defect hides until a user types in the editor. When the check is inverted to ignore the editor's keys, the opposite failure follows: Cmd+Return with the `/` menu open both runs the menu command and submits, and Escape that closes the `/` menu also closes the dialog and discards the notes.

## When to Apply

- A new dialog, sheet or page that embeds `NoteEditor` (or `MarkdownField`) and also handles Return, Cmd+Return or Escape at the window or document level.
- A new editor key binding or widget that uses Return or Escape: add its state to the capture-phase record of each such dialog.

## Examples

The tests in `src/home/ImportSheet.test.tsx` cover each case through the real editor, with no mock of ProseMirror:

- Escape after plain text in Notes cancels the sheet. With the old `defaultPrevented` check, this test fails.
- Escape with the `/` menu open closes only the menu; a second Escape cancels the sheet.
- Cmd+Return in a task item checks the task and does not import.
- Cmd+Return with the `/` menu open does not import.
- Escape with `isComposing: true` does not cancel the sheet.

To check that a test depends on the record, set the capture-phase result to `false` and run the tests: the task-item and `/`-menu tests must fail.

## Related

- [WKWebView does not focus a clicked button](wkwebview-click-does-not-focus-button-breaks-shortcut-capture.md): the reason the sheet handles its keys at the window, not on a focused element.
- [A combobox field inside a portal popup lost the Tab order](combobox-field-in-portal-popup-lost-tab-order-and-caret.md): another key-routing defect between a field and its container.
