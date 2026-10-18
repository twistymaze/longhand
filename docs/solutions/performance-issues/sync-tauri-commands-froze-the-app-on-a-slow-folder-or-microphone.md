---
title: Sync Tauri commands froze the whole app while a folder or a microphone was slow
date: 2026-10-17
category: performance-issues
module: Tauri commands (src-tauri/src/commands/*.rs), tray handlers (src-tauri/src/tray.rs) and their frontend callers
problem_type: performance_issue
component: service_layer
severity: high
symptoms:
  - "The app showed a beachball and no window redrew while a note folder in a synced or network folder was slow to read"
  - "The app stopped responding while a notes save held the store lock"
  - "A tray click on \"Open Longhand\" froze the app for up to 5 s while a microphone opened"
root_cause: thread_violation
resolution_type: code_fix
framework_version: tauri 2, rust 1.94.0
tags:
  - main-thread
  - tauri-command
  - spawn-blocking
  - store-lock
  - reply-order
  - stale-reply
---

# Sync Tauri commands froze the whole app while a folder or a microphone was slow

## Problem

Tauri 2 runs a `#[tauri::command] pub fn` with no `async` on the main thread. One main thread serves every window, all IPC replies and every redraw. Many commands in `src-tauri/src/commands/` were sync and did work that can wait for a long time (PR #36):

- They took the store lock, which a notes save holds while it writes `notes.md` into the home folder.
- They read the capture state (`CaptureManager::inner`), also through `ProcessingManager::waits()`, `CaptureManager::status()` and `with_recording_held`. A microphone open holds `inner` for up to the 5 s start limit.
- They read a note folder or the storage folder (`session_document` reads up to three files with a 10 s limit each; `session_detail` checked folders with no limit).
- They saved shared settings, and that save can wait for the storage folder.

The tray menu and tray icon handlers also run on the main thread, and "Open Longhand" read the capture status there.

## Solution

**A command that can wait is `async` and runs its body on a blocking thread.** Use `commands::blocking` (`src-tauri/src/commands/mod.rs`), which wraps `tauri::async_runtime::spawn_blocking`:

```rust
#[tauri::command]
#[specta::specta]
pub async fn session_document(
    capture: State<'_, Arc<CaptureManager>>,
    session_id: String,
) -> Result<SessionDocument, String> {
    let store = capture.shared_store();
    blocking(move || {
        // The store lock and the folder reads run here, not on the main thread.
        // …
    })
    .await
}
```

- Do not use `#[tauri::command(async)]` alone for such a body. The body then blocks a tokio worker, and the processing runner uses the same workers.
- The TypeScript signatures in `src/bindings.ts` do not change: tauri-specta already gives a `Promise` for a sync command.
- An `async` command that takes a borrowed `State<'_, T>` must return `Result`, or it does not compile. A command with a plain return type (`move_status`, `agent_instructions_status`) takes `AppHandle` and gets the state with `app.state()` inside the closure, so its return type stays.
- A command that changes only local settings can stay sync.
- In the tray, `show_main_window` stays on the main thread and runs first, so the window shows at once. The status read and the `ShowSession` emit then run in `spawn_blocking`.

**Off the main thread, a reply can arrive after the events that it caused.** A sync command's reply came back before any event that the command emitted. A blocking thread has no such order. So each caller of a moved command must drop a reply that a newer read or a newer event replaced:

- `TypeAndTags` gives each tag read a number and applies only the newest reply.
- `useMove` in `DestinationPicker` counts Move events. A status reply that started before a Move finished is dropped, and the page reads again when the shown Move still has no folder. The independent review found that, without this, an old reply put the folder of the finished Move on the next Move.

The status read of the recording bar (`useCapture`) still has no read number. Its 5 s status poll corrects an old reply.

## Prevention

- For a new command, ask: can it take the store lock, the capture state, a folder read, or a shared settings save, directly or through a helper? If yes, make it `async` with `blocking`.
- When you move a command off the main thread, check every frontend caller for a reply that can come after a newer read or event. Give the read a number, or count the events that can replace it.
- Code in a tray or menu handler runs on the main thread. Show the window there, and move every read that can wait to `spawn_blocking`.

## Related

- [../integration-issues/shared-settings-in-a-synced-folder-rules.md](../integration-issues/shared-settings-in-a-synced-folder-rules.md): "Run every read off the main thread."
- [settings-lock-held-across-a-slow-shared-save.md](settings-lock-held-across-a-slow-shared-save.md): no lock that the main thread needs during storage-folder I/O.
- [../logic-errors/runner-store-lock-before-suspended-check-deadlocks.md](../logic-errors/runner-store-lock-before-suspended-check-deadlocks.md): the lock order (capture state, then store) still applies on a blocking thread.
