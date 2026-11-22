---
title: "The reply to applicationShouldTerminate: ran before NSTerminateLater, because run_on_main_thread runs at once on the main thread"
date: 2026-11-22
category: integration-issues
module: "Save of the notes at a quit (src-tauri/src/quit.rs)"
problem_type: integration_issue
component: background_job
severity: medium
symptoms:
  - "A review found that the first Cmd+Q, app menu Quit, Dock Quit or logout could hang when the main window did not exist"
  - "Only a second quit request ended the app"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "tauri 2.12.5, tauri-runtime-wry 2.12.3, tao 0.37.2 on macOS"
tags:
  - tauri
  - appkit
  - quit
  - main-thread
check: "none: first case. The defect needs a real AppKit terminate sequence on the main thread, which a unit test cannot drive. The fix puts the reply in one function, queue_reply, that only a spawned task calls; the signed-build manual test of each quit path checks it"
---

# The reply to applicationShouldTerminate: ran before NSTerminateLater

## Problem

`src-tauri/src/quit.rs` adds `applicationShouldTerminate:` to tao's app delegate. The method starts the save of the notes and returns `NSTerminateLater`. When the save ends, the app sends `replyToApplicationShouldTerminate: YES` through `app.run_on_main_thread`.

When no main window exists, the save step runs its callback at once, on the main thread, inside `applicationShouldTerminate:`. `tauri-runtime-wry` runs a `run_on_main_thread` task at once when the caller is already on the main thread (`send_user_message`: `if current_thread().id() == self.main_thread_id { handle_user_message(...) }`). So the reply ran before the method returned `NSTerminateLater`. AppKit can ignore a reply that comes before `NSTerminateLater`, and then the quit waits for a reply that does not come.

## Solution

The callback starts a task on the async runtime, and that task calls `run_on_main_thread`. From a worker thread, `run_on_main_thread` puts the task in the event loop queue, so the reply always runs after the delegate method returns.

```rust
super::save_notes_then(app, |app| {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { queue_reply(&app) });
});
TERMINATE_LATER
```

## Prevention

- `run_on_main_thread` does not always defer. On the main thread it runs the task at once. When the order matters (a reply that must come after the current AppKit call returns), queue the task from another thread.
- Two independent reviewers (the correctness reviewer and a Codex cross-model review) found this. No unit test drives AppKit's terminate sequence, so the manual test of each quit path in a signed build is the check.
