---
title: Save notes on quit - Plan
type: fix
date: 2026-11-22
topic: save-notes-on-quit
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Save notes on quit - Plan

## Goal Capsule

- **Objective:** Text that the user types in a note just before a quit is in the note file when the app starts again.
- **Means:** Before each quit, the backend asks the main window to send each waiting editor change and waits for the note saves, for at most a short limit (KTD1, KTD2). One `fix` PR.
- **Authority:** `CLAUDE.md` wins over this plan.
- **Stop conditions:** Stop and ask when a quit path cannot wait without `RunEvent::ExitRequested` + `prevent_exit()`, or when the manual test shows that a quit can hang.
- **Execution profile:** `ce-work` in a worktree from `origin/main`, one commit per unit, one PR with a `fix` title. The maintainer runs the manual test on a signed build and approves the merge.
- **Open blockers:** None.

---

## Product Contract

### Summary

The note editor waits 500 ms (at most 1000 ms) after a change before it sends the change to the backend. A quit does not send the waiting change, so the last text that the user typed is lost. This plan makes every quit path save the waiting change first.

### Problem Frame

`src/editor/NoteEditor.tsx` (borrowed from anarlog) sends a change after `SAVE_DEBOUNCE_MS` = 500 ms, or after `SAVE_MAX_WAIT_MS` = 1000 ms. It sends early on blur, when the editor unmounts, and through `notesSaved` (`src/lib/notesSaves.ts`) when the user leaves a note. A quit does none of these:

- The tray Quit item and the Windows main-window close call `app.exit(0)` (`src-tauri/src/tray.rs`, `src-tauri/src/window.rs`).
- The app menu Quit (`PredefinedMenuItem::quit`, `src-tauri/src/menu.rs`), Cmd+Q, the Dock Quit and a logout send `terminate:` to `NSApplication`. tao's app delegate has no `applicationShouldTerminate:`, so AppKit ends the app at once.
- The update restart calls `relaunch::request_relaunch`, which calls `app.exit(0)` or `app.request_restart()`.
- The `RunEvent::Exit` arm (`src-tauri/src/lib.rs`) stops processing and installs a downloaded update. It runs on the main thread, so it cannot wait for the webview.

Only the Settings window saves on close (`src/settings/closeSaves.ts`).

anarlog fixed the same defect in commit `0a9b72568f5b23a32121a9cf55fe401cca845842` (#8238): a confirmed quit asks the main window for a quick save, each editor sends only a queued local edit, the write queue gets 2 s, and Rust waits at most 3 s. Its Dock handler adds `applicationShouldTerminate:` to the app delegate. anarlog uses `RunEvent::ExitRequested` + `prevent_exit()`, which Longhand does not use (tauri-apps/tauri#13511, see `src-tauri/src/window.rs`).

### Requirements

- R1. A change that the user typed less than 500 ms before a quit is saved before the process ends.
- R2. These quit paths save first: the tray Quit, the app menu Quit and Cmd+Q, the Dock Quit, a logout or shutdown, the Windows main-window close that quits, and the update restart (which also installs at exit).
- R3. A quit never waits longer than a fixed limit, also when the webview does not answer or a save does not complete.
- R4. A second quit request while the first one waits ends the app at once.
- R5. The quit sends only the waiting change and the edit that a conflict holds, as a leave of the note does. It sends nothing when no change waits.
- R6. A quit with no main window, or with no open note, is not slower than before by more than the round trip to the webview.

### Scope Boundaries

- No change to the debounce times.
- No change to `RunEvent::Exit` work (processing shutdown, install at exit), except that it now runs after the save.
- Not in this PR: text in the Settings window at a quit (only a window close saves it now). File an idea if the manual test shows a loss.
- Not in this PR: a Windows logoff or shutdown. Windows ends the app with no `applicationShouldTerminate:` step; it needs a `WM_QUERYENDSESSION` handler.
- No user-visible message. A save that fails or passes the limit goes to the app log only (CLAUDE.md, UI design).

---

## Planning Contract

### Key Technical Decisions

- KTD1. **One Rust entry point for each quit.** A new module `src-tauri/src/quit.rs` has `save_notes_then(app, then)`. The first call emits a typed `QuitRequested` event to the main window and starts a task that waits for the answer command `notes_saved_for_quit`, for at most `QUIT_SAVE_LIMIT` = 3 s. Then it runs `then`. A later call while the first waits runs `then` at once (R4). With no main window, or when the emit fails, it runs `then` at once. The wait is on the async runtime, never on the main thread, so the webview and IPC continue to run. (session-settled: from the upkeep item and the parent session — chosen over `RunEvent::ExitRequested` + `prevent_exit()`, which tauri#13511 makes unsafe.)
- KTD2. **The webview sends and waits; Rust owns the limit.** The listener in `src/App.tsx` calls a new `allNotesSaved()` in `src/lib/notesSaves.ts`: it calls each registered send function, then waits for every running notes save of every note, in rounds as `notesSaved` does. Then it calls `notes_saved_for_quit`. The webview has no timer of its own: the 3 s Rust limit (KTD1) is the only limit. The send functions already send only a waiting change (`NoteEditor`'s `flush` returns when no document waits) and the held conflict edit (`NotesTab`), so R5 needs no new editor API. This is anarlog's `flushQueuedChange` + `flushDatabaseWritesWithin`.
- KTD3. **AppKit quits wait with `NSTerminateLater`.** Add `applicationShouldTerminate:` to tao's `TaoAppDelegateParent` class, as `src-tauri/src/zoom.rs` adds a window delegate method. The first call starts `save_notes_then` with a `then` that sends `replyToApplicationShouldTerminate: YES` on the main thread, and returns `NSTerminateLater`. AppKit then ends the app through `applicationWillTerminate:`, so `RunEvent::Exit` runs as before. A call after the save started returns `NSTerminateNow`. This covers the app menu Quit, Cmd+Q, the Dock Quit, and a logout or shutdown, which waits for the reply (anarlog's `TerminateCancel` + `app.exit(0)` cancels a logout). The `PredefinedMenuItem::quit` item stays. tao sets the app delegate in `EventLoop::new`, before `setup` adds the method, and AppKit can read a delegate's methods when it gets the delegate (see `src-tauri/src/zoom.rs`). So after the method is added, the code sets the same delegate on `NSApp` again.
- KTD4. **`app.exit(0)` callers use `quit::quit(app)`.** The tray Quit, the Windows close that quits (`CloseAction::Quit`), and `relaunch::request_relaunch` (update restart) go through `save_notes_then`. On macOS, `request_relaunch` starts the relaunch helper inside the `then`, just before `app.exit(0)`, so the helper's 5 s wait for the old process starts after the save. `relaunch_now` runs before the event loop, with no notes open, and does not change.

### Risks

- `NSTerminateLater` runs the run loop in modal-panel mode. tao's user-event source, the IPC and the timers are in the common modes, so the emit, the answer and `run_on_main_thread` run. The manual test checks each AppKit path; the app log shows the save and the reply.
- A hidden main window can slow its timers. The 3 s Rust limit ends the quit in each case (R3).
- tao can add its own `applicationShouldTerminate:` in a later version. `class_addMethod` then returns false: log a warning, and a test checks that the method is added.

---

## Implementation Units

### U1. Frontend: save all notes on a quit request

- **Goal:** The main window answers `QuitRequested` after it sends each waiting change and the saves complete.
- **Requirements:** R1, R3, R5
- **Dependencies:** U2 (the event and the command in the bindings)
- **Files:** `src/lib/notesSaves.ts`, `src/lib/notesSaves.test.ts` (new), `src/App.tsx`, `src/App.test.tsx`, `src/editor/NoteEditor.test.tsx`
- **Approach:** Add `allNotesSaved(limitMs)` next to `notesSaved`, with the same rounds over all notes. The `App` listener logs the start and the result and calls the command.
- **Test scenarios:**
  - A change typed 100 ms before the request (fake timers, the debounce not elapsed) is sent, and the answer comes after its save settles.
  - With no waiting change, nothing is sent and the answer comes at once.
  - A save that starts a next save (busy chain): the answer waits for both.
- **Verification:** `npm run typecheck`, `npm test`.

### U2. Backend: quit coordinator and quit paths

- **Goal:** Each quit path waits for the answer, for at most 3 s.
- **Requirements:** R2, R3, R4, R6
- **Dependencies:** None
- **Files:** `src-tauri/src/quit.rs` (new), `src-tauri/src/lib.rs`, `src-tauri/src/tray.rs`, `src-tauri/src/window.rs`, `src-tauri/src/relaunch.rs`, `src-tauri/src/commands/` (the answer command), `src/bindings.ts`, `src-tauri/PROVENANCE.md`
- **Approach:** KTD1, KTD3, KTD4. Register the event and the command in `specta_builder`, and add the delegate method in setup on macOS. Record the anarlog source in `PROVENANCE.md`.
- **Test scenarios:**
  - The first request emits and waits; the answer runs `then` once.
  - No answer: `then` runs after the limit (tokio paused time, no real delay).
  - A second request while the first waits runs `then` at once, and the later answer does not run it again.
  - The delegate method is added to a test class, and a class that already has it keeps its method.
- **Verification:** `cargo test -p longhand quit`, `cargo clippy`, `cargo fmt --all --check`, `export_bindings` diff.

### U3. Signed build and manual test

- **Goal:** The maintainer confirms that no quit path loses text.
- **Requirements:** R1, R2
- **Dependencies:** U1, U2
- **Files:** none
- **Approach:** A signed, non-notarized macOS build (`docs/development-workflow.md#manual-tests`).
- **Test scenarios:** For each path, type a word in a note and quit at once, start the app again, and check the word: tray Quit, app menu Quit, Cmd+Q, Dock Quit, and an update restart (a build with an older version offers the latest release). A logout is optional. If Cmd+Q or the Dock Quit loses the word, check the delegate step in KTD3 first. Also: quit with no note open is fast; a second Cmd+Q while the first waits ends the app.
- **Verification:** The maintainer reports that each scenario passes.

---

## Verification Contract

| Check | Proves |
|---|---|
| U1 tests | R1, R3, R5 in the webview |
| U2 tests | R3, R4, R6 and the delegate method |
| `cargo test --workspace`, clippy, fmt, `npm run typecheck`, `npm test`, `scripts/check-provenance.sh` | No regression, provenance recorded |
| U3 manual test on a signed build | R1, R2 on the macOS quit paths and the update restart. The Windows close that quits uses the same `quit::quit` call as the tray Quit; the U2 tests cover it. |

---

## Definition of Done

- U1 and U2 are committed on one branch, the PR has a `fix` title, and CI is green.
- The maintainer reports that the U3 test passed.
