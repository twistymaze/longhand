---
title: The store plugin saved the defaults over a settings file that it could not read
date: 2026-10-16
category: integration-issues
module: Local settings file (src-tauri/src/managers/local_store.rs, src-tauri/src/managers/settings.rs)
problem_type: integration_issue
component: service_layer
severity: high
symptoms:
  - After a launch, all local settings were the defaults (storage folder, shortcuts, endpoint, microphone)
  - Home showed a different, empty list of notes, because the storage folder setting was lost
  - A crash during a settings save could leave a damaged settings.json
root_cause: wrong_api
resolution_type: code_fix
framework_version: tauri 2, tauri-plugin-store 2, rust 1.94.0
tags:
  - tauri-plugin-store
  - settings
  - atomic-write
  - data-loss
  - launch
---

# The store plugin saved the defaults over a settings file that it could not read

## Problem

Longhand opened `settings.json` through `tauri-plugin-store`. When the plugin cannot read or parse the file, it does not return the error. It starts with an empty store. The app then filled in the defaults and saved them. So one failed read (antivirus or a sync tool that holds the file, an I/O error, a file that a crash cut off) replaced all local settings with the defaults, and the user was not told. The plugin also writes the file in place, so a crash during a save could damage it.

## Solution

Longhand owns `settings.json` (`src-tauri/src/managers/local_store.rs`). The plugin is still used for `updates.json` only.

1. **Read with retries.** `read` tries `LAUNCH_TRIES` (5) times, `LAUNCH_DELAY` (200 ms) apart. A parse error is tried again too, because another app can still be writing the file.
2. **Ask before defaults replace a file.** When the read still fails at launch, `load_at_launch` shows an alert (`startup.rs`). **Quit** changes no file. **Use Default Settings** first keeps the old file as `settings.unreadable-<UTC>.json` (`set_aside`), then starts with the defaults.
3. **Write nothing before the launch read.** Code that runs before the launch read (for example the update install at launch) reads the file with no write. When that read fails, it uses the defaults in memory only.
4. **Write atomically.** `write` writes a temporary file, syncs it, and renames it over `settings.json`. The rename is the commit. It then syncs the folder with `session_save::fsync::sync_dir`, and only logs an error from that step. A folder cannot be opened with `File::open` on Windows, so do not sync a folder by hand. The launch read removes temporary files that a stopped write left (`remove_temp_files`).
5. **Change the values in memory only after the write.** A failed write leaves the old values in memory and on disk.

The update archive in `updates.json` uses the same temporary file, sync and rename steps, with the archive written last, so a quit during a download no longer leaves a cut-off update that the next launch offers as ready.

## Prevention

- Before you use a plugin for a file that holds user data, find out what it does when a read fails. A plugin that starts empty on a failed read must not own a file that the app saves.
- A file that holds user choices is never replaced with defaults without the user's choice, and the old file is kept.
- Every save of such a file writes a temporary file and renames it.
