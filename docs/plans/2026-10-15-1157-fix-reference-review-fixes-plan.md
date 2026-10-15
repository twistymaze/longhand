---
title: Reference review fixes, October 2026 - Plan
type: fix
date: 2026-10-15
topic: reference-review-fixes-2026-10
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Reference review fixes, October 2026 - Plan

## Goal Capsule

- **Objective:** A microphone that does not start cannot stop the user from stopping or controlling a recording. On Windows, a change of the default microphone does not prevent recording until a restart. The note editor applies its link rules to the correct text. The editor uses the prosemirror-view release that anarlog's security update group moved to. The voice detector does not send audio twice.
- **Means:** Four independent fixes from the reference source review of 2026-10-15, one commit each, in one PR. The PR title is a `fix` title that names each fix.
- **Product authority:** The user chose these four fixes on 2026-10-15 after the review in `docs/reference/reference-source-reviews.md`. R1-R7 govern behavior.
- **Open blockers:** None.

## Product Contract

### Summary

Put a time limit on the microphone start, and refuse a second start while an earlier start is still blocked. On Windows, when the default microphone does not open, open the same device by its ID. Port anarlog's corrected changed-range calculation to the note editor. Update prosemirror-view to 1.42.3. Port Handy's fix to the voice detector's pre-roll.

### Problem Frame

The review of 304 upstream commits found four defects that Longhand has. Code reading found each one. Nobody has seen one of them in Longhand.

- **Microphone start.** `crates/audio-cpal/src/lib.rs` waits for the capture thread to report the start with no time limit. The capture manager holds its lock during that wait. If CoreAudio blocks the stream start, Stop, status and all other capture commands wait too. anarlog added a 5-second limit (`9a9ed32f`) and a guard against a second start (`69350022`) for this case.
- **Windows default microphone.** With cpal 0.18.2, after the default device changes during a session, each open of the default handle can fail with `RPC_E_CHANGED_MODE` until the app restarts. Shorthand saw this and fixed it in `19029f4` and `22ef0ed`. Longhand uses the same cpal, opens the default handle when no microphone is chosen (`crates/audio-cpal/src/config.rs`), and resolves it again on each source restart.
- **Editor changed ranges.** `src/editor/plugins/changed-ranges.ts` (copied from anarlog `39613e5`) reports each step's range in that step's own intermediate document. In a transaction with more than one step, autolink and the link boundary guard can check the wrong text. anarlog fixed this in `a84a8a5d`.
- **prosemirror-view.** Longhand pins 1.41.7, which came from anarlog. anarlog's Dependabot security group moved to 1.42.3 and pinned prosemirror-model so that only one copy is installed (`a112ca8a`).
- **Voice detector pre-roll.** In `crates/vad/src/smoothed.rs` (borrowed from Handy), when speech starts again soon after a hangover, frames already sent are sent again. No production code calls `SmoothedVad` today. Handy fixed it in `7b2a28a`.

### Requirements

**Microphone start**

- R1. A microphone start that does not complete in a fixed time fails with an error, and the capture manager is available again. Stop, status and the other capture commands do not wait for a blocked start.
- R2. While an earlier microphone start is still blocked, a new start fails at once and does not start a second capture thread.
- R3. On Windows, when no microphone is chosen and the open of the default device fails, Longhand opens the same device once more by its ID.

**Note editor**

- R4. Changed ranges are positions in the final document of all the transactions in a group, so autolink and the link boundary guard check the text that the user changed.
- R5. The editor uses prosemirror-view 1.42.3, and only one copy of prosemirror-model is installed.

**Voice detector**

- R6. When speech starts, the pre-roll contains only frames that were not sent before.

**Provenance**

- R7. Each changed borrowed file has its `PROVENANCE.md` entry updated to the upstream revision of the fix.

### Acceptance Examples

- AE1. Covers R1. A test microphone source that never reports its start makes the start fail after the time limit. A Stop sent during the wait returns without waiting for the limit.
- AE2. Covers R2. A second start during that wait fails at once.
- AE3. Covers R3. A forced failure of the first default open on Windows (as Shorthand does with `SHORTHAND_DEBUG_FAIL_DEFAULT_MIC`) leads to a recording from the same device.
- AE4. Covers R4. anarlog's regression test passes: a typed bullet that joins the list above it gives ranges in the final document.
- AE5. Covers R6. Speech, a hangover, then speech again inside the pre-roll window sends no frame twice.

### Key Decisions

- One PR with four commits, as for the review of 2026-09-29. Each fix is small, all come from one review, and the PR makes one patch release. (session-settled: user-approved — chosen over one PR for each fix: the user selected all four for "the fix PR".)
- Port the voice detector fix although no production code calls `SmoothedVad`. The file is borrowed, and a later use would otherwise bring the defect back. (session-settled: user-directed — chosen over leaving the latent defect: the user selected it from the shown options with its latent status stated.)
- The Windows retry is in scope although nobody has seen the fault in Longhand. (session-settled: user-approved — chosen with the note that it is Windows-only and not seen.)

### Scope Boundaries

- No change to how Longhand chooses a microphone, or to the Bluetooth avoidance in `config.rs`.
- No change to system audio capture. `crates/audio-system` opens loopback through the `wasapi` crate, not a cpal default handle.
- No other dependency updates in this PR.
- The other review findings are ideas in `docs/ideas/someday/` or notes in `docs/reference/borrowing-notes.md`. The transcribe-cpp update and the Handy recorder crates are separate ideas (`transcribe-cpp-update`, `handy-recorder-crates`).

### Outstanding Questions

- Planning: the time limit for the microphone start. anarlog uses 5 s. Check that a slow Bluetooth or USB device start takes less.
- Planning: whether a timed-out start counts as a restart for `crates/supervisor`, so the recording tries again, or ends the segment.
- Planning: `docs/solutions/ui-bugs/typed-text-went-to-hidden-notes-after-tab-click.md` quotes line numbers in prosemirror-view's `dist/index.js`. Check them after the update.

### Sources / Research

- `docs/reference/reference-source-reviews.md`, log entry 2026-10-15.
- anarlog `69350022`, `9a9ed32f` (microphone start), `a84a8a5d` (changed ranges), `a112ca8a` (prosemirror-view).
- Shorthand `19029f4`, `22ef0ed` (Windows default microphone).
- Handy `7b2a28a` (voice detector pre-roll).
