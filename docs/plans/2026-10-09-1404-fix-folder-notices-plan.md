---
title: Stop folder notices on the note page
type: fix
date: 2026-10-09
execution: code
---

# Stop folder notices on the note page

## Summary

The folder tree is a storage detail. The app moves and renames note folders without help, and it tries again when a move or rename fails. So the note page reports a type change or a rename only when the change fails with an error. It does not report a folder that moves later or a move that must try again.

Trigger case: the user starts a new note without a type and changes the type at once. The note records, so the folder move waits, and the page showed "The type changed. The folder moves when the note is free." That is normal behaviour.

This plan applies the message rule in `CLAUDE.md` (UI design) to folder messages. It does not audit other messages.

## Decisions

- Errors (`result.status === "error"`) still show.
- These do not change: the move messages in Settings, the type-delete confirmation, "storage folder unavailable", the sync-wait lines, and the Folder line in the Home preview.
- The Rust code that records an owed move or rename (`crates/pipeline/src/folder_rename.rs`, `crates/pipeline/src/type_move.rs`) does not change.

## Implementation units

### 1. Type change notices

- `src/session/TypeAndTags.tsx`: `typeNotice` returns a notice only for an error. Remove the `later` and `kept` notices. Remove the type button's `title` tooltip and the `moveWaits` prop.
- `src/session/SessionPage.tsx`: stop passing `moveWaits` (line 711).
- Tests:
  - `src/session/TypeAndTags.test.tsx` (lines 101, 114): a busy note and a `kept` result both show the new type and no notice. An error still shows the negative notice.
  - `src/session/SessionPage.test.tsx` (line 1621): a type change on a busy note shows no folder text.

### 2. Rename notice

- `src/session/TitleField.tsx`: `commitTitle` returns a notice only for an error. `src/home/Home.tsx` uses the same function, so the Home rename changes too.
- Tests:
  - `src/home/Home.test.tsx` (line 537): a `kept` rename shows the new title and no message.
  - A rename error still shows its message. Add a test if none exists.

### 3. Remove `type_move_waits`

Nothing reads the field after unit 1.

- `src-tauri/src/commands/session.rs`: remove `type_move_waits` from `SessionEntry` (lines 55, 459, 471). Keep the type value from `pipeline::type_move::shown_type`. Update the tests near line 2007.
- Regenerate `src/bindings.ts`.
- Remove the field from the TS fixtures: `src/App.test.tsx`, `src/home/group.test.ts`, `src/home/Home.test.tsx` (including line 1438), and `src/processing/describe.test.ts`.
- Tests: the related crate tests and `bun test ./src/`.

### 4. Idea entry

File one idea with `bun scripts/ideas.ts` (see `docs/ideas/README.md`): a "needs attention" place for a folder move or rename that does not succeed after many tries.

## Verification

- `bun test ./src/` and typecheck pass.
- The related Rust crate tests and clippy pass.
- Manual test in a signed build: start a new note, change its type during the recording. No notice shows. After processing ends, the folder is in the folder of the new type.
