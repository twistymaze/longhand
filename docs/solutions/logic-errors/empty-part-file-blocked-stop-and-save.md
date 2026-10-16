---
title: An empty .part file from a crash made a note impossible to stop or save
date: 2026-10-15
category: logic-errors
module: Partial-chunk recovery and the unregistered-audio check (crates/recorder/src/chunks.rs, crates/recorder/src/wav.rs, crates/pipeline/src/recover.rs)
problem_type: logic_error
component: service_layer
severity: high
symptoms:
  - Code review found that a crash in the first moment of a segment leaves a 0-byte .part file
  - Each Stop of that note failed, and the save stage refused with "some recorded audio is not part of the note yet"
root_cause: logic_error
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - crash-recovery
  - part-file
  - wav-header
  - bufwriter
  - unregistered-audio
---

# An empty .part file from a crash made a note impossible to stop or save

## Problem

PR #23 added a guard: a working folder is never deleted while the `audio-recovery` folder of the working folder holds audio that no segment row includes (`pipeline::recover::unregistered_audio`). The guard counted each `.part` file that `recorder::recover_partial_chunks` could not repair. A crash just after a segment opened leaves a 0-byte `.part`. That file holds no audio, but it blocked Stop and the save stage for good. Code review found this before the merge.

## Symptoms

- A 0-byte `.part` stays in the `audio-recovery` folder of the working folder after launch recovery.
- Stop returns the "audio could not be added" error each time. The save stage refuses the note.

## Solution

- `WavFile::create` (`crates/recorder/src/wav.rs`) writes the 44-byte header into a `BufWriter`. Nothing reaches the disk until the buffer fills or flushes, so a crash before the first flush leaves an empty file. `wav::repair` cannot read a header from it.
- `recover_partial_chunks` now removes a `.part` whose length is at most `wav::HEADER_BYTES`, before it calls `wav::repair`. Such a file holds no samples. When the removal fails, the file goes to `left` and stays.
- `unregistered_audio` counts each `.part` that stays, whatever its segment index. A segment row covers only the closed chunks of its segment. The earlier per-index check let a row of the same index hide a `.part` that holds samples, and the save stage then deleted the folder.
- `register_chunks` keeps a segment under a second, and its files, while a `.part` of that segment stays. `recorder::remove_segment` deletes the `.wav` and `.part` files of an index.

## Why This Works

A file no longer than the header cannot hold a sample, so removing it loses nothing. A longer file that the repair cannot read can hold samples, so it keeps the folder. The guard now asks "is there audio that no row covers" per file, not per segment index.

## Prevention

- A guard that blocks a user action because a file exists must ignore files that cannot hold data. Otherwise one empty file blocks the action for good.
- When a writer buffers its header, a crash can leave a file that is shorter than the header. Recovery code must handle a 0-byte file, a header-only file, and a file with a damaged header as separate cases.
- Call `unregistered_audio` and `recover_partial_chunks` only while no writer runs on the folder. The open chunk of a running writer is a `.part`, and recovery now deletes an empty `.part`.

## Related Issues

- [save-state-machine-crash-window-recovery.md](save-state-machine-crash-window-recovery.md): recovery must not destroy its own evidence.
- [../security-issues/audio-removal-trusted-synced-file-names-before-delete.md](../security-issues/audio-removal-trusted-synced-file-names-before-delete.md): check what a folder holds before a delete.
