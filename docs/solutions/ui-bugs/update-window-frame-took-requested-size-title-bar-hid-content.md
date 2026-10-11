---
title: "The update window cut off its OK button because the window showed less than the size that the page requested"
date: 2026-10-06
category: ui-bugs
module: "Update window (src/update/UpdateWindow.tsx, src-tauri/src/window.rs, src-tauri/src/interface_zoom.rs)"
problem_type: ui_bug
component: frontend
severity: medium
symptoms:
  - "The Software Update window cut off the bottom of the OK button and its focus ring"
  - "The window frame was 624 x 133 pt, equal to the requested content size, so the 32 pt title bar hid the bottom 32 pt of the page"
  - "The page scrolled about 11 CSS pixels when the default button took focus"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "Tauri 2, WKWebView on macOS"
related_components:
  - tooling
tags:
  - tauri
  - macos
  - window-size
  - set-size
  - title-bar
  - wkwebview
  - interface-zoom
  - resize-observer
---

# The update window cut off its OK button because the window showed less than the size that the page requested

## Problem

The update window sizes itself to its content, as a macOS alert does. The page measures its content and asks the window for that size with `getCurrentWindow().setSize(LogicalSize)`. In the app, the window showed less than the page asked for, and the bottom of the OK button was outside the visible page. Fixed in archived PR 42.

The first fix (archived PR 37, "size the update window to its content at each interface zoom") measured `<main>` with `getBoundingClientRect`, multiplied the result by the interface scale, and called `setSize`. Its unit test checked only the multiplication. That session did not test the window in a real build at high zoom, so the difference between the requested size and the visible page did not show (session history).

## Symptoms

- The bottom of the OK button and its focus ring were cut off.
- The top margin looked thin.
- The interface zoom was 130%.
- `focus()` on the default button scrolled the page by about 11 CSS pixels.

The pixels of a screenshot gave the first hard evidence (measured with uv and Pillow):

- Window frame: 1248 x 266 px at 2x, which is 624 x 133 pt. This equals the requested size.
- Title bar: 32 pt on this macOS version.
- Content area: about 101 pt, which is 133 pt minus 32 pt.

The whole window frame had the requested size. The title bar took 32 pt from the page, which needed all 133 pt.

## What Didn't Work

- **A new scale calculation.** The scale math was correct. A test harness (swiftc, `NSWindow` and `WKWebView`) used `pageZoom` 1.3 and a wry-style zero-frame parent view as the content view. It loaded the real `UpdateWindow` page from a vite preview, where `mockIPC` forwarded `plugin:window|set_size` to `setContentSize`. The page sized correctly: `innerHeight` 111 against `scrollHeight` 112, a 1 px rounding difference. So `getBoundingClientRect` under `pageZoom` returns CSS pixels.
- **Finding the cause in the Tauri size path.** The path is `Window::set_size` in tauri, then `WindowMessage::SetSize` in tauri-runtime-wry, then `set_inner_size` in tao, then `NSWindow setContentSize`. That call sets the content size, not the frame size, and the harness agrees. In the real app, the frame took the requested height. The Tauri-side reason is not known.
- **A correction in every observer callback.** A draft of the fix read the viewport in every `ResizeObserver` callback. Self-review rejected it before it shipped: a `ResizeObserver` callback can run before the window applies the first size, so the viewport still shows the old size and the page would shrink the window by mistake.

## Solution

`useWindowFitsContent` in `src/update/UpdateWindow.tsx` no longer assumes that the requested size equals the visible viewport:

1. The first request is the content size.
2. On each window `resize` event, it sets `offset = last request - (innerWidth, innerHeight)`.
3. Each fit requests `ceil(content) + offset`.
4. A fit that gives the same size as the last request does nothing.

Before (archived PR 37, paraphrased; the code is no longer in the tree):

```ts
const rect = main.getBoundingClientRect();
getCurrentWindow().setSize(
  new LogicalSize(rect.width * scale, rect.height * scale),
);
```

After (`src/update/UpdateWindow.tsx`):

```ts
const fit = () => {
  const rect = element.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return;
  const width = Math.ceil(rect.width) + offset.width;
  const height = Math.ceil(rect.height) + offset.height;
  if (requested?.width === width && requested.height === height) return;
  requested = { width, height };
  getCurrentWindow().setSize(new LogicalSize(width, height)).catch(() => {});
};
const windowResized = () => {
  if (requested) {
    offset = { width: requested.width - window.innerWidth, height: requested.height - window.innerHeight };
  }
  fit();
};
```

The window also has no interface zoom now, as a macOS alert has none:

- `show_update_window` in `src-tauri/src/window.rs` builds the window at 100% zoom.
- `interface_zoom::change` in `src-tauri/src/interface_zoom.rs` skips the window through `has_zoom`. A Rust test covers it.
- `useInterfaceZoom({ enabled: which !== "update" })` in `src/App.tsx` turns off the zoom keys in that window.

With no zoom, one CSS pixel is one logical pixel, so the scale factor is no longer part of the fit.

In a harness that subtracted 32 pt from every request, the page fit after two requests: viewport 103 pt, content 103 pt, `scrollTop` 0. The user then confirmed a notarized build at a non-100% zoom.

## Why This Works

The page cannot know how much of the requested size the window keeps for itself (here, 32 pt for the title bar). It can measure the viewport after the window applies a size. The difference between the request and the viewport is the part that does not show. The page adds that difference to the next request, so the viewport becomes equal to the content.

The page reads the viewport only in the `resize` event. At that time the window has applied a request. A `ResizeObserver` callback has no such guarantee. The observer callback still calls `fit`, but it uses the stored offset and does not read the viewport.

The check against the last request stops a loop: after the offset settles, the next fit gives the same size and does nothing.

## Prevention

- Do not assume that a requested window size equals the visible viewport in a Tauri window. After a resize, compare the request with `innerWidth` and `innerHeight`.
- Read the viewport only after the window reports a resize, not in observer callbacks.
- Test with a viewport that is smaller than the request. `src/update/UpdateWindow.test.tsx` does this in "a window that shows less than the content grows by the part that does not show": after a request of 133 pt the test sets the viewport to 101 pt and expects a new request of 165 pt. A test that checks only the requested size cannot find this fault.
- Measure the pixels of a screenshot before you make a theory. The frame size, the title bar height and the content height pointed to the cause at once.
- Test window sizing in a signed bundle on the real macOS version. The harness sized correctly where the real app did not.
- Open question: the Tauri-side reason that the frame takes the requested height is not known. If a later Tauri or tao version changes this, the offset becomes 0 and the fit still works.

## Related

- [tao traffic light Y is not the top inset](tao-traffic-light-y-not-top-inset.md): another macOS title-bar difference, in the main window.
- `docs/plans/2026-09-28-1240-feat-preview-builds-and-updates-plan.md`: U6 and KTD9 define the update window.
- `docs/plans/2026-09-26-2119-feat-ui-polish-plan.md`: KTD3 defines the interface zoom.
