---
title: Clickable links in notes
description: "Open a link in a note in the default browser"
area: interface
added: 2026-09-26
source: "a conversation during archived PR 13 (About pane), about the Settings opener allow-list"
related: [ux-redesign]
---

# Clickable links in notes

## Idea
A link in a note's markdown opens in the default browser when you click it.

## Why
People paste links into notes and expect to follow them. The user raised it while the About pane's links were made clickable.

## Notes / Current state
- The About pane opens links through tauri-plugin-opener, and only the Settings window may use it (`src-tauri/capabilities/settings.json`, `https://*`). Notes live in the main window, which has its own capability file and no opener permission.
- Open questions:
  - which schemes to allow (https only, or also http and mailto);
  - whether a click opens the link directly, or first shows the full address, because a note can contain text pasted from anywhere;
  - how the editor tells a click that opens a link from a click that edits the text (Cmd-click, or a click outside edit mode);
  - how to stop the webview itself from going to the page.
- The opener's URL scope uses glob patterns, and `*` also matches `/`. How to write a scope: `docs/solutions/tooling-decisions/opener-url-scope-glob-matching.md`.
- anarlog's editor already handles links (paths in `reference-source/anarlog/`): an autolink plugin (`packages/editor/src/plugins/autolink.ts`), a link mark with `rel="noopener noreferrer nofollow"` (`packages/editor/src/note/schema.ts`), and `packages/editor/src/plugins/link-boundary-guard.ts`. Read them before designing this.
