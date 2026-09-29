---
title: Header alignment with the window buttons under zoom
description: "Divide the header inset next to the window buttons by the zoom factor, because the native buttons do not scale"
area: interface
added: 2026-09-29
source: "anarlog review, 2026-09-29: commits 0a1271b4, 87bc2aa5"
related: [ui-polish]
---

# Header alignment with the window buttons under zoom

## Idea
Divide the header space next to the window buttons by the interface zoom factor.

## Why
Longhand has native zoom (`src-tauri/src/interface_zoom.rs`) and a fixed inset for the window buttons (`src-tauri/src/window.rs:46`). The native buttons do not scale with the webview, so the header is probably out of line at 150–200% (inferred, not tested). anarlog fixed the same problem.
