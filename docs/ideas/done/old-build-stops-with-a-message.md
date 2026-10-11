---
title: An old build stops with a message
description: "An old build that finds data from a newer build shows an alert and quits, instead of a crash at startup"
area: distribution
added: 2026-10-05
source: "a conversation on 2026-10-05: a startup crash on a work computer after a macOS update"
related: [separate-identifier-for-test-builds]
---

# An old build stops with a message

## Idea
When an old build starts and finds settings, a database or other data from a newer build, it shows the alert "a newer version of Longhand is necessary" and quits. It does not crash.

## Why
On 2026-10-05, macOS opened an old test build at login. The log stopped after `settings loaded (schema version 6)`. The crash report showed `abort()` in the `applicationDidFinishLaunching` notification: a panic in `setup_app` (`src-tauri/src/lib.rs`). The user saw only a crash, with no cause.

`startup::check_database` already shows an alert for a database that the build cannot open. In this crash, the old build stopped before that check had an effect, or a different step failed first.

## Outcome
This idea duplicated work that had already shipped. Archived PR 30 ([older build startup plan](../../plans/2026-09-30-1528-fix-older-build-startup-plan.md)) makes an older build rebuild a database that is not its own, and shows an alert for every other startup failure. The lesson is in `docs/solutions/runtime-errors/older-build-panicked-on-newer-database-at-launch.md`. The 2026-10-05 crash came from a test build from before archived PR 30 that macOS opened at login. Test builds now go in `~/Downloads/longhand-test/` and are deleted after the test ([Manual tests](../../development-workflow.md#manual-tests)). No further work is planned. The notes below are from before this check.

## Notes / Current state
- The panic message is unknown. The log does not record it, because the panic hook does not write to the log. First step: write panics to the log file, so that the next crash shows its cause.
- Find each startup step that can panic on newer data: the settings store, the shared settings in the storage folder, the models, and the capture setup. Change each panic to an error that `run` shows as an alert (KTD7).
- A version check of the settings schema and the database version can run before the other steps.
