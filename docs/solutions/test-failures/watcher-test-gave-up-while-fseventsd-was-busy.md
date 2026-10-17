---
title: The folder watcher test gave up while fseventsd was busy after the full test run
date: 2026-10-17
category: test-failures
module: Library watcher tests (src-tauri/src/managers/library.rs)
problem_type: test_failure
component: testing_framework
severity: medium
symptoms:
  - "Test a_folder_added_under_a_watched_root_emits_its_id failed with no LibraryChanged in 10 seconds"
  - "The test failed in a full workspace test run on a PR that did not change the watcher"
  - "Run alone right after the full run, it failed about one run in three; later it passed 16 of 16 runs"
root_cause: async_timing
resolution_type: test_fix
tags:
  - macos
  - fsevents
  - fseventsd
  - watcher
  - library
  - test-timing
---

# The folder watcher test gave up while fseventsd was busy after the full test run

## Problem

`a_folder_added_under_a_watched_root_emits_its_id` starts a real `LibraryWatcher` on a temporary folder, writes a note folder, and waits for a `LibraryChanged` event. It slept 500 ms, then waited at most five times for 2 s, and wrote `session.md` again after each wait (PR #7). After the full workspace test run, it failed often.

## Root cause

Traces in the test and in `Worker::events` showed that in a failed run, no `Events` command came from the watch at all. The rewrites of `session.md` gave no events either. In the runs that passed, the first event came after 0.6 s to 10 s. The full test run makes and deletes many temporary folders, and fseventsd then delivered events late. The test did not cause a fault in the watcher. Its limit of 10 s was a guess of how fast the operating system delivers events, so the result depended on timing.

## Solution

The test waits for the change event, not for a time (PR #31):

- No sleep before the first write.
- Each 1 s wait that receives nothing writes `session.md` again, until a change arrives.
- A limit of 120 s only stops a test that hangs. It is not a guess of the delivery time.

```rust
let limit = Instant::now() + Duration::from_secs(120);
let changes = loop {
    match emitted.recv_timeout(Duration::from_secs(1)) {
        Ok(c) if !c.changed.is_empty() => break c,
        Ok(_) => {}
        Err(_) => {
            assert!(Instant::now() < limit, "no LibraryChanged");
            session_save::note_files::write_doc(&dir, &doc("a1")).unwrap();
        }
    }
};
```

FSEvents can also deliver events from before the watch started, for example the create event of the temporary root. The test accepts any batch that names the note, and it does not assert the target kind, because a busy system can send a rescan flag (a full scan).

## Prevention

- A test of real file system events waits for the event that it checks. Give it only a large limit that stops a hang, never a limit that guesses the delivery time of the operating system.
- To find out whether events arrive at all, trace the `Events` command in `Worker::events`, not only the emitted change. No command means that the operating system sent nothing, and the fault is not in the batch or the reconcile.
- Run a timing suspect alone several times right after the full workspace test run. A test that passes alone on a quiet system can still fail then.

## Related Issues

- [runner-test-stopped-before-the-title-job-that-the-save-queued.md](runner-test-stopped-before-the-title-job-that-the-save-queued.md): another test that depended on timing.
- PR #7: the earlier change to this test (rewrite after each wait).
- PR #31: this fix.
