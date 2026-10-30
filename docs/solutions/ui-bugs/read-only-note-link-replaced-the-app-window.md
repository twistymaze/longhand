---
title: "A click on a link in a read-only note replaced the app window, with no way back"
date: 2026-10-30
category: ui-bugs
module: "Window navigation (src-tauri/src/window.rs) and note links (src/editor/plugins/link-open.ts)"
problem_type: ui_bug
component: frontend
severity: high
symptoms:
  - "A user on macOS clicked a link in My Notes, and the web page replaced Longhand in the main window"
  - "The window had no back control, so the only way back to the app was to quit it"
  - "In an editable note, a plain click on the same link did nothing, so the fault did not reproduce in a normal note"
root_cause: missing_validation
resolution_type: code_fix
framework_version: "tauri 2.12 / WKWebView on macOS"
related_components:
  - tauri
tags:
  - wkwebview
  - prosemirror
  - contenteditable
  - navigation
  - on-navigation
  - links
  - opener
---

# A click on a link in a read-only note replaced the app window, with no way back

## Problem

A user clicked a link in My Notes. The web page loaded in the Longhand main window and replaced the app. The window has no back control, so the user could not get back to Longhand without a quit. PR #86 fixed it.

## Root cause

Two facts together:

1. **A read-only ProseMirror editor is not `contenteditable`.** With `editable` false, the editor root has `contenteditable="false"`. WKWebView then follows a click on an `<a href>` in the note like a link on a web page. An editable editor does not follow a plain click, so the fault shows only when My Notes is read-only.
2. **No window had a navigation guard.** The `WebviewWindowBuilder` calls in `src-tauri/src/window.rs` set no `on_navigation`, so the webview could go to any address.

My Notes is read-only only in three cases (`notesHeld` in `src/session/SessionPage.tsx`): the storage folder is unavailable, the note has a record problem (`longhand.json` is missing, does not parse, or a newer Longhand wrote it), or the note's files wait for the sync. **Processing does not make My Notes read-only.** The first diagnosis said it did, and the manual test showed that this was wrong. Read `notesHeld` before you tell a user how to reproduce a read-only note.

To reproduce the old fault on purpose: duplicate a note folder that has a link in `notes.md`, put a character at the start of the copy's `longhand.json`, and click the link in the copy.

## Solution

- **Guard each window (`src-tauri/src/window.rs`).** `navigation()` decides, and `navigation_guard()` is the `on_navigation` handler of the main, Settings, licences, changelog and update windows:
  - Allow the app page: the `tauri` scheme, `http(s)://tauri.localhost` (Windows), `about:`, and the development server origin only when `tauri::is_dev()`.
  - Open `http`, `https` and `mailto` in the default browser through tauri-plugin-opener, and refuse the navigation.
  - Refuse every other scheme, for example `file:` and `javascript:`.
- **Open links on purpose.** `src/editor/plugins/link-open.ts` opens a link with a click in a read-only editor, and with Command-click (Control-click on Windows) in an editable one. Its `mousedown` handler stops the selection, so the format toolbar does not open. Summary links (`src/session/SummaryTab.tsx`) open with a click. Both call `openLink` in `src/lib/links.ts`, which accepts only `http`, `https` and `mailto`; the main window's capability allows the same three schemes.

## Prevention

A new window, or a new surface that shows text with links, needs no extra work for safety: the window guard sends each link to the browser. Add the guard to each new `WebviewWindowBuilder`. The widget window shows no links and has no guard.
