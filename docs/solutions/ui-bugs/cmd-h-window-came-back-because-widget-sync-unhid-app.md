---
title: "Cmd-H did not keep the main window hidden, because the recording widget sync showed the widget again and that unhid the app"
date: 2026-10-14
category: ui-bugs
module: "Recording widget panel (src-tauri/src/widget.rs)"
problem_type: ui_bug
component: frontend
severity: medium
symptoms:
  - "On macOS, with a note open and the recording widget shown, Cmd-H hid the main window, and the window came back after approximately one second"
  - "After the user closed the recording widget with the x, Cmd-H kept the main window hidden"
root_cause: config_error
resolution_type: code_fix
framework_version: "tauri 2 / tao 0.35 / tauri-nspanel 2.1.1 on macOS"
tags:
  - nspanel
  - tauri-nspanel
  - cmd-h
  - app-hide
  - can-hide
  - recording-widget
---

# Cmd-H did not keep the main window hidden, because the recording widget sync showed the widget again and that unhid the app

## Problem

On macOS, Cmd-H (Hide Longhand) hid the main window only for approximately one second while the recording widget was on screen. Then the main window came back. Fixed in PR #16.

## Symptoms

- With a note open and the recording widget shown, Cmd-H hid the main window, and the window came back after approximately one second.
- When the user closed the recording widget with the ×, Cmd-H worked correctly.

## What Didn't Work

- **Skip the show while the app is hidden (`NSApp.isHidden`).** This option stops the loop, but the recording widget then hides together with the app. While the main window is hidden, the user has no recording indicator, which is the purpose of the widget. This option was not used.
- **Unit tests.** The cause is AppKit behaviour. The widget tests in `src-tauri/src/widget.rs` check only the choice logic (`step`, `placement`), so they cannot find this fault. Only a manual test in a signed build found it and verified the fix.

## Solution

The recording widget panel has `canHide = NO`. `panel::build` sets it before the first hide (`src-tauri/src/widget.rs:490`):

```rust
// Hide (Cmd-H) does not hide the pill. A show of a window in a
// hidden app unhides the app, so the sync would show the main
// window again in one second.
panel.as_panel().setCanHide(false);
panel.hide();
```

Cmd-H now hides only the main window. The recording widget stays on screen. A click on the Dock icon shows the main window again.

## Why This Works

The fault had four steps:

1. Cmd-H calls `NSApp hide:`. AppKit hides each window of the app that can hide, which included the recording widget panel.
2. While a note is open, the widget thread syncs each second (`RECONCILE`, `src-tauri/src/widget.rs:116`). The entry still has `shown = true`, so `sync` calls `place_and_show`.
3. A window of a hidden app reports `isVisible == NO`. So `place_and_show` finds the window not visible and calls `window.show()` (`src-tauri/src/widget.rs:635-636`). tao runs `show()` as `makeKeyAndOrderFront:`.
4. When a window of a hidden app comes to the front, AppKit unhides the app, so the main window comes back.

With `canHide = NO`, step 1 does not hide the panel. The panel stays visible, so step 3 does not call `show()`, and nothing unhides the app.

## Prevention

- A window that the app shows again on a timer, or after an event, must stay out of the app hide (`canHide = NO`). If not, the code must check `NSApp.isHidden` before the show. A show in a hidden app unhides all windows of the app.
- For each change to the recording widget, also test Cmd-H in the signed build, with a note open and the widget shown. Wait at least 3 s, because the sync runs once each second.

## Related Issues

- [CSS hover lagged in the recording widget](hover-lagged-in-a-panel-that-is-never-key.md): a different macOS-only behaviour of the same panel.
