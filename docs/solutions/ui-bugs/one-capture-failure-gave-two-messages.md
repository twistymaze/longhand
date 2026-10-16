---
title: One capture failure gave two messages, and an alert that cleared at once
date: 2026-10-15
category: ui-bugs
module: Failed-capture alert and the Stop result (src-tauri/src/managers/capture.rs, src-tauri/src/managers/alerts.rs, src/session/SessionPage.tsx)
problem_type: ui_bug
component: service_layer
severity: medium
symptoms:
  - A Stop that made the note Interrupted raised the failed-capture alert and also showed an error notice on the note page
  - The notice text showed the internal note id and the word "registered"
  - When the store write of the interrupt failed, the tray alert and the sound cue stopped at once
root_cause: logic_error
resolution_type: code_fix
framework_version: rust 1.94.0
related_components:
  - frontend
tags:
  - message-rule
  - alert
  - notification
  - stop-outcome
  - plain-language
---

# One capture failure gave two messages, and an alert that cleared at once

## Problem

PR #23 made the app tell the user when recorded audio cannot be added to a note. Code review found that the first version gave two messages for one event, with internal text, and that one path cleared the alert before it could play a cue. CLAUDE.md permits one message only when the user must act, or when the app could not correct the problem.

## Symptoms

- `after_stop` raised the alert (tray, OS notification, cue) and then returned the error. `stop_recording` maps the error with `to_string`, and `SessionPage` shows each error as a negative notice. So the user got two messages.
- The error text was `the audio of {id} could not be registered ({cause}); it stays`.
- The alert loop in `alerts.rs` ends when the stored state is not a failed state. When the `CaptureLost` write failed, the note stayed Capturing, so the loop ended on its first pass.

## Solution

- A Stop that makes the note Interrupted returns `Ok(StopOutcome::Interrupted { session_id })` (`stop_result` in `capture.rs`). The page shows nothing for an `Ok` result, so the alert is the only message. A Stop or Resume of a note that was already Interrupted raises no alert and returns the error, so the page notice is the only message.
- The error and the notification use fixed plain text with no id and no raw error: "Some recorded audio could not be added to the note" (`UNREGISTERED_DETAIL`). The id and the raw cause go to the log.
- The alert loop uses `alerts::alert_holds(state, open)`. It holds for an unacknowledged Interrupted note, and also for a note that the store says is Capturing while nothing in `CaptureManager` records it.

## Why This Works

Each event now has one owner of the message: the alert when the app changed the note to Interrupted, the page notice when the user's own action was refused. The alert loop no longer reads a missing store write as "the problem is gone".

## Prevention

- When a backend path raises an alert or notification, check what it also returns to the interface. A returned error is a second message.
- Never put `to_string()` of an internal error in UI text. Give the user a fixed plain sentence and log the cause.
- A loop that stops an alert when the stored state changes must handle a failed store write. Otherwise the alert ends at the moment it matters most.

## Related Issues

- [folder-notice-for-a-move-the-app-does-without-help.md](folder-notice-for-a-move-the-app-does-without-help.md): the same message rule.
