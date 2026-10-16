---
title: A settings lock held across a slow shared save made local setting changes wait
date: 2026-10-16
category: performance-issues
module: Settings changes (src-tauri/src/managers/settings.rs update_with)
problem_type: performance_issue
component: service_layer
severity: high
symptoms:
  - With a slow or offline storage folder, a change of the alert sound, microphone, zoom or a shortcut did not take effect, and the app stopped responding
  - A quick off-and-on change of a shared setting returned success but saved nothing
root_cause: thread_violation
resolution_type: code_fix
framework_version: tauri 2, rust 1.94.0
tags:
  - lock
  - main-thread
  - shared-settings
  - ordering
  - condvar
---

# A settings lock held across a slow shared save made local setting changes wait

## Problem

`update_settings` held one lock while it saved the shared part of a change into the storage folder. A save into a slow network share or an online-only sync file can take a long time. Every other settings change, also a change of a local setting, waited for that lock, and so did code on the main thread. The app stopped responding.

## Solution

`update_with` in `src-tauri/src/managers/settings.rs`:

1. **Local part first, under the lock.** The change writes its local fields to `settings.json` while it holds the lock, then releases the lock.
2. **Shared save in turn order, without the lock.** Each shared change takes a turn number (`Updates`, `Turns`, a `Condvar`, the static `UPDATES`). A save waits only for the turns before it, so shared changes are saved in the order that the user made them, and local changes never wait for them. `TurnGuard` gives the turn back also when the save panics or returns an error.
3. **Overlay of pending patches.** A change made while earlier shared saves wait starts from the values that those saves will write (`with_pending`, `apply_shared`), not from the file. Without the overlay, a quick change back compares equal to the old file value, and the second change saves nothing.
4. **On a refused shared save, revert only this change's own values.** `without_local_changes` puts back only the local fields that still hold the value of this change. A field that another command changed in the meantime keeps that change. An error from the revert write is logged, and the shared error is returned.

An earlier version patched the local fields after the shared save. That lost a local change that another command made during the save, so the local write moved to the start.

## Prevention

- Never hold a lock that the main thread or a fast path needs during I/O into the storage folder. Reads and writes there have no time limit.
- When you remove a lock to keep the order of writes, give each write a turn, and compute each new value from the values that the earlier turns will write.
- A revert after a failure changes only the values that the failed change made and that nobody changed since.
