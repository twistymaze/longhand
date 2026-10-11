---
title: "CSS hover lagged in the recording widget, because macOS panels that are never key get late pointer moves"
date: 2026-10-11
category: ui-bugs
module: "Recording widget panel and page (src-tauri/src/widget.rs, src/widget/RecordingWidget.tsx)"
problem_type: ui_bug
component: frontend
severity: medium
symptoms:
  - "In the signed macOS app, the hover highlight on the pill's Pause, Stop and x buttons appeared well after the pointer moved onto them"
  - "The highlight could stay on a button after the pointer moved to the next one"
  - "The Chromium Playwright preview and the happy-dom tests showed no delay"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "tauri 2 / tauri-nspanel 2.1.1 / WKWebView on macOS"
related_components:
  - tooling
tags:
  - wkwebview
  - webkit
  - nspanel
  - tauri-nspanel
  - hover
  - tracking-area
  - recording-widget
---

# CSS hover lagged in the recording widget, because macOS panels that are never key get late pointer moves

## Problem

The recording widget (PR #53) is a non-activating NSPanel that never becomes the key window, so it does not take focus from the app in front. In that panel, WKWebView updated CSS `:hover` late: the button highlight followed the pointer with a clear delay.

## Symptoms

- In the signed macOS build, the hover highlight on the pill's buttons appeared well after the pointer moved onto them.
- A highlight could stay on a button after the pointer moved to the next button.
- The Chromium Playwright preview and the happy-dom unit tests showed no delay, so only the signed build found it (user test, 2026-10-10).

## What Didn't Work

- **CSS `:hover` alone.** It works in a key window. In this panel, WebKit gets pointer moves late, so `:hover` is late. The panel must stay not key (`can_become_key_window: false`, `src-tauri/src/widget.rs:524`), because a key panel takes the keyboard focus from the call app in front.
- **Tests in Chromium or happy-dom.** Neither runs the page in a non-key NSPanel, so neither can show the delay. See also [WKWebView does not focus a clicked button](wkwebview-click-does-not-focus-button-breaks-shortcut-capture.md), the same class of WKWebView-only behaviour.

## Solution

The panel already had a native tracking area for the hover × (enter and exit events). The fix, made during the PR #53 manual test and merged with it, adds pointer moves to that tracking area and lets the page set the hover itself.

1. The tracking area also reports pointer moves (`src-tauri/src/widget.rs:530-532`):

   ```rust
   options: tauri_nspanel::TrackingAreaOptions::new()
       .active_always()
       .mouse_entered_and_exited()
       .mouse_moved(),
   ```

2. Each event sends the pointer position in page pixels. AppKit counts from the bottom left of the window and the page from the top left, so `y` is flipped (`src-tauri/src/widget.rs:512-519`):

   ```rust
   fn pointer(event: &NSEvent, inside: bool) -> WidgetPointer {
       let location = event.locationInWindow();
       WidgetPointer { inside, x: location.x, y: HEIGHT - location.y }
   }
   ```

   One page pixel is one point because the widget window has no interface zoom.

3. The page finds the control under the point and marks it (`src/widget/RecordingWidget.tsx:45-46`, `:90-92`):

   ```ts
   function controlAt(x: number, y: number): Control | null {
     const control = document.elementFromPoint(x, y)?.closest("button")?.dataset.control;
     return (control as Control | undefined) ?? null;
   }
   // events.widgetPointer.listen(({ payload }) => setHovered(payload.inside ? controlAt(payload.x, payload.y) : null))
   ```

   Each button carries `data-control` (a typed name, not its label text) and `data-hover={hovered === control}`.

4. The hover classes come from the pointer events on macOS and from CSS elsewhere (`src/widget/RecordingWidget.tsx:55`):

   ```ts
   const hoverFill = isMac ? "data-[hover=true]:bg-fill" : "hover:bg-fill";
   ```

   On macOS the CSS `hover:` classes are left out, so a late CSS hover cannot highlight a second button.

## Why This Works

The tracking area is a native AppKit object with `active_always`, so AppKit sends its enter, exit and move events to the panel whether or not the panel is key or Longhand is active. The page no longer depends on WebKit's own pointer tracking, which is the part that is late in a non-key window. `elementFromPoint` gives the same hit test that CSS `:hover` uses.

## Prevention

- For any Longhand surface in a non-key or non-activating window (the widget panel today), do not rely on CSS `:hover`, focus or keyboard events from WebKit. Drive them from native events, as the widget does.
- Check hover and click behaviour of such a window only in a signed build (CLAUDE.md, Manual tests). Chromium and happy-dom cannot show it.
- Name controls with a typed `data-control` value, not with label text, so a label change does not break the hover.
- A unit test can pin the page side: emit `widget-pointer` with a point, mock `document.elementFromPoint`, and assert `data-hover` on the right button (`src/widget/RecordingWidget.test.tsx`, "a pointer event marks the control under the pointer").
