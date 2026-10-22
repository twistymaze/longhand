---
title: "WebKit put the dark-mode text colour into copied HTML, so a paste into a light email showed no text"
date: 2026-10-22
category: ui-bugs
module: "Copy and cut of note text (src/lib/clipboard.ts, src/editor/plugins/clipboard.ts, src-tauri/src/commands/clipboard.rs)"
problem_type: ui_bug
component: frontend
severity: high
symptoms:
  - "In dark mode, text copied from a summary pasted into an email as white text on a white background"
  - "The text showed only after the user changed the text colour in the email"
  - "The Copy button, Cmd+C in the Summary and Transcript tabs, and Cmd+C in My Notes all had the problem"
  - "The unit test that checked the copied HTML for style attributes passed"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "tauri 2 / WKWebView on macOS 27"
related_components:
  - tooling
  - testing_framework
tags:
  - wkwebview
  - webkit
  - clipboard
  - pasteboard
  - nspasteboard
  - dark-mode
  - prosemirror
  - utf-8
---

# WebKit put the dark-mode text colour into copied HTML, so a paste into a light email showed no text

## Problem

On macOS, WebKit changes each HTML item that a page writes to the clipboard. It renders the HTML in a document with the window's appearance and writes each element with its computed styles. In dark mode, the computed text colour is white. The app made HTML with no styles, but the clipboard got `color: rgb(255, 255, 255)` on each block.

## Symptoms

- A paste of copied note text into a light email showed no text until the user changed the text colour.
- All three copy paths had the problem: the Copy button (`navigator.clipboard.write`), Cmd+C on a selection in the Summary or Transcript tab (WebKit's own copy), and Cmd+C in My Notes (ProseMirror's copy handler, which calls `clipboardData.setData`).
- `copy.test.ts` checked that the Copy button's HTML had no `style` attribute, and it passed. The test runs in happy-dom, which has no WebKit clipboard step.

## What Didn't Work

- **HTML with no styles from the page.** The Copy button already wrote clean HTML. WebKit added the styles after the page wrote it.
- **A `copy` event handler that calls `clipboardData.setData("text/html", …)`.** WebKit changes this HTML in the same way. A test program printed `<h2 style="… color: rgb(255, 255, 255) …">` for HTML that the handler wrote with no styles.

Thus no JavaScript path in WKWebView writes HTML that the clipboard keeps as written.

## Solution

Write the macOS pasteboard natively, and write only HTML and plain text (PR #62):

- `write_clipboard` (`src-tauri/src/commands/clipboard.rs:26-37`) calls `NSPasteboard.clearContents`, then `setString:forType:` for `public.html` and for plain text. It writes no RTF and no web archive, because WebKit's own selection copy writes both with the colours.
- `copyText` (`src/lib/clipboard.ts:18`) uses the native command first on macOS, then the webview write.
- `writeCopyEvent` (`src/lib/clipboard.ts:48`) calls `preventDefault` on a copy or cut event. On macOS it writes through the native command. On Windows it calls `setData`, because WebView2 keeps that HTML as the page writes it.
- The Summary and Transcript tabs: `onCopy` on the session pane (`src/session/SessionPage.tsx:878`) calls `copySelection`, which copies the selected structure with `rangeHtml`.
- My Notes: a ProseMirror plugin (`src/editor/plugins/clipboard.ts`) replaces ProseMirror's copy and cut.

The pasteboard HTML also needs a charset. AppKit reads HTML with no charset as Windows-1252, so `Café — “x”` pastes as `CafÃ© â€” â€œxâ€`. `pasteboard_html` (`src-tauri/src/commands/clipboard.rs:22`) puts `<meta charset="utf-8">` before the HTML. The review of PR #62 found this with `textutil -convert txt`, which uses the AppKit HTML reader.

## Why This Works

The native pasteboard write does not go through WebKit, so nothing renders the HTML or adds computed styles. The receiving app gets `<h3>`, `<ul>`, `<strong>` and links with no colour or font, and it uses its own text styles.

## Prevention

- **Check clipboard HTML in WebKit, not in happy-dom.** A unit test cannot see the WebKit step. To see what WebKit puts on the clipboard, use a small Swift program that loads the page in a `WKWebView` with the `darkAqua` appearance, copies, and prints `NSPasteboard.general.string(forType: .html)` and `.types`. Save the user's clipboard first and put it back after. Then do a paste test in a signed build in dark mode.
- **A copy handler that replaces ProseMirror's copy must keep what ProseMirror's own paste reads.** The cleaned HTML keeps `data-type` and `data-checked` on task lists and task items, `start` on `<ol>`, `<hr>`, and relative links (`keptCopy`, `src/lib/clipboard.ts:70`). Without them, a copy and paste inside My Notes changed tasks into bullets and lost rules and list numbers.
- **A cut deletes only after the clipboard holds the text.** The native write is async. The plugin deletes the selection in `written.then` (`src/editor/plugins/clipboard.ts:21`), and only when the write succeeded and the document did not change. A cut that deletes at once can lose text when the write fails.
- **Do not call the native command on Windows.** `write_clipboard` returns false there. A wait for that IPC call before `navigator.clipboard.write` can use up the click's user activation.
