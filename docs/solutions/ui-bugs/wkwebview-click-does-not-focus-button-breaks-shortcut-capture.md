---
title: "WKWebView does not focus a clicked button, so the shortcut capture control got no keys"
date: 2026-09-26
category: ui-bugs
module: "Settings shortcut capture (src/settings/ShortcutField.tsx)"
problem_type: ui_bug
component: frontend
severity: high
symptoms:
  - "In the signed macOS app, a click on a shortcut button showed \"Press keys…\", but a chord or Escape did nothing"
  - "A click on the other row's button while one row waited left both rows waiting"
  - "The Longhand log showed no resume and no new registration after launch, so no chord reached the backend"
  - "The unit tests and the Chromium Playwright preview passed"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "tauri 2 / WKWebView on macOS 26"
related_components:
  - tooling
  - testing_framework
tags:
  - wkwebview
  - webkit
  - focus
  - keydown
  - blur
  - shortcut-capture
  - happy-dom
  - playwright
---

# WKWebView does not focus a clicked button, so the shortcut capture control got no keys

## Problem

The Settings shortcut capture control puts its key and blur handlers on the capture `<button>` (`src/settings/ShortcutField.tsx:182-183`). Chromium focuses a button when the user clicks it. WebKit, which is the webview of Longhand's macOS windows, does not. The button never became `document.activeElement`, so the keys went to the document, and the button's `onKeyDown` and `onBlur` never ran.

## Symptoms

- In the signed app (`bun tauri build --bundles app`), a click on a shortcut button showed the capture state ("Press keys…" and the blue outline). A chord or Escape did nothing.
- A click on the other row's button while one row waited left both rows waiting. The first row did not end its capture, because its `onBlur` never ran.
- `~/Library/Logs/net.ewal.longhand/Longhand.log` showed no resume and no new registration after launch. No chord reached the backend.

## What Didn't Work

- **Unit tests.** `src/settings/ShortcutField.test.tsx` sends key events with `fireEvent.keyDown(button, …)` directly to the button. These tests pass whether or not the click focused the button.
- **The Playwright preview.** The preview runs in Chromium `headless_shell`, and Chromium focuses a button on a click. The preview showed correct behaviour.
- **Code review.** The reviewers examined when `onBlur` runs, for example on an app switch. No reviewer asked whether the click gave the button focus.

## Solution

Focus the button when capture starts (PR #10, `src/settings/ShortcutField.tsx:110-128`):

```tsx
const start = async () => {
  if (mode !== "rest") return;
  const mine = ++attempt.current;
  setRefusal(null);
  setMode("preparing");
  holding.current = true;
  const paused = holdPause();
  // Take the pause before the other row ends its wait, so its finish does
  // not resume the shortcuts.
  const other = waiting;
  waiting = endWait;
  other?.current();
  // WebKit does not focus a button on a click, and the keys go to the
  // focused element.
  button.current?.focus();
  await paused;
  if (attempt.current !== mine) return; // The capture ended during the pause.
  setMode("capturing");
};
```

The same fix added a module-level `waiting` slot (`src/settings/ShortcutField.tsx:39`). A row that starts to wait ends the wait of the other row directly. The code does not depend on a blur event for this. The row holds the shared pause (`holdPause()` counts it at once) before it calls `focus()`. So when the focus moves away from the other row's button, the blur of that button cannot resume the shortcuts.

A regression test checks the focus (`src/settings/ShortcutField.test.tsx:115-119`):

```tsx
test("the button takes focus, so the keys reach it", async () => {
  renderField(registered("CommandOrControl+Alt+Shift+L"));
  const button = await capture();
  expect(document.activeElement).toBe(button);
});
```

The test fails when the click does not move the focus to the button, which is what WebKit does. The user confirmed the fix in a signed bundle: a chord and Escape both work, and only one row waits at a time.

## Why This Works

A key event goes to the focused element. In WKWebView, a click does not focus a `<button>`, so a handler on the button receives no keys. The call to `focus()` gives the button the focus that the click does not give. After that, `onKeyDown` receives the keys, and a later blur (Tab, a click elsewhere, an app switch) runs `onBlur`.

## Prevention

- **Test keyboard behaviour in the signed bundle.** happy-dom, `tauri dev` in a browser, and the Chromium Playwright preview do not show WebKit's focus behaviour. Only the app's own WKWebView does.
- **Assert focus after the user action.** A test that sends key events to a stored element shows that the handler works. It does not show that the element receives the keys. When a control needs focus, assert `document.activeElement` after the click.
- **Focus explicitly.** A control that puts `onKeyDown` or `onBlur` on an element other than a text field must call `focus()` when it needs the keys. A click does not focus it in WKWebView.
- **In review, ask which element has focus.** A claim that an event runs "on the focused element" is correct only when the element really has focus.

## Related Issues

- `docs/solutions/ui-bugs/tauri-csp-img-src-blocks-inlined-app-icon.md`: another defect that only the signed bundle shows, because browser previews do not reproduce WKWebView behaviour.
