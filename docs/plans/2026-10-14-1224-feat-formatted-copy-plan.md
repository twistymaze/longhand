---
title: Copy with Formatting - Plan
type: feat
date: 2026-10-14
topic: formatted-copy
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Copy with Formatting - Plan

## Goal Capsule

- **Objective:** When a person copies a summary, a transcript or My Notes and pastes it into Mail, Notes, Slack, Word or Google Docs, the paste shows headings, bold, lists and links, not `#` and `**` marks. A paste into a plain-text app still gives the same markdown as today.
- **Means:** One clipboard item with an HTML format and a plain-text format, written with the web Clipboard API (KTD1).
- **Product authority:** This Product Contract. The idea file is `docs/ideas/done/formatted-copy.md`.
- **Open blockers:** None.

## Product Contract

### Summary

The Copy button on the Summary, Transcript and My Notes tabs writes one clipboard item with two formats. The HTML format is made from the tab's markdown and has no Longhand styles. The plain-text format is the markdown that Copy writes today. The app that receives the paste chooses the format.

### Problem Frame

Copy writes only plain text (`src/session/copy.ts`). Rich-text apps paste that text as it is, so the user sees `#`, `**` and `-` marks in an email or a document. macOS and Windows let one clipboard item hold more than one format, and the receiving app takes the format it can use best.

### Requirements

**Formats**

- R1. Copy on the Summary, Transcript and My Notes tabs writes one clipboard item with an HTML format and a plain-text format.
- R2. The plain-text format is exactly the text that Copy writes today: the summary body, the `transcript.md` text, or the notes document.
- R3. The HTML format shows the same content as the markdown: headings, paragraphs, bold, italic, lists with their nesting, links, strikethrough and highlight.
- R4. The HTML has no Longhand fonts, colours, sizes or classes. The pasted text takes the styles of the receiving app.

**Safety and failure**

- R5. Raw HTML in the markdown is pasted as text, not as markup. A link with a `javascript:` or similar address is not a link in the HTML.
- R6. If the clipboard refuses the two-format item, Copy writes the markdown only, and the button shows "Copied". The current error notice shows only when the markdown write also fails.

### Key Decisions

- **The plain-text format stays markdown.** Markdown apps, code editors and agent prompts keep the structure, and rich-text apps take the HTML instead. Governs R2. (session-settled: user-directed — chosen over text with the markdown marks removed: markdown apps would lose the structure.)
- **The HTML has no Longhand styles.** A paste looks like the other text in the receiving document. Governs R4.

### Scope Boundaries

- No RTF format. macOS rich-text apps read HTML. The manual test checks this (see Risks).
- No change to the button, its label, or "Copied".
- No Copy as Markdown or Copy as Text choice. The receiving app chooses.
- Task list items keep their `[ ]` and `[x]` text in the HTML. They do not become check boxes.
- Considered and not built: a separate check that the HTML write succeeded in the receiving app. The manual test covers it, and the markdown fallback (R6) covers a refused write.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Use `navigator.clipboard.write` with one `ClipboardItem`** that holds `text/html` and `text/plain` blobs. Copy already uses the web Clipboard API from the button click, and WKWebView on macOS 15 and WebView2 support `ClipboardItem` with these two types. The Tauri clipboard plugin is not necessary: it adds a dependency and a capability for the same result. Covers R1.
- KTD2. **Make the HTML with a markdown-it instance in the copy module**: CommonMark, `html: false`, and the strikethrough and highlight plugins from `src/editor/markdown/parser.ts`. `html: false` escapes raw HTML, and markdown-it's link check removes `javascript:`, `vbscript:`, `file:` and `data:` addresses. Export the two plugins; do not reuse `getParser()`, because the editor disables inline code, which summaries use. Disable the `image` rule, as the editor does, so that `![alt](url)` stays text and a paste does not load a remote image. The Summary tab renders with `parseMarkdown` in `src/session/markdown.ts`, which nests a list marker that is indented more than its parent marker; CommonMark nests only at the parent's content column. The summary HTML must keep the nesting that the tab shows. Covers R3, R5.
- KTD3. **Send markdown-it's output as it is**, with no wrapper and no `style` or `class` attributes. Covers R4.
- KTD4. **Fall back in `copyText`, not in the page.** `copyText` tries the two-format write. If `ClipboardItem` is missing or the write throws, it calls `writeText` with the markdown. It returns false only when both fail. `SessionPage` keeps its current notice logic. Covers R6.

### Risks

- A macOS app that reads only RTF pastes the plain markdown. The manual test pastes into Mail, Notes, TextEdit, Pages or Word, Slack and Google Docs. If one shows markdown marks, the work stops and the user decides whether to add RTF.
- WebKit can refuse a second clipboard write after a failed first write in the same click. Then the fallback also fails, and the current error shows. This is acceptable.

---

## Implementation Units

### U1. HTML from the copied markdown

- **Goal:** A function that gives the HTML for a tab's markdown.
- **Requirements:** R3, R4, R5. KTD2, KTD3.
- **Dependencies:** None.
- **Files:** `src/session/copy.ts`, `src/editor/markdown/parser.ts` (export `strikethroughPlugin` and `highlightPlugin`), `src/session/copy.test.ts`. The plugins are borrowed code with a record in `src/editor/PROVENANCE.md`; the code stays in `parser.ts`, so the record changes only if it lists where the code is used.
- **Approach:** Add a function in `copy.ts` that renders markdown to HTML with a module-level markdown-it instance (KTD2). Update the file comment, which says that the copy is the file text only.
- **Patterns to follow:** The plugin setup in `getParser()` in `src/editor/markdown/parser.ts`.
- **Test scenarios:**
  - The summary body `## Overview\n\nWe **agreed**.\n\n- one\n  - two` gives an `h2`, a paragraph with `strong`, and a list with a nested list.
  - The transcript text `# Transcript\n\n- [00:00:00] Speaker 1: Hello.\n` gives an `h1` and a list item with the time and the speaker text.
  - `~~old~~ and ==key==` gives `s` and `mark` elements.
  - `<script>alert(1)</script>` in the markdown is escaped text in the HTML, with no `script` element.
  - `[x](javascript:alert(1))` gives no `a` element with that address.
  - `[site](https://example.com)` gives a link to that address.
  - `![x](https://example.com/a.png)` gives no `img` element.
  - The summary body `1. **Budget:**\n  - detail` gives an ordered list item that contains a nested list, as the Summary tab shows it.
  - The output has no `style` or `class` attribute.
- **Verification:** The tests pass.

### U2. Write both formats, with the markdown fallback

- **Goal:** Copy writes the two-format item, and writes the markdown only when the clipboard refuses the item.
- **Requirements:** R1, R2, R6. KTD1, KTD4.
- **Dependencies:** U1.
- **Files:** `src/session/copy.ts`, `src/session/SessionPage.tsx` (only if the call to `copyText` changes), `src/session/copy.test.ts`, `src/session/SessionPage.test.tsx`.
- **Approach:**
  1. Change `copyText` to take the markdown, make the HTML with U1, and write one `ClipboardItem` (KTD1).
  2. On a missing `ClipboardItem` or a thrown write, call `writeText` with the markdown (KTD4).
  3. Start the write in the click handler with no `await` before it, as today, so that the click is still the user gesture.
- **Patterns to follow:** The current `copyText` tests, which put a mock clipboard on `navigator`.
- **Test scenarios:**
  - `write` gets one item with `text/html` and `text/plain`. The plain text is the markdown, unchanged. `copyText` returns true.
  - `write` throws: `writeText` gets the markdown, and `copyText` returns true.
  - `ClipboardItem` is not defined: `writeText` gets the markdown, and `copyText` returns true.
  - `write` and `writeText` both throw: `copyText` returns false.
  - In `SessionPage.test.tsx`, Copy on the Transcript and My Notes tabs writes an item whose plain text is the tab's markdown, and the button shows "Copied". Update the current test, which mocks only `writeText`.
- **Verification:** The tests pass, and the manual test passes.

---

## Verification Contract

| Check | Command | Applies to |
|---|---|---|
| Frontend tests | `bun test ./src/` | U1, U2 |
| Type check | `bun run typecheck` | U1, U2 |
| Provenance | `scripts/check-provenance.sh` | U1 |
| Manual test | Signed build (`bun tauri build --bundles app`), see [Manual tests](../development-workflow.md#manual-tests) | U2 |

Manual test steps in the signed build:

1. Open a note with a summary that has headings, bold text and nested lists. Select Copy on the Summary tab.
2. Paste into Mail, Notes, TextEdit, Pages or Word, Slack and Google Docs. Each paste shows headings, bold and lists, with the receiving app's font, and no `#` or `**` marks.
3. Paste into Terminal or a code editor. The paste is the markdown, the same as before this change.
4. Do steps 2 and 3 for the Transcript tab and the My Notes tab.

---

## Definition of Done

- R1–R6 are true in a signed build.
- The tests in U1 and U2 pass, and every check in the Verification Contract passes.
- The manual test passes. If an app in step 2 shows markdown marks, the work stops and the user decides about RTF (see Risks).
- The diff has no code from abandoned attempts.
- The idea file moves to `docs/ideas/done/` after the PR merges.
