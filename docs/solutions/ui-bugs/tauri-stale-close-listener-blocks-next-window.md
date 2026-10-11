---
title: A destroyed webview's close listener stopped the close of the next Settings window
date: 2026-09-25
category: ui-bugs
module: Settings window close (src/settings/closeSaves.ts, src-tauri/src/window.rs)
problem_type: ui_bug
component: frontend
severity: high
symptoms:
  - "The red close button of the Settings window did nothing"
  - "The problem occurred only sometimes: after an earlier Settings window showed the Summaries pane and then closed"
  - "A relaunch of the app removed the problem until the next time"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "tauri 2.11.6 / @tauri-apps/api 2.11.1 / macOS 26"
related_components:
  - tooling
tags:
  - tauri
  - on-close-requested
  - event-listener
  - window-close
  - destroy
  - settings-window
  - window-label
---

# A destroyed webview's close listener stopped the close of the next Settings window

## Problem

Sometimes the Settings window did not close when the user clicked its close button. The cause was a close listener from an earlier Settings window. That window was destroyed, but Tauri kept its listener record, and the record stopped the close of each later window with the same label.

## Symptoms

- The close button of the Settings window did nothing. No error showed, and the page's console showed nothing.
- It occurred only after this sequence: open Settings, show the Summaries pane (which registers a close listener), close the window, and open Settings again. The new window opens on the Status pane, which has no close listener.
- A relaunch of the app removed the problem.

## What Didn't Work

- The first diagnosis blamed the close handler: `flush()` does not settle, or `destroy()` throws. A time limit on the save wait is useful, but it does not fix this bug. The new window has no handler that could hang.
- A panic in a sync settings command that leaves an IPC promise pending was also a candidate. The settings manager already recovers from a poisoned lock, and no sync command waits on the main thread. Nothing in Rust hung.

## Solution

Remove the JS close listener **before** the page destroys its window (`src/settings/closeSaves.ts`, the `close` function of `install`):

```ts
// Remove the listener before the destroy. Tauri keeps the listener
// record of a destroyed webview, and that record stops the close of
// the next Settings window.
const removed = await within(Promise.resolve(stop?.()), stepLimit);
stop = undefined;
const destroyed = await within(win.destroy(), stepLimit);
```

Two backstops cover the cases where the page cannot remove its listener (a page crash, a reload, a hung script):

- If `destroy()` fails, the page calls `close_settings_window` (`src-tauri/src/commands/window.rs:20`). That command destroys only the calling window, so a late call cannot close a newer Settings window.
- On each Settings close request, Rust starts a timer (`src-tauri/src/window.rs:141`, `SETTINGS_CLOSE_LIMIT` = 6 s, longer than the page's longest close of 5 s). If the same window is still open when the timer ends, Rust destroys it. A count of built windows stops the timer from closing a window that opened after the request.

The fix was confirmed in the notarized app (archived PR 9).

## Why This Works

In tauri 2.11.6, the JS listeners are held in one map keyed by **webview label** (`tauri-2.11.6/src/event/listener.rs:63`). Only `unlisten_js` removes an entry (`listener.rs:239`). Nothing removes the entries of a webview when the webview is destroyed.

When a window gets a close request, Tauri checks that map by label. If it holds a close-requested listener, Tauri prevents the close and sends the event to the page (`tauri-2.11.6/src/manager/window.rs:170-174`):

```rust
WindowEvent::CloseRequested { api } => {
  if window.has_js_listener(WINDOW_CLOSE_REQUESTED_EVENT) {
    api.prevent_close();
  }
  window.emit_to_window(WINDOW_CLOSE_REQUESTED_EVENT, &())?;
}
```

`onCloseRequested` in `@tauri-apps/api` calls `destroy()` after the handler. A destroyed page runs no React effect cleanup, so the unlisten function never ran. The stale entry then matched the label of the next Settings window. Tauri prevented its close and sent the event to a listener id that no page had.

## Prevention

- **Unlisten before a destroy.** Any page that registers `onCloseRequested` and then calls `destroy()` itself must call the unlisten function first. Effect cleanup does not run when the webview is destroyed.
- **Windows that reuse a label are at risk.** Settings has a fixed label (`SETTINGS_WINDOW`), so a stale record carries over to each new window. A window with a new label each time does not have this problem.
- **Test the second window, not the first.** The first close always worked. A test must close a window that had the listener, then send a second close request and check that no listener gets it (`src/settings/SummaryPrompts.test.tsx`, "a window close removes its listener before the destroy"). The Tauri test mock also keeps the record of a removed listener (it reads the wrong argument name), so that test expects one mock warning.
- **Log the close steps.** `src/lib/log.ts` sends webview errors and each close step to `Longhand.log` with the target `webview`. Before this change, the page's console errors never reached the log, so this bug left no evidence.
- After a Tauri update, read `event/listener.rs` again. If Tauri starts to clear listener records on webview destroy, the unlisten step is still correct but no longer needed.
