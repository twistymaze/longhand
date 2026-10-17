---
title: The free-space check asked every mounted volume, also on the thread that writes the audio
date: 2026-10-17
category: performance-issues
module: Free-space lookup (crates/recorder/src/storage.rs available_space)
problem_type: performance_issue
component: service_layer
severity: high
symptoms:
  - "A network volume that does not answer (a sleeping NAS, a stale SMB or NFS mount) could block the recording writer for longer than the 10 s capture ring"
  - "Dropped frames become silence in the recording, with only a log warning"
  - "The status, Resume, a save and the storage folder check could wait on the same volume"
root_cause: thread_violation
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - free-space
  - disk-floor
  - network-volume
  - sysinfo
  - objc2
  - windows
  - unc
  - recording
---

# The free-space check asked every mounted volume, also on the thread that writes the audio

## Problem

`recorder::storage::available_space` and a copy in `src-tauri/src/managers/destination.rs` called `sysinfo::Disks::new_with_refreshed_list()` and took the deepest mount point that contains the folder. On macOS, sysinfo 0.38.4 asks every mount from `getfsstat` for its capacity keys, and it skips non-local volumes only after those calls return. The recorder calls the lookup every 5 s (`STORAGE_CHECK_SECONDS` in `ChunkedWriter::write_within_chunk`) on the thread that writes the chunks. The capture ring holds 10 s (`RING_SECONDS` in audio-cpal), so a stall longer than that drops microphone frames.

The sweep finding was not reproduced: no test had a mount that does not answer, and it was not proven that macOS blocks for more than 10 s. It was fixed because a lookup for one folder must not depend on unrelated volumes (PR #32).

## Solution

`available_space(dir)` asks the operating system only for the volume that holds `dir`, with one call. Every caller uses it: the writer probe (`system_probe`), `pipeline` save and assemble, the capture status and Resume, and `destination::check`.

- **macOS:** `NSURL::fileURLWithPath` and `resourceValuesForKeys_error` with `NSURLVolumeAvailableCapacityForImportantUsageKey`, then `NSURLVolumeAvailableCapacityKey` when the first is missing or 0 (`capacity`, sysinfo's rule). This is the figure that Finder shows. It counts purgeable space. `statfs` and `statvfs` do not, so they give a smaller figure and could stop a recording that has room. Also, macOS `statvfs` has 32-bit block counts. The call is in `objc2::rc::autoreleasepool`, because the writer thread has no pool.
- **Windows:** `GetDiskFreeSpaceExW`. The API needs a trailing backslash for a UNC name (`\\server\share\`), so `directory_name` adds one when the path has none. It reads the total free bytes, as sysinfo did.
- **Other platforms:** `None`. Longhand ships for macOS and Windows; an unknown figure passes every check.

There is no probe thread. When the folder's own volume does not answer, the audio writes to that volume block too, so a thread would save no audio.

## Behavior that changed on purpose

sysinfo listed only local volumes. A storage folder on a network volume got the boot volume's figure on macOS and no figure on Windows. Now it gets that volume's own figure, so a full NAS can show low space in the storage folder check or stop a save that passed before.

## Accepted limits

- A folder on a volume that does not answer still blocks its own check.
- On Windows the figure is the volume's free space, not a per-user quota on a share (`lpFreeBytesAvailableToCaller`). Before the change such a folder had no figure at all.

## Prevention

- A free-space or other volume lookup for one folder asks only that folder's volume. Do not list all mounts on a path that must not wait, and never on the audio writer thread.
- Before you replace a figure with another API's figure, check that the two agree. Here the comparison test (`the_figure_matches_the_mount_list_figure_for_the_volume`) brackets the new figure between two sysinfo reads, and the key choice has its own test (`the_capacity_for_important_usage_comes_first_and_zero_means_missing`).
- A dependency change also changes the shipped licence list. Run `scripts/check-licences.sh` before the push (see [Checks and CI](../../development-workflow.md#checks-and-ci)). PR #32 first failed CI's "Licences (R12)" step because sysinfo left the list.

## Related Issues

- [settings-lock-held-across-a-slow-shared-save.md](settings-lock-held-across-a-slow-shared-save.md): reads and writes in the storage folder have no time limit.
- [cargo-feature-unification-ci-rebuilds.md](cargo-feature-unification-ci-rebuilds.md): sysinfo is now only a dev-dependency.
- PR #32.
