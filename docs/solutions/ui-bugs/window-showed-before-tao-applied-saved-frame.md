---
title: Windows showed at the screen centre because tao applies a frame change later but shows a window at once
date: 2026-10-10
category: ui-bugs
module: "Window lifecycle (src-tauri/src/window.rs show, show_now, order_front_without_key; src/update/UpdateWindow.tsx)"
problem_type: ui_bug
component: frontend
severity: medium
symptoms:
  - "At launch the main window showed at the centre of the screen for a moment, then moved to its saved place"
  - "The Settings window did the same when it opened"
  - "The update window took the focus, so a Space press meant for another app clicked Install and Relaunch"
  - "The update window showed at 480 x 200 and then changed to the size of its content"
root_cause: async_timing
resolution_type: code_fix
framework_version: "tauri 2.11.6 / tao 0.35.3 / tauri-plugin-window-state 2.4.1 / macOS"
related_components:
  - tooling
tags:
  - tauri
  - tao
  - window-state
  - macos
  - window-flash
  - key-window
  - focus
  - gcd
---

# Windows showed at the screen centre because tao applies a frame change later but shows a window at once

## Problem

On macOS, a window that the window-state plugin restores showed at the default place first and then moved to its saved place. The update window had a related defect: each show made it the key window, so key presses meant for another window went to it. Fixed in archived PR 52.

## Symptoms

- At launch the main window showed at the centre of the screen, then moved. The Settings window did the same.
- The update window opened as the key window with the focus on Install and Relaunch. A Space press typed into another app clicked the button.
- The update window showed at its build size (480 x 200), then changed to the size of its content.

## What Didn't Work

- **`.focusable(false)` on the update window.** The window could not take key presses, but macOS draws the window buttons of a window that can never be key in grey. The window looked inactive even after a click. The user rejected it.
- **A page filter for keyboard clicks (`event.detail === 0`).** It stopped Return and Space on a focused button. It also stopped VoiceOver, because a VoiceOver press is a click with a click count of 0. An adversarial review found this.
- **A backup timer that found the update window by its label.** If the update state closed the window and built a new one in less than 1 s (a recording that starts and stops while an update is offered), the old timer showed the new window at 480 x 200. A Codex review found this.

## Solution

The cause is in the tao 0.35.3 crate (its `platform_impl/macos/window.rs`, in the cargo registry, not this repo). On macOS, `set_outer_position` and `set_inner_size` queue `setFrameTopLeftPoint` and `setContentSize` on the main dispatch queue (tao's `macos/util/async.rs`, `exec_async`). `set_visible(true)` calls `makeKeyAndOrderFront` at once (`make_key_and_order_front_sync`). Tauri runs the window-state plugin's restore in its `on_window_ready` hook through `run_on_main_thread`. When the build runs on the main thread (as at launch), the restore runs before the build returns. So the plugin queues the move, the app shows the window at once, and the queued move runs on the next run loop turn. A window built with no position starts centred (`ns_window.center()`).

1. **Show after the queued frame change.** `show` (`src-tauri/src/window.rs:402`) puts the show on the same main dispatch queue. The queue runs blocks in order, so the show comes after the plugin's move and resize:

   ```rust
   #[cfg(target_os = "macos")]
   dispatch2::DispatchQueue::main().exec_async(move || show_now(&app, &window, focus));
   ```

2. **Show the update window without making it key.** tao's show always calls `makeKeyAndOrderFront`. `order_front_without_key` (`src-tauri/src/window.rs:378`) calls `orderFrontRegardless` on the `NSWindow`. The window comes to the front also when Longhand is not the active app, and a click makes it key as usual. The builder adds `.focused(false)`, `.always_on_top(true)` and `.accept_first_mouse(true)`, so the window is not behind other windows and the first click chooses.

3. **Ignore Return and Space in the page.** The page focuses no button and calls `preventDefault()` for Enter and Space key-down (`src/update/UpdateWindow.tsx:160`). Pointer and VoiceOver clicks still choose.

4. **Show the update window after it fits.** The window builds hidden. The page calls the `show_update_window` command after the first window resize that needs no new size request (`windowResized` in `src/update/UpdateWindow.tsx`). A 1 s timer shows the window if the page does not. `UPDATE_OPENS` (an `OpenCount`) records each build, and a timer acts only when its window is still the current one (`src-tauri/src/window.rs:290-295`).

## Why This Works

The frame change and the show now go through one ordered queue, so no frame is drawn at the old place. The update window never becomes key by itself, so a key press goes to the window where the user types. The page guard covers the case where the window became key after a click.

## Prevention

- When code shows a new window that a plugin or tao setter moves or resizes, show it through `show` in `window.rs`. Do not call `window.show()` directly after the build.
- Use `focus: false` for a window that opens without a user action. Do not use `.focusable(false)` to keep key presses out, unless grey window buttons are acceptable (the recording widget is a panel, so there it is correct).
- To stop keyboard activation in a page, stop the keys. Do not filter clicks by `detail`, because assistive technology sends clicks with a count of 0.
- A delayed action on a window that is found by its label needs a build count (`OpenCount`), because a window with the same label can replace it.
- Test window position, focus and key-window behaviour in a signed bundle. Unit tests cannot see AppKit timing. See [Manual tests](../../development-workflow.md#manual-tests).

## Related

- [The update window cut off its OK button](update-window-frame-took-requested-size-title-bar-hid-content.md): the size fit that step 4 waits for. That doc describes the earlier default-button focus, which archived PR 52 removed.
- [tao's traffic light y is not the top inset](tao-traffic-light-y-not-top-inset.md): another tao macOS window behaviour found by reading tao source.
- [WKWebView does not focus a button on click](wkwebview-click-does-not-focus-button-breaks-shortcut-capture.md): why a click does not give a button the focus in the update page.
