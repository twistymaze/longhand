---
title: Clear the rule exceptions
description: "Fix the known defects that the rule ledger lists as exceptions, so that each list is empty"
area: interface
added: 2026-11-21
source: "docs/plans/2026-11-20-1050-chore-lessons-become-checks-pilot-plan.md (deferred fix PR); PR #103"
related: [lessons-become-checks]
---

# Clear the rule exceptions

## Idea

Fix the known defects that the new checks found, and remove each one from its exception list:

- Four Escape handlers do not ignore a key for the input method: `src/home/ImportSheet.tsx`, `src/lib/ConfirmDialog.tsx`, `src/session/SessionMenu.tsx`, `src/settings/chord.ts` (`ESCAPE_EXCEPTIONS` in `src/lib/keyboard.test.ts`). An Escape that ends an IME composition closes or cancels the surface.
- Three raw `rename_new` calls in `crates/pipeline/src/type_move.rs` (`#[expect(clippy::disallowed_methods)]`) do not wait while another app holds the folder.
- The sub-second `elapsed()` assert in `capture.rs` (`a_stop_cuts_the_backoff_short`, in the `scripts/check-timing.ts` exception list) can fail on a loaded runner.
- The dependency-age check still tells the agent to add an exception for a bundled entry.

## Why

Each item is a real defect that a check now reports. The exception lists keep CI green until a `fix` PR removes them. The Escape and rename items change what users see, so they make a release.

## Notes / Current state

One `fix` PR. After each fix, remove the entry; the lists can only get shorter.
