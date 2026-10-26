---
title: Notes Field in the Import Sheet - Plan
type: feat
date: 2026-10-26
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Notes Field in the Import Sheet - Plan

## Goal Capsule

- **Objective:** A user who imports one audio file can type short context for it in the import sheet, and the first summary of the new note already uses that context.
- **Means:** A compact My Notes editor in the single-file import sheet. The import request carries the text, and the import worker puts it into the note row when it creates the row (KTD1, KTD2).
- **Authority:** This plan's Product Contract, then `CLAUDE.md`, then `docs/plans/2026-10-24-0928-feat-import-audio-recordings-plan.md` for all import behavior that this plan does not change.
- **Stop conditions:** Stop and ask when the editor's slash menu or format toolbar cannot show fully inside the sheet without a change to the editor itself, or when the change needs a new database migration.
- **Execution profile:** Lightweight. Two units on one branch, one PR.
- **Finish and ship:** `ce-work` in a worktree from `origin/main`, then a PR with a `feat` title. Ask the user "patch or minor?" before the PR opens.

---

## Product Contract

### Summary

The single-file import sheet gets a Notes field under Tags. It uses the My Notes editor at a compact size. The text becomes the new note's manual notes before processing starts. A sheet for several files does not change.

### Problem Frame

An imported recording has no context for the summary until the first summary is done. To add context, the user must open the note, type into My Notes and select Regenerate Summary. The import sheet already sets the type and tags before processing for the same reason (see `docs/plans/2026-10-24-0928-feat-import-audio-recordings-plan.md`, R2). That plan kept Notes out of the sheet so that the sheet stays small. The user now wants a small Notes field for basic context, with a sheet that is still compact.

### Requirements

**The field**

- R1. When the user imports one file, the import sheet shows a Notes field under Tags. A sheet for several files has no Notes field.
- R2. The Notes field is the My Notes editor, with the same formatting, slash commands and placeholder text.
- R3. The field is compact. It is at least 4 lines tall. It grows with the text to approximately 7 lines, and then the text scrolls inside the field. The sheet for one file becomes approximately 28rem wide (it was 24rem). The sheet for several files stays 24rem wide.

**The result**

- R4. The text in the field becomes the new note's manual notes before processing starts, so the first summary uses it.
- R5. An empty field, or a field with only spaces, gives a note with empty manual notes, as before.

**Keyboard**

- R6. Return in the Notes field makes a new line, as in My Notes. Cmd+Return starts the import from any field of the sheet, except where the editor uses Cmd+Return itself (to check a task).
- R7. Escape cancels the sheet, except when Escape closes an open menu or link field of the editor. Then only that menu or link field closes.

### Key Decisions

- **The Notes field is in the sheet.** The user changed the decision "The manual sheet has title, type and tags only" of the import plan. Governs R1, R3. (session-settled: user-directed — chosen over a sheet with no Notes field: users need to give basic context before the first summary, and a compact field keeps the sheet small.)
- **Notes only for one file.** Governs R1. (session-settled: user-directed — chosen over the same notes copied into each note of a batch: a batch rarely shares one context, and copied text in many notes is hard to correct.)
- **Typed notes are lost on Cancel or a failed import, with no warning.** This is the behavior of Title and Tags today. Governs R7. (session-settled: user-approved — proposed in the scoping synthesis; the user confirmed.)

### Scope Boundaries

- No Notes field for several files, and no notes on the watched-folder setting.
- No change to how the summary uses manual notes.
- No warning on Cancel when the field has text.
- No change to the My Notes editor itself.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **The import request carries the notes, and the worker writes them into the row when it creates the row.** `pipeline::import::Request` gets a `notes: String` field with `#[serde(default)]`. `NewImport` in `crates/session-core/src/jobs.rs` gets a `notes: &str` field, and `create_import` puts it into the `notes` column in the same `INSERT`. The early home write that `create()` starts in the same store lock then renders `notes.md` from the row (`crates/pipeline/src/early_home.rs`), so the folder and the row agree from the start. Rationale: the row exists only after the worker's first step, and the journal holds the request, so a crash before the row exists still gives the notes. A later `set_notes` call from the UI would race the worker. Covers R4.
- KTD2. **The command takes `notes: Option<String>`, and the sheet sends null for a batch or an empty field.** `import_files` in `src-tauri/src/commands/import.rs` passes the notes into each `Request` (an empty string when null). The UI decides R1 and R5: it sends the trimmed editor Markdown only for one file and only when it is not empty. The command stays generic, as it is for `title`. Covers R1, R5.
- KTD3. **An old journal entry stays readable.** `#[serde(default)]` makes an entry that an older build wrote parse with empty notes. `Request` does not deny unknown fields, so an older build reads a newer entry and ignores the notes. Only the notes of an import that is in progress during a downgrade are lost. That is acceptable. No migration: the `notes` column exists.
- KTD4. **The sheet uses `NoteEditor` directly in a frame that copies the `MarkdownField` frame.** `MarkdownField` (`src/settings/MarkdownField.tsx`) puts its label, hint and count above the editor, and that layout does not fit the sheet's two-column grid. The sheet puts the label "Notes" in the label column and a frame with the classes of the `MarkdownField` frame in the field column. The frame sets `--markdown-field-rows: 4` for the minimum height, and a maximum height of approximately 7 lines with `overflow-y: auto`. `NoteEditor` gets `className="markdown-field"`, as in `MarkdownField`, because the rule `.prosemirror-editor.markdown-field` in `src/editor/editor.css` matches the editable element. The editor uses `immediate`, so the sheet's state always holds the current text when the user selects Import. The editor keeps its default placeholder (R2). Covers R2, R3.
- KTD5. **The sheet's window key handler decides from editor state recorded before the editor handles the key.** `event.defaultPrevented` cannot tell inside the editor: prosemirror-view calls `preventDefault()` on every Enter and Escape (`captureKeyDown` in `node_modules/prosemirror-view/dist/index.js` returns true for key codes 13 and 27). So:
  - For a key press whose target is outside `.prosemirror-editor` (the title field, TagPicker, the toolbar's link field), the handler keeps an `event.defaultPrevented` check. The link field (`src/editor/widgets/format-toolbar.tsx`) calls `preventDefault()` on its own Escape.
  - For a key press whose target is inside `.prosemirror-editor`, a capture-phase window listener records the state before the editor handles the key: for Escape, whether the slash menu is open (`[data-editor-escape-consumer]:not([role=toolbar])` is in the document, `src/editor/widgets/slash-command.tsx`); for Cmd/Ctrl+Return, whether the selection is in a task item (`ul[data-type="taskList"] > li`, the `Mod-Enter` keymap in `src/editor/note/keymap.ts`). The bubble-phase handler skips the key when the record is true. A capture-phase record is necessary because the slash menu can leave the DOM before the bubble phase.
  - Otherwise Escape cancels the sheet (R7), and Cmd+Return (`metaKey`, or `ctrlKey` on Windows) submits the form (R6). Covers R6, R7.

### Assumptions

- The format toolbar is portaled to `document.body` with `position: fixed`, so the Notes frame does not clip it. Its flip boundary becomes the small frame, so for a selection on the first line it can show below the selection and cover the next lines. U2 checks in the signed build that this is acceptable. If the slash menu or toolbar cannot show fully, apply the stop condition.

### Risks

| Risk | Mitigation |
|---|---|
| The `Request` shape is in the import journal on disk. | KTD3: `serde(default)`; a U1 test reads an entry with no `notes` field. |
| `src/bindings.ts` changes. CI fails when it differs from the generated file. | Run the `export_bindings` test in `src-tauri` and commit the regenerated `src/bindings.ts`. |
| WKWebView does not move the focus on a click (`webkit-buttons-do-not-focus-on-click`). | The sheet keeps its window key handler. Tests check `document.activeElement` where the focus matters. |

### Sources

- `src/home/ImportSheet.tsx`, `src/home/ImportSheet.test.tsx`: the sheet and its tests.
- `src/settings/MarkdownField.tsx`, `src/editor/editor.css` (`.prosemirror-editor.markdown-field`): the compact editor frame.
- `crates/pipeline/src/import.rs` (`Request`, `create()`), `crates/session-core/src/jobs.rs` (`NewImport`, `create_import`, `set_notes`).
- `src-tauri/src/commands/import.rs` (`import_files`, `import_paths`, `Choices`).
- `docs/solutions/` notes on the notes editor: `notes-markdown-round-trip-and-typed-shortcuts-changed-text.md`, `typed-text-went-to-hidden-notes-after-tab-click.md`.

---

## Implementation Units

### U1. Notes in the import request and the new note row

- **Goal:** An import request can carry notes, and the new note starts with those notes in its row and in `notes.md`.
- **Requirements:** R4, R5; KTD1, KTD2, KTD3.
- **Files:**
  - `crates/pipeline/src/import.rs`: `Request.notes`, and the value in `create()`.
  - `crates/session-core/src/jobs.rs`: `NewImport.notes`, and `create_import`.
  - `src-tauri/src/commands/import.rs`: the `notes` parameter, `Choices`, and the request.
  - `src-tauri/src/managers/processing.rs`: the watched-folder `Request`, with empty notes. Also update each `Request { … }` in tests (`crates/pipeline/tests/import.rs`, `crates/pipeline/tests/runner.rs`).
  - `src/bindings.ts`: regenerated.
- **Approach:** Follow KTD1 to KTD3. Change no other step of the import worker.
- **Test scenarios:**
  - `crates/pipeline/tests/import.rs`: an import whose request has the notes "Weekly sync with Dana" gives a note whose store notes hold that text when the `Created` event arrives. This rig runs with `early_homes: None`, so it writes no `notes.md`.
  - `crates/pipeline/tests/early_home.rs`: a row made by `create_import` with notes, then `early_home::step` from the `Begun` state, gives a `notes.md` with that text.
  - Same file: an import with empty notes gives empty notes, as before.
  - Same file: a journal entry that has no `notes` field (the JSON of an older build) parses, and the import completes with empty notes.
  - `crates/session-core/tests/jobs.rs`: `create_import` with notes stores them, and `notes(id)` returns them.
  - `src-tauri/src/commands/import.rs` tests: `import_paths` with notes gives each request those notes; with no notes, each request has empty notes.
- **Verification:** The scenarios above pass, and `src/bindings.ts` has a `notes: string | null` parameter on `importFiles`.

### U2. The Notes field in the sheet

- **Goal:** The single-file sheet shows a compact Notes field and sends its text. The keyboard works as R6 and R7 say.
- **Requirements:** R1, R2, R3, R5, R6, R7; KTD2, KTD4, KTD5.
- **Dependencies:** U1.
- **Files:**
  - `src/home/ImportSheet.tsx`
  - `src/home/ImportSheet.test.tsx`
  - `src/editor/editor.css`, only if the maximum height needs a class there.
- **Approach:** Add a Notes row to the grid after Tags, for one file only, with the label in the label column (`self-start`, as Tags). Put `NoteEditor` in the frame from KTD4, with `labelledBy` on the label. Hold the Markdown in state. At submit, trim the Markdown and send it, or send null when it is empty or the sheet is for several files. Change the width of the single-file sheet to approximately 28rem (R3). Change the window key handling as KTD5 says. Submit with `form.requestSubmit()`, with the same `busy` check. Update the header comment of the file so that it names the Notes field.
- **Execution note:** First invoke `apple-design` (CLAUDE.md, UI design). Check the size in the running sheet, not only in tests.
- **Test scenarios:**
  - For one file, the sheet shows a textbox named "Notes". Text typed into it is sent as `notes` with the import.
  - For one file with an empty Notes field, `notes` is null.
  - For one file with only spaces in the Notes field, `notes` is null.
  - For three files, the sheet has no Notes textbox, and `notes` is null.
  - Return in the Notes field makes a new line and does not import.
  - Cmd+Return in the Notes field imports once, with the typed notes.
  - Cmd+Return in the Title field imports.
  - Escape while the slash menu is open closes the menu and does not cancel the sheet. A second Escape cancels the sheet.
  - Plain text typed in Notes, then Escape, cancels the sheet.
  - The existing tests (Escape cancels, Escape does nothing while the import runs, the first field has the focus) still pass.
- **Verification:** The scenarios pass. In a signed build, the sheet for one file is compact, the field grows to approximately 7 lines and then scrolls, and the slash menu and format toolbar show fully.

---

## Verification Contract

| Check | Command | When |
|---|---|---|
| Pipeline import tests | `cargo test -p pipeline --test import --test early_home` | U1 |
| Store tests | `cargo test -p session-core --test jobs` | U1 |
| Command tests and bindings | `cargo test -p longhand import` and `cargo test -p longhand export_bindings` | U1 |
| Sheet tests | `npx vitest run src/home/ImportSheet.test.tsx` | U2 |
| Frontend gates | `npm run typecheck`, `npm test` | Before the push |
| Rust gates | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, one full `cargo test --workspace` | Before the push |
| Manual test | Signed build: import one file with notes; the first summary uses them; check size, scroll, slash menu, Cmd+Return, Escape | Before the PR |

Read [Checks and CI](docs/development-workflow.md#checks-and-ci) and [Manual tests](docs/development-workflow.md#manual-tests) first.

## Definition of Done

- R1 to R7 are true, and the tests in U1 and U2 pass.
- `src/bindings.ts` is regenerated and committed.
- The user tested the signed build.
- The diff has no unused code from approaches that did not work.
- The PR title is a `feat` Conventional Commit, and the user chose patch or minor.
