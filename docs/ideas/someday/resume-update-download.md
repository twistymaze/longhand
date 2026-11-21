---
title: Resume a partial update download
description: "An update download that stops (quit, sleep, network drop) continues from where it stopped, not from zero"
area: distribution
added: 2026-11-21
source: "Upkeep review, 2026-11-21: anarlog f87c4d58 (#8166)"
related: []
---

# Resume a partial update download

## Idea
Longhand keeps a partial update archive on disk, with its signature, when the app quits. The next download asks the server for the remaining bytes with an HTTP Range request, and checks that the server sent that range. When the server ignores or limits the range, Longhand downloads the full archive. Only an archive with a correct signature becomes the cached update. Cache pruning waits while a download writes to the cache, and also removes old partial files.

## Why
The update archive is large, because it holds the native recognition runtimes. Longhand downloads updates in the background and installs them at quit. Today `src-tauri/src/managers/updates.rs` and `updates/archive.rs` keep the full download in memory and write it only when it is complete, so a quit, sleep or network drop starts the download again from zero. Users on slow or unreliable connections can fail to get an update.

## Notes / Current state
- anarlog f87c4d58 puts the download code in a new crate, `updater-core`, that `plugins/updater2` calls instead of `Update::download`. Longhand's update code came from `plugins/updater2/src/ext.rs`.
- The change adds a crate (or Longhand's own range-request code) and changes the update cache format, so it needs a plan.
- Take the pruning guard also: a check that prunes the cache while a download writes it can delete the file in use.
- Upstream: https://github.com/fastrepl/anarlog/commit/f87c4d586da9d6b3d64175aceaa95f4fa10ab41b
