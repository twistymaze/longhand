---
title: "WebKit sends the Enter that confirms an IME composition with isComposing false, so text fields saved on it"
date: 2026-11-15
category: ui-bugs
module: "Keydown handlers of text fields (src/lib/keyboard.ts)"
problem_type: ui_bug
component: frontend
severity: medium
symptoms:
  - "With Japanese, Chinese or Korean input, the Enter that confirms a composition saved the note title and left the field"
  - "The same Enter added a tag, created a type, saved a setting or ran a / menu item"
  - "The three handlers that checked event.isComposing did not stop it"
  - "An Escape that cancels a composition in the New Type field closed the dialog"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "tauri 2 / WKWebView on macOS / prosemirror-view 1.42 / @handlewithcare/react-prosemirror 3.2"
related_components:
  - testing_framework
tags:
  - ime
  - composition
  - keydown
  - webkit
  - wkwebview
  - prosemirror
---

# WebKit sends the Enter that confirms an IME composition with isComposing false

## Problem

An input method (IME) for Japanese, Chinese or Korean uses Enter to confirm a composition. In Longhand that Enter also acted: the note title saved, a tag was added, a setting was committed. The handlers that checked `event.isComposing` did not stop it either.

## Root cause

WebKit (WKWebView on macOS) sends `compositionend` first, and then the keydown of the confirming Enter. At that time `isComposing` is already false. Only `keyCode` 229 shows that the input method used the key. Chromium (WebView2 on Windows) sends the keydown with `isComposing` true. So a check of `isComposing` alone works on Windows and fails on macOS.

## Solution (PR 97)

`src/lib/keyboard.ts` has `isImeComposing(event)`: `isComposing || keyCode === 229`. It accepts a DOM event or a React event. Each keydown handler that checks Enter returns at its start when it is true, so Escape and the arrow keys also go to the composition:

```tsx
onKeyDown={(e) => {
  if (isImeComposing(e)) return;
  if (e.key === "Enter") commit();
}}
```

`src/lib/keyboard.test.ts` reads every source file under `src/` and fails when an Enter check has no `isImeComposing(` call between the start of its handler and the check. Use the helper in each new keydown handler that checks Enter.

## Two traps

1. **ProseMirror plugin listeners run before ProseMirror's composition check.** `useEditorEventListener("keydown", ...)` of react-prosemirror runs as a DOM event handler, before `editHandlers.keydown` of prosemirror-view calls `inOrNearComposition`. So the `/` menu (`src/editor/widgets/slash-command.tsx`) needs the guard. The editor keymap (`src/editor/note/keymap.ts`) does not: ProseMirror skips it while `view.composing` is true, and on Safari for 500 ms after `compositionend`.
2. **An early return in a field lets the key go to the parent.** The New Type field ignored the composition Escape, but the keydown then went to the dialog panel's own handler, which closed the dialog. A parent keydown handler that acts on Escape or Enter needs the guard too, after its `stopPropagation()`. The scan test finds only Enter checks, so check parent Escape handlers by hand.

## Tests

`fireEvent.keyDown(field, { key: "Enter", keyCode: 229 })` gives the keydown that WebKit sends. `src/test/keys.ts` has `imeKeyDown(target, key)` for this. A field test sends it, checks that nothing saved, then sends a plain Enter and checks that the save occurs.

jsdom does not run the Safari branch of `inOrNearComposition`, so a test with the full editor keymap would insert a paragraph that the app does not insert. Test the plugin listener alone.

## Prevention

In a keydown handler of a text field, call `isImeComposing(e)` first. Do not use `e.isComposing` alone.
