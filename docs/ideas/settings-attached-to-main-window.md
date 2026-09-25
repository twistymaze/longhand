---
title: Settings attached to the main window
status: parked
area: interface
added: 2026-09-25
source: "User review of the UX redesign bundle, 2026-09-25"
related: [ux-redesign]
---

# Settings attached to the main window

## Idea
Make the Settings window a child of the main window on macOS, so that it moves with the main window, or show Settings as a sheet on it. Tauri can do the first with the window builder's `parent`.

## Why parked
The user noticed that about half of the apps they use tie Settings to the main window, and was content to keep the separate window for now. A separate Settings window that opens with ⌘, is also the usual macOS pattern (Safari, Mail).

## Notes / Current state
Settings is its own window today (`src-tauri/src/window.rs`, `open_settings`). It opens centred, at 760 × 540, and it does not save its size or place.
