# Provenance

Borrowed material in this directory, one record per file (R27). The source is Anarlog's editor package. `cn.ts`, `icons.tsx` and `NoteEditor.test.tsx` are written for Longhand.

## `markdown.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/markdown.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — `md2json`, `json2md` and `JSONContent` kept; the image helpers, `EMPTY_DOC`, `isValidContent` and `parseJsonContent` dropped

## `markdown/schema.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/markdown/schema.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — tables, image, clip, file attachment, mention, underline and inline code dropped; highlight and strike rank before bold and italic, so that they are the outer marks in the markdown

## `markdown/parser.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/markdown/parser.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — strike, highlight, `<br>` hard break, task list and empty-paragraph rules kept; underline, tables, clip, file attachment and mention dropped; strike and highlight parse the marks inside them; the backticks, image and autolink rules are off; the task list rule converts only the direct items of a list; blank lines after a list are empty paragraphs

## `markdown/serializer.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/markdown/serializer.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — tables, image, clip, attachment, mention, underline and inline code dropped; lists are tight; text is escaped only where the parser would read it as syntax (`escapeText`); a list after a list of the same kind takes the other marker; a link address with a space goes in angle brackets

## `note/schema.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/note/schema.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — tables, image, attachment, app link, mention, session, clip, underline and inline code dropped; the code block keeps its language; mark order matches `markdown/schema.ts`

## `note/keymap.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: ce4d1bd0d31db48ad49542375a54481026ea584f
- Source paths: `packages/editor/src/note/keymap.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — Mod-u, Mod-`, the title navigation handlers and `createTaskItemAttrs` dropped; the smart-quote, dash, ellipsis, fraction, abbreviation and symbol rules dropped, so that stored text is what was typed; `[ ] ` at the start of a bullet item makes that item a task and keeps its text; single `*`, `_` and `~` follow the markdown flanking rules; Backspace undoes an input rule; Mod-Enter checks or clears a task; the code block exit keeps the language

## `widgets/format-toolbar.tsx`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 5338e3c7cfe0349a43813838f6ce08d084086531
- Source paths: `packages/editor/src/widgets/format-toolbar.tsx`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — underline, code and comment buttons and the title check dropped; a link button and `setLink` added; Longhand icons, `cn` and colour tokens; composition gate ported with `useEffect`; shows only while the editor or its link field has the focus

## `widgets/slash-command.tsx`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/widgets/slash-command.tsx`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — Longhand icons, `cn` and colour tokens; plain task attributes; sentence-case labels; Task list and Divider keep the text of the line; a new query selects the first item

## `node-views/task-item-view.tsx`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/node-views/task-item-view.tsx`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the task item stores `checked` only; the task status, task ids and error boundary dropped; `getSafeNodePos` written in place

## `node-views/task-checkbox.tsx`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/node-views/task-checkbox.tsx`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — takes `checked: boolean` in place of a task status

## `plugins/doc-change-listener.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/plugins/doc-change-listener.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `plugins/autolink.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/plugins/autolink.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `plugins/url.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/plugins/url.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `plugins/changed-ranges.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: a84a8a5df1666a1ea19383548c246f03f9584687
- Source paths: `packages/editor/src/plugins/changed-ranges.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `plugins/changed-ranges.test.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: a84a8a5df1666a1ea19383548c246f03f9584687
- Source paths: `packages/editor/src/plugins/image-trailing-paragraph.test.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: reference — upstream's regression case for a bullet that joins the list above shows the defect. Longhand has no image trailing paragraph plugin, so it tests `getChangedRanges` directly with its own transaction and schema, and adds the single-step, no-change and autolink cases

## `plugins/clear-marks-on-enter.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/plugins/clear-marks-on-enter.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `plugins/link-boundary-guard.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/plugins/link-boundary-guard.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `plugins/placeholder.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/plugins/placeholder.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `styles/base.css`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/styles/prosemirror/base.css`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — image rules dropped; Longhand colour tokens

## `styles/task-list.css`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/styles/prosemirror/nodes/task-list.css`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — Longhand colour tokens

## `styles/note-typography.css`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/styles/prosemirror/note-typography.css`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — title, mention, underline, image and extra highlight colour rules dropped; the dark-mode rule moves to `editor.css` under `prefers-color-scheme`; Longhand colour tokens

## `editor.css`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/styles/prosemirror.css`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — only the placeholder and composition rules kept; Longhand adds the highlight token and the reduced-motion rule

## `NoteEditor.tsx`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/note/index.tsx`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: reference — Longhand's own component; it follows the plugin list, `ProseMirror` wiring and flush-on-dispose pattern of lines 739–790 and 955–1000

## `markdown.test.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/markdown.test.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the task, empty-paragraph, round-trip and schema-mirror cases kept and changed for the reduced schema; Longhand adds the full round trip, plain-text, AE7 and nested-mark cases

## `note/keymap.test.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: ce4d1bd0d31db48ad49542375a54481026ea584f
- Source paths: `packages/editor/src/note/keymap.test.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the replacement tests dropped; task attributes are `checked` only; Longhand adds the typed-markdown cases

## `plugins/autolink.test.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/plugins/autolink.test.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — Vitest; Longhand adds the pasted-URL case

## `plugins/doc-change-listener.test.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/plugins/doc-change-listener.test.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — Vitest

## `widgets/format-toolbar.test.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/widgets/format-toolbar.test.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the title tests dropped; Longhand adds the `setLink` cases

## `widgets/format-toolbar.test.tsx`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 5338e3c7cfe0349a43813838f6ce08d084086531
- Source paths: `packages/editor/src/note/index.test.ts`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — only the IME composition case kept, split into one test for each rule; Vitest and fake timers in place of real waits; the toolbar mounts in a small `ProseMirror` harness in place of `NoteEditor`; composition events do not bubble and are cancelled, so that ProseMirror's own handler does not collapse the selection in the test DOM; Longhand adds the no-composition and unmount cases

## `node-views/task-checkbox.test.tsx`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `packages/editor/src/node-views/task-checkbox.test.tsx`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — Vitest; `checked` in place of a task status
