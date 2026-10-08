---
title: "A combobox field inside a portal popup lost the Tab order and the caret"
date: 2026-10-08
category: ui-bugs
module: "Home Tags filter menu (src/home/TagFilterMenu.tsx)"
problem_type: ui_bug
component: frontend
severity: low
symptoms:
  - "Tab or Shift+Tab from the Tags menu field closed the menu and the focus left the filter bar"
  - "A click in the Tags menu field did not move the caret, and a drag did not select text"
  - "The unit tests passed before the fix: no test pressed Tab or checked a press on the field"
root_cause: logic_error
resolution_type: code_fix
framework_version: "React 19 / tauri 2 / WKWebView on macOS"
related_components:
  - testing_framework
tags:
  - portal
  - popup
  - combobox
  - focus
  - tab-order
  - mousedown
  - preventdefault
  - floating-ui
---

# A combobox field inside a portal popup lost the Tab order and the caret

## Problem

The Home Tags menu (PR #46) shows a filter field and the tag list in one popup. The popup is rendered with `createPortal` into `document.body`, because the Home pane has `overflow: hidden` and would cut off a wide menu. The field is therefore after `#root` in the document. Two behaviours that are correct for a popup that holds only a list became defects when the field moved into the popup.

## Symptoms

- Tab from the field moved the focus past the end of the document, and Shift+Tab moved it to the last control in `#root`. The field's blur handler closed the menu with no focus return, so the keyboard user lost their place in the filter bar.
- The popup cancels `mousedown` so that a press on a row does not blur the field. The field is a child of the popup, so a press on the field was cancelled too: the caret did not move and a drag selected nothing.
- The tests passed, because no test pressed Tab in the field or checked the default action of a press on the field. A code review found both defects before the user did.

## What Didn't Work

- Copying the popup handlers from `src/lib/TagPicker.tsx`. TagPicker renders its field inline next to the opener and portals only the listbox (`src/lib/TagPicker.tsx:135` and `:171`). Its popup-wide `onMouseDown={(e) => e.preventDefault()}` (`src/lib/TagPicker.tsx:181`) is safe there, because the field is not inside the popup, and Tab from its field follows the normal document order.

## Solution

Both fixes are in `src/home/TagFilterMenu.tsx`.

Cancel a press only when it is not on the field (`src/home/TagFilterMenu.tsx:152-154`):

```tsx
onMouseDown={(e) => {
  if (e.target !== inputRef.current) e.preventDefault();
}}
```

Handle Tab in the field and give the focus back to the opener, as Escape does (`src/home/TagFilterMenu.tsx:189-194`):

```tsx
} else if (e.key === "Tab") {
  e.preventDefault();
  close(true); // close the menu and focus the Tags button
}
```

`close(true)` sets a flag that a layout effect reads after the popup closes, and the effect calls `focus()` on the button. The explicit call is necessary because WKWebView does not focus a clicked button (see the related doc below). The next Tab then continues from the Tags button in the normal order.

The same branch handles Shift+Tab, because it does not test `shiftKey`. Tests in `src/home/TagFilterMenu.test.tsx` press Tab (not Shift+Tab) and check that the button has the focus. They also dispatch a `mousedown` on the field and on a row, and check `defaultPrevented`.

## Why This Works

A portal changes the position of the popup in the document, not only on the screen. Tab order follows the document, so every focusable element inside a portal popup is outside the focus order of the bar that opened it. The opener must take the focus back when the user leaves the popup with the keyboard. The popup-wide `mousedown` cancel exists only to keep the focus in the field. A press on the field already keeps the focus there, so the cancel has no purpose for that target and only removes the caret behaviour.

## Prevention

- Prefer TagPicker's structure: keep the text field inline and portal only the list. Then neither problem occurs.
- If a field must be inside the portal popup, handle Tab and Shift+Tab in the field (close and focus the opener), and exclude the field from any popup-wide `mousedown` cancel.
- Test both: press Tab and assert `document.activeElement`, and dispatch a `mousedown` on the field and assert `defaultPrevented` is false.

## Related

- [WKWebView does not focus a clicked button](wkwebview-click-does-not-focus-button-breaks-shortcut-capture.md): the reason the focus return calls `focus()` explicitly.
- [Typed text went to the earlier notes cursor after a tab click](typed-text-went-to-hidden-notes-after-tab-click.md): another focus defect in a different surface.
