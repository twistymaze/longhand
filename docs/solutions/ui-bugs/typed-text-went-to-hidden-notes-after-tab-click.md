---
title: Typed text went to the earlier notes cursor after a tab click
date: 2026-10-07
category: ui-bugs
module: "Note page tabs and notes editor (src/session/SessionPage.tsx, src/editor/NoteEditor.tsx)"
problem_type: ui_bug
component: frontend
severity: high
symptoms:
  - "After a change from Transcript to My Notes, typed text went to a different place than the visible cursor"
  - "Markdown shortcuts such as `* ` did not make a list, and Enter did not make a new line"
  - "Text typed while Transcript showed went into the hidden notes"
  - "A blue focus ring showed on the My Notes tab while the user typed"
root_cause: logic_error
resolution_type: code_fix
related_components:
  - testing_framework
tags:
  - prosemirror
  - focus
  - selection
  - tabs
  - webkit
  - happy-dom
---

# Typed text went to the earlier notes cursor after a tab click

## Problem

During a recording, the user changed from My Notes to Transcript and back. Then typed text went to the editor's earlier cursor, not to the visible cursor, and markdown shortcuts stopped. Fixed in PR #44.

## Symptoms

- Typed text went to a different place than the visible cursor. A `*` showed before the cursor.
- `* ` did not make a list. Enter, Space, Backspace and the arrow keys did not change the notes.
- Text typed while Transcript showed went into the hidden notes.
- The My Notes tab showed a focus ring. A click into the notes, or another tab change, sometimes stopped the problem.

## What Didn't Work

- The first hypothesis was a composition (IME or macOS inline prediction) that never ended. While `view.composing` is true, react-prosemirror ignores selection changes (`node_modules/@handlewithcare/react-prosemirror/dist/esm/ReactEditorView.js:68`) and input rules stop. A synthetic `compositionstart` in WebKit gave the same symptoms. But the symptoms also occurred with no composition, so this hypothesis was not necessary and was ruled out.
- happy-dom does not show the defect. It does not put typed text into an editor that does not have the focus. The unit test can only check the focus and the selection, not where WebKit puts the text.

## Solution

`src/session/SessionPage.tsx:252-287`:

- When My Notes hides, a layout effect keeps a copy of the selection range when it is in the notes panel, and removes the selection (`selection.removeAllRanges()`).
- A click on My Notes puts the range back and then calls `focus()` on the editor. The range is put back only when its nodes are still in the editor. A sync reload gives the editor new nodes.
- The tab list sends `click: true` for a click and `false` for an arrow key (`onTab={showTab}`). An arrow key keeps the focus on the tab, as the ARIA tabs pattern requires.

## Why This Works

The causal chain:

1. A click on a tab gives the tab button the focus. The DOM selection stays in the notes editor, also when the panel gets `hidden` (`src/session/SessionPage.tsx:799`).
2. WebKit puts typed text into the editable selection even when a button has the focus. The `beforeinput` event goes to the editor.
3. The react-prosemirror `beforeinput` plugin inserts the text at `view.state.selection`, not at the DOM selection (`beforeInputPlugin.js:238`, `insertText`). The editor does not have the focus, so it does not read selection changes into its state. Thus the text goes to the earlier cursor.
4. `keydown` goes to the focused tab button. The editor keymap does not run. Space activates the tab and does not insert a space, so the `* ` input rule never sees its space.

With no selection in the editor, typed text goes nowhere. With the focus in the editor, the keymap and the input rules run, and selection changes go into the editor state.

The range must be put back **before** `focus()`. When WebKit focuses an editable element that has no selection, it puts the cursor at the start. ProseMirror's focus handler puts its own selection back only after 20 ms (`node_modules/prosemirror-view/dist/index.js:3811-3821`). Before that, the `selectionchange` from the focus writes the start position into the editor state. In a WebKit check, `focus()` alone moved the cursor to the start of the notes.

## Prevention

- A panel that stays mounted while hidden must not keep the DOM selection. WebKit sends typed text to an editable selection, wherever the focus is.
- After a tab change, give the focus to the element that must receive the keys. A selection in an editor is not the same as the focus.
- The regression tests in `src/session/SessionPage.test.tsx`:
  - "a tab change takes the selection out of My Notes, and a click on My Notes gives the editor the focus again" checks `window.getSelection()` after the change to Transcript, and `document.activeElement` and the selection after the return.
  - "an arrow key to My Notes keeps the focus on the tab".
- Check focus and text input in WebKit, not in happy-dom or Chromium. The harness from [the earlier cursor defect](notes-cursor-moved-to-end-after-tab-change.md) works: a preview page that renders `SessionPage` with `mockIPC` (also mock `live_transcript`, with `recognition`, `parts`, `current` and `state`), vite, and `webkit.launch()` from `playwright-core@1.53.0`. Find the hidden editor with `.prosemirror-editor`, not by role. In WebKit on macOS, the Home and End keys do not move the cursor in a line.

## Related Issues

- [The My Notes cursor moved to the end after another tab showed](notes-cursor-moved-to-end-after-tab-change.md): the same tab change, and the decision to keep the My Notes panel mounted.
- [WKWebView does not focus a clicked button](wkwebview-click-does-not-focus-button-breaks-shortcut-capture.md): another WebKit focus defect. In the user's app, the tab button showed a focus ring, so it had the focus.
