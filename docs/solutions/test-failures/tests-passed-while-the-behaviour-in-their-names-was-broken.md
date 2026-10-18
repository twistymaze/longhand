---
title: Twelve tests passed while the behaviour in their names was broken
date: 2026-10-17
category: test-failures
module: Tests across crates/pipeline, crates/shared-settings, crates/library, crates/session-core, crates/session-save, src-tauri managers and src/settings
problem_type: test_failure
component: testing_framework
severity: medium
symptoms:
  - "A test stayed green when the code that its name describes was broken on purpose"
  - "A property test failed on about 0.2 % of runs with no app change"
  - "A debounce test accepted a save delay from 0.5 s to 2 s"
root_cause: logic_error
resolution_type: test_fix
tags:
  - test-fidelity
  - mutation-proof
  - fake-timers
  - proptest
  - fixtures
---

# Twelve tests passed while the behaviour in their names was broken

## Problem

A code sweep found twelve tests that could not fail for the defect that their names describe (PR #37). Each test ran and passed, but its assertions were true for another reason. A regression in that area could reach users with every test green.

## Root cause

The tests had one of these faults:

1. **The only assertion is an outcome that the failure path also gives.** The cut-off reply test checked only `Outcome::Finished`. A failed summary also finishes the note, with a stub summary.
2. **The fixture takes another code path.** The in-use margin test wrote a new marker file, so the file-time fallback said "fresh" and the margin check did not decide. The large Background file was ASCII, so the character-count check gave "too long" and the size check before the read was not needed. The "failed speaker stage" test resumed the note, so the stage did not fail.
3. **The expected value comes from the code under test.** `made_ids_do_not_change` compared a made prompt id with itself.
4. **An assertion that is always true.** The settings drift guard had an exhaustive match that forced a new arm for a new setting, but not a value in the fixture. The `assert!` over that match always passed.
5. **A platform allowance on every platform.** The lock test checked the holder only inside `if let Some(holder)`. Windows cannot read a locked file, but the skip also ran on macOS.
6. **The test reads the store directly, not the function that the app calls.** The resume test called `store.capture_choice` and compared the result with the value that it had just written.
7. **A property that states the opposite of the documented rule.** The timeline property expected `InGap` one millisecond before the next segment. With a 1 ms gap, that position is the end of the previous segment, and a segment's end belongs to that segment.
8. **A wide real-time window.** The debounce tests checked "no save at 500 ms" and "a save within 1.5 s". A delay of 1.9 s passed. All edits also came at the same instant, so a timer that does not restart at each edit passed too.

## Solution

Make each test fail when its named behaviour breaks, and prove it with a mutation:

1. Assert the product, not only the outcome: `SummaryMeta::Complete` with the reply text, the first summary request with `max_tokens < 1_500`, and a later request with more room.
2. Force the guarded path: age the marker file 31 minutes and write its time at a literal 5-minute lead (a lead of `CLOCK_MARGIN` moves with the constant, so a zero-margin mutation stays green). Fill the large Background file with `0xff`, which is not UTF-8, so only the size check gives "too long". Make the speaker stage fail with the fake provider.
3. Compare with fixed values from an independent calculation: the UUID v5 values came from `uv run --no-project python -I -c 'import uuid; …'` with the same namespace and input.
4. Write the fixture through the real save, read it back with the app's parser, and destructure the parsed `SharedFields` with no `..`. Each field must be `Field::Known`. An exhaustive `field_of(&SharedValue)` match makes a new writer-side variant fail to compile until it has a field and a value. Rejected: hand-numbered slots with a count, because a reused slot number or an unchanged count still passes.
5. Gate the allowance to the platform that needs it: `#[cfg(not(windows))] assert_eq!(holder, Some(..))`.
6. Extract the function that the app calls (`capture::stored_capture`, used by `note_capture`) and test it. Name the test for what it proves. The `AppHandle` callers stay without a unit test; say so in the PR.
7. State the documented rule in the property, bias the generator to the boundaries (`prop_oneof![0u64..3, 0u64..600_000]` for gaps, `Just(0)` for the probe), and add fixed tests for a 0 ms and a 1 ms gap.
8. Use fake timers, as `src/editor/NoteEditor.test.tsx` does:

```ts
jest.useFakeTimers();
try {
  const user = userEvent.setup({ delay: null }); // the default delay waits on a fake timer
  // ...render, click
  await user.keyboard("Er");
  await advance(800); // less than the delay: the next edit must start it again
  await user.keyboard("v");
  await advance(950);
  expect(saves(tauri)).toEqual([]);
  await advance(50);
  expect(saves(tauri)).toEqual(["Erv"]);
} finally {
  jest.useRealTimers();
}
// advance = (ms) => act(async () => { jest.advanceTimersByTime(ms); })
```

The `async` act lets the queued save promise run before each check. Without it, a 500 ms mutation fires its timer, but the save is still queued at the "no save" check. The 950/50 ms margin is there because the fake clock can fire a timer a millisecond early.

## Prevention

- Prove each new or changed test with a temporary mutation of the code it guards: see it fail, revert the mutation exactly, see it pass. Never commit the mutation. Record each mutation and its result in the PR.
- Choose the mutation from the defect that the test must catch, not from what the new assertion obviously catches. The plan's mutations did not cover two cases that the independent review found: a writer-only `SharedValue` variant, and a timer that does not restart at each edit.
- In an `EnterWorktree` session, the auto-mode classifier refused shell commands while an app file held a mutation. Read the file with the Read tool, and revert with `git checkout -- <file>`.

## Related

- [runner-test-stopped-before-the-title-job-that-the-save-queued.md](runner-test-stopped-before-the-title-job-that-the-save-queued.md): a test must wait for the event that it checks.
- [watcher-test-gave-up-while-fseventsd-was-busy.md](watcher-test-gave-up-while-fseventsd-was-busy.md): no guessed delivery time.
- [failed-expect-on-happy-dom-node-blocks-waitfor.md](failed-expect-on-happy-dom-node-blocks-waitfor.md): timer-bound UI tests near the bun limit.
