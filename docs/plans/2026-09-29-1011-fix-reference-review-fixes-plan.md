---
title: Reference review fixes - Plan
type: fix
date: 2026-09-29
topic: reference-review-fixes
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Reference review fixes - Plan

## Goal Capsule

- **Objective:** Long recordings are transcribed with less memory on Parakeet models, and transcription failures report their real cause. The app does not crash on a microphone that has no readable name. The format toolbar does not cover text that an IME is still composing.
- **Means:** Three independent fixes from the reference source review of 2026-09-29, one commit each, in one PR (KTD1-KTD3).
- **Product authority:** The user asked for these three fixes on 2026-09-29 after the review in `docs/reference/reference-source-reviews.md`. R1-R6 govern behavior.
- **Execution profile:** Implement in a managed worktree from `origin/main`. U1, U2 and U3 have no dependencies on each other.
- **Stop conditions:** If transcribe-cpp 0.2.4 changes an API that Longhand uses, or a real-audio route check shows a regression, stop U1 and report it. Ship U2 and U3 without U1. The Definition of Done then applies to U2 and U3 only, and the PR states that R1 and R2 are not met.

## Product Contract

### Summary

Upgrade the transcription runtime to transcribe-cpp 0.2.4 and move the runtime pin with it. Read microphone names through a call that cannot panic. Hide the format toolbar while an IME composition is active and for a short settle time after it ends.

### Problem Frame

The review of upstream changes found three defects that Longhand has.
transcribe-cpp 0.2.3 uses more memory for Parakeet on long audio (upstream #161, a change in the Parakeet pre-encoder). It also reports some Whisper out-of-memory and backend failures as model-file (GGUF) errors (#171). The decode-budget change (#165) does not affect Longhand's catalog models: in the 0.2.4 source, Whisper keeps its fixed limit of 256 new tokens, and the Parakeet and Nemotron families do not report `OutputTruncated`.
On cpal 0.18, `Device`'s `Display` returns an error when the device description cannot be read, and `to_string()` then panics. Longhand calls it for every microphone name.
Longhand's format toolbar came from anarlog before anarlog fixed the IME case. During composition the selection covers the composed text, so the toolbar opens over the text that the user is typing.

### Requirements

**Transcription runtime**

- R1. Longhand builds and runs with transcribe-cpp 0.2.4 on macOS and Windows.
- R2. A new note records the 0.2.4 runtime revision, and processing accepts that note.

**Microphone names**

- R3. Listing, choosing, opening and logging a microphone never panics when cpal cannot read the device description.
- R4. A microphone with an unreadable name still shows in the list with a fallback name, and a saved choice still matches it by ID.

**Format toolbar**

- R5. The format toolbar stays hidden while an IME composition is active in the note editor.
- R6. After the composition ends, the toolbar shows again for a real selection: after the first selection change, or at most 50 ms after the end of the composition.

### Scope Boundaries

- No change to `crates/asr/src/recovery.rs`. The split on `OutputTruncated` stays as the backup.
- No change to the choice of stream format in `preferred_config`. The review recorded it as note only.
- The other review findings stay in `docs/ideas/someday/`.

### Sources / Research

- `docs/reference/reference-source-reviews.md`, log entry 2026-09-29, "Act now".
- Handy `2d526b1` (transcribe-cpp bump). Upstream transcribe.cpp #165 (decode budget), #161 (Parakeet memory), #171 (error codes).
- Shorthand `a5da3f0`, `src-tauri/src/audio_toolkit/audio/device.rs` `device_display_name`.
- anarlog `7b57ea48` (the fix and its render test in `packages/editor/src/note/index.test.ts`) and `5338e3c7` (`useMountEffect` refactor of the same code).

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Pin `TRANSCRIBE_REVISION` to the 0.2.4 source commit `4807edaf210d0d7e8a6f7fb2a44b65966a2797f0`.** The current pin `63a44d92…` is the `.cargo_vcs_info.json` commit of the 0.2.3 crate, so the new pin follows the same rule. The 0.2.4 crates on crates.io give `4807edaf…` for both `transcribe-cpp` and `transcribe-cpp-sys`. Their Rust `src/` and `bindings/rust/sys/build.rs` are the same as 0.2.3. The C header changes only the patch number and a comment.
- KTD2. **Read the name from `Device::description()` and use a fallback on error.** This follows Shorthand's `device_display_name`. Use `DeviceDescription::name()`, not the `Display` of the description: that `Display` adds the manufacturer and device type, which would change every label and break the match by name in `find_choice`. The fallback is the fixed name "Unknown microphone", because the list must show the device (R4). `device_id` stays first in `cache_key` and `find_choice`, so a fallback name never replaces the ID match.
- KTD3. **Port anarlog's composition gate as it is, with `useEffect` in place of `useMountEffect`.** The gate is set on `compositionstart`. On `compositionend`, the component saves the state. The gate opens at the first selection change after that state, or after a 50 ms timer. A new `compositionstart` and unmount cancel the timer. anarlog found each of these rules from a separate defect (commit messages of `7b57ea48`), so keep all of them. Longhand has no `useMountEffect`, and an empty-dependency `useEffect` cleanup does the same.

### Assumptions

- Notes recorded before the bump keep the 0.2.3 pin, and processing refuses them with "the note requires a different runtime revision". Nobody else uses Longhand yet, so no migration is necessary. Test notes from the development machine may need a new recording.
- An unreadable device description happens mostly when a device goes away between the listing and the name read. The frequency on macOS is not measured.

### Risks and Dependencies

- The 0.2.4 native build brings ggml 0.25.3. The Windows CI job builds ggml from source, so the first CI run after the bump is cold and slow (`docs/solutions/performance-issues/cargo-feature-unification-ci-rebuilds.md`).
- 0.2.4 changes the Parakeet encoder and the Whisper error codes. A same-audio comparison with 0.2.3 (U1) is the only check that can show a regression.

---

## Implementation Units

### U1. Upgrade transcribe-cpp to 0.2.4

- **Goal:** Build with transcribe-cpp 0.2.4 and pin the new runtime revision.
- **Requirements:** R1, R2
- **Dependencies:** None
- **Files:**
  - `crates/asr/Cargo.toml` (the three target-specific entries)
  - `src-tauri/Cargo.toml`
  - `Cargo.lock`
  - `crates/models/src/recognition.rs` (`TRANSCRIBE_REVISION`)
  - `licences/generated/macos.txt`, `licences/generated/windows.txt`
  - `src/settings/acknowledgements.ts` (the ggml and transcribe.cpp rows), `src/settings/About.test.tsx`
- **Approach:**
  1. Change every `transcribe-cpp` version from `0.2.3` to `0.2.4` and update the lock file for `transcribe-cpp` and `transcribe-cpp-sys` only. ggml is vendored inside `transcribe-cpp-sys`, so no other lock entry changes (Handy's `2d526b1` lock diff has only these two crates).
  2. Set `TRANSCRIBE_REVISION` per KTD1. `crates/pipeline/src/recognition.rs` and `src-tauri/src/managers/processing.rs` read the constant, so they need no change.
  3. Generate the licence lists again with `scripts/check-licences.sh`.
  4. Change the hand-kept version `0.2.3` to `0.2.4` in the About acknowledgements (revision and docs.rs licence URLs) and in the expected URLs of its test. No check finds these (`docs/solutions/workflow-issues/full-licence-list-blind-spots.md`).
- **Execution note:** This is mostly a dependency change. Prove it with the build, the existing tests and the real-audio route check, not with new unit tests.
- **Test scenarios:**
  - Test expectation: none for new unit tests. The existing tests that build an execution spec from `TRANSCRIBE_REVISION` must pass unchanged.
  - Real audio: the same audio of at least five minutes on 0.2.3 (current `main`) and on 0.2.4 (the branch). Run it after Stop for each catalog model that uses transcribe.cpp (Parakeet Unified, Parakeet TDT, both Nemotron streaming models, Whisper large-v3 turbo), and during recording for Parakeet Unified with Pyannote and with Nemotron 3.
  - For each run, record transcript coverage, text differences and peak memory. A regression is lower coverage, lost text or higher peak memory on 0.2.4.
- **Verification:** The workspace builds and tests pass on macOS. `scripts/check-windows.sh` and `scripts/check-licences.sh` pass. The comparison is recorded locally with source duration, pace, models, runtime, coverage and peak memory, and shows no regression.

### U2. Read microphone names without a panic

- **Goal:** `device_name` returns the device's name or a fallback name. It never panics.
- **Requirements:** R3, R4
- **Dependencies:** None
- **Files:**
  - `crates/audio-cpal/src/config.rs`
  - `crates/audio-cpal/PROVENANCE.md` (status line of the Handy `src/config.rs` record only; the helper is Longhand's own code on cpal's public API, so it needs no Shorthand record)
- **Approach:**
  1. Put the choice between a readable name and the fallback in a small pure helper that takes the result of the description read. `device_name` calls it with `device.description()` (KTD2).
  2. All current callers already go through `device_name`, so they need no change: `input_of`, `cache_key`, the Bluetooth swap log in `config.rs`, and `default_device_name`, `list_mic_devices` and the open log in `crates/audio-cpal/src/lib.rs`. Confirm with `git grep` that no other code formats a cpal `Device` with `Display`.
- **Patterns to follow:** `is_bluetooth` and `is_physical` in `config.rs` already use `description()` and treat an error as "not known".
- **Test scenarios:**
  - A readable description gives exactly its `name()`, the same text as the current `to_string()`.
  - A description error gives the fallback name and no panic.
  - `find_choice` with a saved ID matches an input whose name is the fallback.
- **Verification:** The unit tests in `crates/audio-cpal/src/config.rs` pass. `git grep` finds no `to_string()` or `{}` formatting of a cpal `Device`.

### U3. Hide the format toolbar during IME composition

- **Goal:** The toolbar does not open over text that an IME is composing.
- **Requirements:** R5, R6
- **Dependencies:** None
- **Files:**
  - `src/editor/widgets/format-toolbar.tsx`
  - `src/editor/widgets/format-toolbar.test.ts` (or a `.tsx` sibling for the render tests)
  - `src/editor/PROVENANCE.md` (`widgets/format-toolbar.tsx` revision and status, plus a record for the ported test)
- **Approach:**
  1. Add the composition gate from anarlog `7b57ea48` to `FormatToolbar` (KTD3). `shouldShowToolbar` also requires that the gate is open.
  2. Existing behavior that depends on `shouldShowToolbar` stays: the link draft closes when the toolbar hides, and the positioning cleanup runs.
  3. Set the provenance revision to `5338e3c7cfe0349a43813838f6ce08d084086531`. Add "composition gate ported with `useEffect`" to the status.
  4. The render tests need the editor view to set exact range selections. `NoteEditor` exposes no view, so mount `FormatToolbar` in a small test harness inside the react-prosemirror `ProseMirror` component and capture the view with `useEditorEffect`. Use fake timers in place of anarlog's real-time waits.
- **Patterns to follow:** `@handlewithcare/react-prosemirror` `useEditorEventListener` and `useEditorEffect`, as anarlog uses them. The fake-timer setup in `src/editor/NoteEditor.test.tsx`.
- **Test scenarios:**
  - Select text, fire `compositionstart`: the toolbar is not in the document.
  - During composition, a range selection does not show the toolbar.
  - After `compositionend` with no selection change, the toolbar stays hidden until 50 ms pass. After 50 ms, a real selection shows it.
  - After `compositionend`, a selection change before 50 ms shows the toolbar at once.
  - A new `compositionstart` inside the 50 ms window keeps the toolbar hidden after the old timer would have fired.
  - Unmount inside the 50 ms window causes no error or state update.
  - Without composition, selecting text still shows the toolbar (no regression).
- **Verification:** `bun test ./src/` and `bun run typecheck` pass. In a signed build, typing with a macOS IME (for example Japanese Romaji) in a note shows no toolbar over the composed text.

---

## Verification Contract

- Rust: focused tests for `audio-cpal`, `models` and `pipeline`, then the full workspace tests and the Rust format and lint checks in `docs/development-workflow.md#checks-and-ci`.
- Frontend: `bun test ./src/` (not bare `bun test`) and `bun run typecheck`. No formatter runs on whole files.
- Scripts: `scripts/check-windows.sh`, `scripts/check-licences.sh`, `scripts/check-provenance.sh`.
- Real audio (U1): the same-audio comparison of 0.2.3 and 0.2.4 in U1, under `docs/development-workflow.md#local-recognition-checks`. Keep audio, model paths and raw results out of tracked files.
- Manual (U1, U3): a signed build per `docs/development-workflow.md#manual-tests`. Record a short note with the default microphone, and type with an IME in a note.

---

## Definition of Done

- R1-R6 pass. U1, U2 and U3 are one commit each on one branch, and CI passes on the PR. If the U1 stop condition fires, R3-R6 pass for U2 and U3, and the PR reports why U1 stopped.
- The real-audio comparison for U1 is recorded, and the PR states its limits.
- Provenance records match the borrowed code, and `scripts/check-provenance.sh` passes.
- `docs/reference/reference-source-reviews.md` marks the three "Act now" items as done with the PR number.
- The diff has no abandoned code and no changes outside the three fixes.
