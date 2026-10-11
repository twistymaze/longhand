---
title: The My Notes cursor moved to the end after another tab showed
date: 2026-10-05
category: ui-bugs
module: "Note page tabs and notes editor (src/session/SessionPage.tsx, src/editor/NoteEditor.tsx)"
problem_type: ui_bug
component: frontend
severity: medium
symptoms:
  - "On a recording note, the user opened Transcript and then My Notes, and the cursor was at the end of the notes"
  - "Text that the user typed after the return went to the end of the notes, not to the position before the tab change"
root_cause: logic_error
resolution_type: code_fix
related_components:
  - testing_framework
tags:
  - prosemirror
  - autofocus
  - tabs
  - happy-dom
  - webkit
---

# The My Notes cursor moved to the end after another tab showed

## Problem

The My Notes panel stays mounted while another tab shows, so that the editor keeps its document, cursor and undo history. But on a recording note, or a note that the user just started, a return to My Notes put the cursor at the end of the notes. Fixed in archived PR 36.

## Symptoms

- The user opened Transcript and then My Notes. The cursor was at the end of the notes, and the editor had the focus.
- Text that the user typed then went to the end of the notes.
- On a saved note that the user did not just start, the cursor did not move. `autoFocus` was always false there.

## What Didn't Work

- A first unit test set a DOM `Range` with `window.getSelection().addRange()` and sent a `selectionchange` event. In happy-dom, that selection does not get into the ProseMirror state. The editor then wrote its own selection (at the end) back to the DOM. The test failed before the tab change, so it could not show the bug. A probe of the fix with that test also failed, and the hypothesis looked wrong until the test was examined.
- The editor DOM gives no access to the `EditorView` in `@handlewithcare/react-prosemirror`: `pmViewDesc` has no `view` field.

## Solution

`SessionPage` gave the notes editor `autoFocus` only while My Notes showed. Now the value does not change with the tab (`src/session/SessionPage.tsx:765`):

```tsx
// Before
autoFocus={tab === "notes" && (phase === "recording" || started)}
// After
autoFocus={phase === "recording" || started}
```

## Why This Works

`NoteEditor` renders `{autoFocus && <FocusAtEnd />}` (`src/editor/NoteEditor.tsx:155`). `FocusAtEnd` uses a `done` ref so that it puts the cursor at the end and focuses the editor one time only (`src/editor/NoteEditor.tsx:163-167`). That ref belongs to the `FocusAtEnd` instance. When the tab changed, `autoFocus` became false and `FocusAtEnd` unmounted. On the return, `autoFocus` became true, a new `FocusAtEnd` mounted with `done` false, and it moved the cursor to the end again.

The panel stays mounted, but a child that a prop shows or hides does not. A one-time effect in such a child runs again each time the prop changes from false to true. With `autoFocus` tied to the note's phase only, `FocusAtEnd` mounts when the notes load, and again only when Resume starts a recording. Resume must put the cursor at the end, so that remount is correct.

## Prevention

- A prop that starts a one-time action must not follow the visible tab when the panel stays mounted. Tie it to the event that must start the action.
- To test the editor selection in happy-dom, use a ProseMirror key binding, not a DOM `Range`. The regression test "My Notes keeps the cursor where the user put it after another tab shows" in `src/session/SessionPage.test.tsx` uses the select-all binding (Meta-a on macOS, Control-a elsewhere), checks that `window.getSelection().isCollapsed` is false, changes the tab two times, and checks the selection again. Before each assertion that a test needs, check that the test set up the state it measures.
- For a browser check of editor focus or selection, use the WebKit build that Playwright has cached (`~/Library/Caches/ms-playwright/webkit-2182`) with `playwright-core@1.53.0` in the scratchpad. It is closer to the app's WKWebView than Chromium. A preview page that renders `SessionPage` with `mockIPC` must also mock `live_transcript`, or the Transcript tab fails.

## Related Issues

- [WKWebView does not focus a clicked button](wkwebview-click-does-not-focus-button-breaks-shortcut-capture.md): another focus defect that Chromium did not show.
- [A failed expect on a happy-dom node blocked waitFor](../test-failures/failed-expect-on-happy-dom-node-blocks-waitfor.md): another limit of happy-dom in the frontend tests.
