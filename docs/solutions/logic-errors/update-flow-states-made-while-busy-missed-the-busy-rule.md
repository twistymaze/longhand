---
title: Update flow states made while busy missed the busy rule, and a failed install was only logged
date: 2026-10-17
category: logic-errors
module: update flow (src-tauri/src/managers/updates/flow.rs, src-tauri/src/managers/updates.rs)
problem_type: logic_error
component: background_job
severity: medium
symptoms:
  - "\"Relaunch Now\" in a ready question that the user opened during processing restarted the app while a stage ran"
  - "After a refused restart claim (a note is open), the update stayed in Installing and nothing showed it again"
  - "A download that failed during a recording showed its error over the recording"
  - "A ready build that a newer release replaced installed at quit when a recording had started"
  - "A failed install at quit or at launch was only logged, so Relaunch Now restarted into the old version again and again"
root_cause: logic_error
resolution_type: code_fix
tags:
  - updates
  - busy
  - state-machine
  - edge-triggered-input
  - install-failure
  - message-rule
---

# Update flow states made while busy missed the busy rule, and a failed install was only logged

## Problem

The update flow (`Flow::step` in `flow.rs`) holds notices while a note records or processes (busy). The manager (`UpdateManager::handle` in `updates.rs`) pushes `Input::Busy` only when busy changes: it compares with `was_busy`. A state that the flow makes while busy is already true therefore never gets the busy rule. Three paths reached that gap (PR #34):

- "Relaunch Now" in a ready question that the user opened with "Check for Updates…" while busy returned `Effect::Restart`.
- A download that failed while busy went to `Failed`, which is visible, and no busy edge followed to hold it.
- A restart that `claim_restart()` refused (it refuses when a note is open) left the flow in `Installing`. The old code sent a busy message and expected the busy edge to hold the build, but busy had not changed, and an open note that does not record or process is not busy at all. No later input left `Installing`.

A failed install at quit or at launch was only logged. The archive stayed saved, so the next launch asked the ready question again, and each "Relaunch Now" quit into the old version.

## Root cause and solution

**An edge-triggered input does not cover states made after the edge.** Every transition that can run while busy reads `facts.busy` itself:

- "Relaunch Now" on a ready question that no newer release replaced, while busy, goes to `ReadyHeld { confirmed: true }` and returns no effect. "Install and Relaunch" and "Try Again" still start the download while busy, because the user asked (R11 and AE8 of `docs/plans/2026-09-28-1240-feat-preview-builds-and-updates-plan.md`). Only the restart waits.
- `downloaded(Err)` while busy goes to the hidden `FailedHeld` state only when the download window was hidden (read `download_hidden` before it is cleared). A download that the user watches shows its error at once.

**A refused effect needs its own input, decided on the current facts.** `Input::RestartRefused` moves `Installing` to `ReadyHeld { confirmed: true }` when busy and to `Ready` when not busy. Replaying a busy input instead hides the cause and does nothing when busy did not change.

**Every held state needs an input that releases it.** `ReadyHeld` and `FailedHeld` are released by `Busy(false)`, "Check for Updates…" and the tray item (`Flow::release`).

**A held ready build records why it was held.** `ReadyHeld { confirmed }` is true when the user chose the build (from `Installing`, from `install_saved`, or from a "Relaunch Now" while busy) and false when only a recording held a shown question or an automatic notice. `install_at_quit` installs an unconfirmed held build only when no newer release replaced it, as it does for `Ready` and `WaitingForQuit`.

**A failed install is saved and shown once.** `install_at_exit` and `install_at_launch` write `install_failure` (`{ version, error, shown }`) to `updates.json`, also when a version waits for a quit but no `Update` is known. The review found two more gaps in the first version of this fix:

- The record was marked shown when the manager started, before any window showed it. A login launch opens no window, so a quit before the error showed lost it. Now `shown` is written only when the update window is visible with `Failed` for that version (`shows_failure`).
- After the user closed the error, the hourly check offered the same saved build again, and a quit then tried the failed install again. Now the flow keeps the failed version (`failed_install`): automatic notices skip it and `install_at_quit` gives none for it, until the flow enters `Installing` or `Downloading` again. "Check for Updates…" still shows the ready question, because the user asked.

`install_at_launch` (`lib.rs` ~246) runs before `UpdateManager::new` (~333) in the same setup, so a failure at launch shows in the same launch, and a failure at quit shows at the next launch. While a record names a build newer than the running one, the launch does not try the install again.

## Prevention

- When a flow gets an input only on a change (an edge), check each transition that can run after the edge: it must read the current fact itself.
- For each effect that the manager can refuse or that can fail, add an input that brings the flow out of the state that waits for the effect.
- For each hidden or held state, name the inputs that release it, and test each one.
- Mark a message as shown only when the window really shows it, not when the state that wants it is set.

## Related

- [../workflow-issues/manual-test-of-the-updater-needs-a-release-flag-build.md](../workflow-issues/manual-test-of-the-updater-needs-a-release-flag-build.md): the quit, launch and restart paths have no automated test; a manual test needs a release-flag build.
- [../ui-bugs/one-capture-failure-gave-two-messages.md](../ui-bugs/one-capture-failure-gave-two-messages.md): one owner per message.
