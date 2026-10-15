---
title: Recover from a web content crash
description: "Reload the main window when the WebKit content process stops, and restore the recording state from Rust"
area: interface
added: 2026-09-29
source: "anarlog review, 2026-09-29: commits 97361972, c34d4500, 44ab3be9, 59ca3648"
related: []
---

# Recover from a web content crash

## Idea
Handle `on_web_content_process_terminate`: reload the main webview (at most 3 times), rebuild the window if it was hidden, and restore the recording state in the UI from Rust after the reload.

## Why
Longhand has no termination handler (`src-tauri/src`). If the WebKit content process stops, the window stays blank. Recording is owned by Rust, and the stop shortcut works natively, so the recording continues, but the user cannot see or control it in the window.

## Notes / Current state
- anarlog added this between `39613e5` and `b02b58de`. It also lets the user stop from native code while the webview is down.
- anarlog review, 2026-10-15 (3362b7e9, 3de75bfb, d6ee34e4, 04543276, 109278c9): anarlog moved the save of live words, stop results, recovery markers and gap repair into Rust, so they continue while the webview is down. Longhand already owns these in Rust. Only the reload handler and the UI state restore are still to do.
