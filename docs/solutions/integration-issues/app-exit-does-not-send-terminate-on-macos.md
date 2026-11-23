---
title: "On macOS, AppHandle::exit does not send terminate:, so applicationShouldTerminate: does not see the tray Quit"
date: 2026-11-22
category: integration-issues
module: "Save of the notes at a quit (src-tauri/src/quit.rs)"
problem_type: integration_issue
component: background_job
severity: medium
symptoms:
  - "Text typed in a note less than 0.5 to 1 s before a quit was lost"
  - "The loss occurred on each quit path: tray Quit, app menu Quit, Cmd+Q, Dock Quit and the update restart"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "tauri 2.12.5, tauri-runtime-wry 2.12.3, tao 0.37.2 on macOS"
tags:
  - tauri
  - appkit
  - quit
  - notes-save
check: "none: first case. A clippy disallowed-methods ban on AppHandle::exit and AppHandle::request_restart outside quit.rs and relaunch.rs is a possible check; it is a code change and goes in its own PR"
---

# On macOS, AppHandle::exit does not send terminate:

## Problem

The note editor waits 500 ms before it sends a change. Each quit path ended the process before that send, so the last change was lost.

## Facts that are not clear from the code

- Tauri's `AppHandle::exit` makes tao call `[NSApp stop:]`, not `[NSApp terminate:]`. AppKit does not call `applicationShouldTerminate:` for it.
- The app menu Quit (`PredefinedMenuItem::quit`), Cmd+Q, the Dock Quit and a logout send `terminate:`. They do not go through Tauri code that the app controls.
- tao's app delegate has no `applicationShouldTerminate:` method. The app adds one with `class_addMethod`, then sets the delegate again so that AppKit reads the new method.

Thus one hook does not cover all quit paths. A hook in `applicationShouldTerminate:` does not see the tray Quit, the Windows close that quits or the update restart. A wrapper around `app.exit` does not see Cmd+Q or the Dock Quit.

## Solution

`src-tauri/src/quit.rs` has one function, `save_notes_then`, for each quit path:

- The tray Quit and the Windows close that quits call `quit::quit`.
- The update restart (`relaunch.rs`) calls `app.exit` or `app.request_restart` only inside `save_notes_then`, and starts its relaunch helper there too.
- On macOS, the added `applicationShouldTerminate:` calls `save_notes_then` and returns `NSTerminateLater`. The reply goes out from a spawned task (see `terminate-reply-ran-before-nsterminatelater.md`).

`RunEvent::ExitRequested` + `prevent_exit()` is not used (tauri-apps/tauri#13511).

## Prevention

- A new quit path calls `quit::quit` or `quit::save_notes_then`. A direct `app.exit` or `app.request_restart` call skips the notes save.
- Test each quit path in a signed build after a change to the quit code. Type a word and quit at once.
