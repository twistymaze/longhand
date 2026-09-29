---
title: Settings attached to the main window
description: "A Settings window or sheet that moves with the main window"
area: interface
added: 2026-09-25
source: "User review of the UX redesign bundle, 2026-09-25"
related: [ux-redesign]
---

# Settings attached to the main window

## Idea
Make the Settings window a child of the main window on macOS, so that it moves with the main window, or show Settings as a sheet on it. Tauri can do the first with the window builder's `parent`.

## Why
On 2026-09-28 the user said that the separate Settings window has been annoying in use, and wants to come back to this. The user noticed earlier that about half of the apps they use tie Settings to the main window. It was parked on 2026-09-25, when the separate window seemed good enough. A separate Settings window that opens with ⌘, is also the usual macOS pattern (Safari, Mail).

## Notes / Current state
Settings is its own window today (`src-tauri/src/window.rs`, `open_settings`). It keeps its size and position (tauri-plugin-window-state). The first open is centred, at 760 × 540 scaled by the zoom.
