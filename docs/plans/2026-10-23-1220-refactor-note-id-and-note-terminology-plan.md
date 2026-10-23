---
title: Note Id and Note Terminology - Plan
type: refactor
date: 2026-10-23
topic: note-id-and-note-terminology
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Note Id and Note Terminology - Plan

## Goal Capsule

- **Objective:** A PR that logs or prints the id of a note no longer fails the CodeQL check, and the `rust/cleartext-logging` rule stays fully on for real secrets. New code and docs call the whole item a "note".
- **Means:** Rename the session id to "note id" in code, in the note record file and in the database. Make "note" the umbrella term and "manual notes" the term for the typed text.
- **Product authority:** The user chose each decision below in the brainstorm on 2026-10-23. The rename of the other `session_*` code names is not active scope (see `docs/ideas/someday/rename-session-to-note-in-code.md`).
- **Open blockers:** None.

---

## Product Contract

### Summary

The session id becomes the note id everywhere: in code names, in the `longhand.json` key and in the database column. "Note" becomes the term in CONCEPTS.md, in the living docs and in new code for the whole item (audio, transcript, summary and manual notes). "Manual notes" becomes the term for the text that the user types. Other existing `session_*` names stay.

### Problem Frame

CodeQL guesses sensitive data from names. Its shared rule treats any variable, field or function whose name matches `session.?(id|key)` as account data (`SensitiveDataHeuristics.qll`, `maybeAccountInfo`). Longhand logs the session id in many places, so the `rust/cleartext-logging` rule raises high alerts. These alerts fail the CodeQL check on the PR. On 2026-10-23, about 100 such alerts had been dismissed by hand as false positives, and PR #61 failed on a test panic message (`docs/solutions/logic-errors/codeql-cleartext-logging-from-a-test-panic.md`).

The id is not sensitive. It is a random UUID v4 that Longhand uses as a folder name, and it is written in clear text to `longhand.json` in the synced storage folder. It gives no access. A person who can read the log can already read the storage folder.

Default setup gives no way to tell CodeQL that one name is not sensitive. The name is also a technical word: users know the item as a "note", and the UI already says "note".

### Key Decisions

- **Rename instead of changing the CodeQL setup.** Advanced setup with a local copy of the query would move the whole CodeQL configuration for six languages into the repo. A rename removes the cause and keeps default setup. (session-settled: user-directed — chosen over a narrowed local query in advanced setup: the user preferred a clear name to a scanner exception.) Governs R1, R2.
- **"Note" is the umbrella term; "manual notes" is the typed text.** "Notes" alone already means the typed text in code (`load_session_notes`, `notes.md`), so the typed text needs its own term. (session-settled: user-directed — chosen over "typed notes", "written notes", "markdown notes" and Granola's "raw notes": "manual" says that the user writes it; the transcript and summary are Markdown too.) Governs R6, R7, R8.
- **Rename the stored names too.** No code name or stored name keeps `session_id`, except where Longhand reads data that an older Longhand wrote. (session-settled: user-approved — chosen over keeping `session_id` in `longhand.json` and the database: a small read-old/write-new rule, and the existing layout rule protects older builds.) Governs R3, R4, R5.
- **No CI check against `session_id`.** CodeQL already fails a PR that logs a value with that name; CONCEPTS.md tells agents to write `note_id`. (session-settled: user-directed — chosen over a CI name check: no added CI complexity for a rare case.)
- **Only the id is renamed in existing code.** Other names such as `SessionState`, `session_dir` and the `session-core` crate stay. (session-settled: user-approved — chosen over a full session → note rename in this change: the id fixes CodeQL, and the full rename is filed as an idea.)

### Requirements

**Code names**

- R1. No Rust or TypeScript identifier names the session id with `session_id`, `sessionId` or `session_key`; the replacements are `note_id` and `noteId`. This includes variables, fields, function names (for example `check_session_id`, `valid_session_id`, `session_id_of`), parameters and Tauri command arguments.
- R2. The CodeQL check passes on the PR, and the first analysis of `main` after the merge reports no open `rust/cleartext-logging` alert about the id of a note.

**Stored data**

- R3. Longhand writes the note id in `longhand.json` with the key `note_id`, in a higher `layout` number. An older build already leaves a note with a higher layout alone, by the existing layout rule.
- R4. Longhand still reads `session_id` from a `longhand.json` in the current layout and from an old-layout `session.md`, and writes `note_id` at the next save. These readers are the only places where the text `session_id` stays.
- R5. The database column that holds the id is named `note_id` after a migration on each install. The migration keeps all rows.
- R6. The longhand-notes plugin knows both layouts: the current one with `session_id` and the new one with `note_id`. It reads a note in either layout. It still reports a layout or key that it does not know, as it does now.

**Terms and docs**

- R7. CONCEPTS.md defines "Note" (the whole item: audio, transcript, summary and manual notes), "Note id" and "Manual notes", and says that existing code uses "session" for a note and "notes" for manual notes.
- R8. Living docs (CONCEPTS.md, CLAUDE.md, AGENTS.md and the guides under `docs/`) use "note" and "manual notes" for these concepts. Plans, solution notes and ideas are dated records and stay as written.
- R9. `docs/solutions/logic-errors/codeql-cleartext-logging-from-a-test-panic.md` says that the rename removed the cause, and keeps its test rule for other sensitive-looking names, such as `secret`.

### Acceptance Examples

- AE1. **Covers R3, R4.** Given a note whose `longhand.json` has `layout: 1` and `session_id`, when the new build opens it, the note shows as usual. When the new build saves the note, the file has `note_id`, the new layout and no `session_id`.
- AE2. **Covers R3.** Given two computers on one storage folder, one updated and one not, when the updated computer saves a note, the older computer leaves that note alone until it is updated. Notes that only the older computer saves stay usable on both.
- AE3. **Covers R4.** Given a note in the old layout, with `session_id` in `session.md`, Convert still converts it, and the new note record file has `note_id`.
- AE4. **Covers R6.** Given the plugin from `main` and an app release from before this change, the plugin reads and changes notes as before.

### Scope Boundaries

- Renaming other `session_*` code names, the `session-core` and `session-save` crates, SQL table names and the `session.md` file name. Filed as `docs/ideas/someday/rename-session-to-note-in-code.md`.
- Renaming existing code that uses "notes" for manual notes. New code uses `manual_notes`.
- UI text. It already says "note" and "My Notes".
- CodeQL configuration. Default setup and every rule stay as they are.
- A CI check that blocks `session_id`.

### Dependencies / Assumptions

- A handful of people use Longhand. The user will tell them to update every computer that uses the same storage folder (AE2).
- The plugin change and the app change merge in one PR. The plugin updates from `main` at the merge, and the app release follows, so the plugin knows the new layout before any app writes it (R6).

### Outstanding Questions

**Deferred to Planning**

- Whether the layout number or a new key alone marks the change, given what the current layout rule and the plugin check. R3 assumes a layout increase.
- Whether the old-layout reader and the current-layout reader can share one place that accepts `session_id`.

### Sources / Research

- CodeQL name rule: `shared/concepts/codeql/concepts/internal/SensitiveDataHeuristics.qll` in github/codeql (`maybeAccountInfo`: `session.?(id|key)`, and the `uid` pattern). Rust sources in `rust/ql/lib/codeql/rust/security/SensitiveData.qll` match function, variant, variable and field names.
- Id format and checks: `crates/session-core/src/note_record.rs` (`LAYOUT`, `check_session_id`, `valid_session_id`). Ids are made with `uuid::Uuid::new_v4()` in `src-tauri/src/managers/capture.rs` and `src-tauri/src/commands/session.rs`.
- Plugin reader: `plugins/longhand/skills/longhand-notes/scripts/_longhand.py`.
- Earlier CodeQL lessons: `docs/solutions/logic-errors/codeql-cleartext-logging-from-a-test-panic.md`, `docs/solutions/security-issues/dispatched-release-build-could-write-main-caches.md`.
- Granola calls the typed text "raw notes" and the AI output "enhanced notes" (https://docs.granola.ai/help-center/taking-notes/ai-enhanced-notes).
