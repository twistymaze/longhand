---
title: The macOS window buttons sat too high because tao's traffic light y is not the top inset
date: 2026-09-25
category: ui-bugs
module: Main window title bar (src-tauri/src/window.rs create_main_window, the page's title strip)
problem_type: ui_bug
component: frontend
severity: medium
symptoms:
  - "On macOS 26 the window buttons sat too close to the top of the window with traffic_light_position(20, 19)"
  - "The 52 px toolbar under the window buttons looked too tall for a title area"
  - "The same y value puts the buttons at a different height on macOS 26 than on earlier versions"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "tauri 2.11 / tao 0.35.3 / macOS 26.6"
related_components:
  - tooling
tags:
  - tauri
  - tao
  - traffic-lights
  - title-bar
  - macos-26
  - hidden-title
  - drag-region
  - windows-title-bar
---

# The macOS window buttons sat too high because tao's traffic light y is not the top inset

## Problem

Longhand's main window uses an Overlay title bar with no title, so the page shows under the title bar. The window buttons were too close to the top of the window, and the bar under them looked too tall, because `traffic_light_position`'s y value is not the gap above the buttons, and macOS 26 changed where the buttons are in their frame.

## Symptoms

- On macOS 26.6, the close, minimize and zoom buttons were too close to the top edge of the window. They were not in the vertical centre of the page's bar.
- The page's toolbar under the buttons (52 px, with `.mac .toolbar { padding-left: 84px }`) looked too tall for a title area.
- Windows showed no problem. Windows keeps its native title bar.

## What Didn't Work

- **`TitleBarStyle::Overlay` with `traffic_light_position(LogicalPosition::new(20.0, 19.0))` and a 52 px toolbar.** This was the first attempt. The value 19 was read as "19 points from the top of the window". On macOS 26 the buttons were too high, and the 52 px toolbar was too tall. Tao does not use y as the top gap (see Why This Works).
- **A standard title bar with `hidden_title(true)` only.** macOS draws the title bar in its own grey. The grey does not match the app's beige palette.
- **`TitleBarStyle::Transparent`.** The title bar shows the NSWindow background colour. That is one colour for both the light and the dark theme, so it does not follow the page's theme.

## Solution

The fix merged in PR #4 (twistymaze/longhand). It has four parts.

**1. The window (Rust, macOS only).** `create_main_window` applies the Overlay style, hides the title, and sets the button position only on macOS (`src-tauri/src/window.rs:13-29`):

```rust
#[cfg(target_os = "macos")]
let builder = builder
    .title_bar_style(tauri::TitleBarStyle::Overlay)
    .hidden_title(true)
    .traffic_light_position(tauri::LogicalPosition::new(20.0, window_button_inset()));
```

The window keeps `.title("Longhand")` (`src-tauri/src/window.rs:15`). Windows shows that title in its native title bar.

**2. The y value comes from the macOS version.** `window_button_inset()` returns 23 on macOS 26 and later, and 17 on earlier versions (`src-tauri/src/window.rs:36-43`):

```rust
#[cfg(target_os = "macos")]
fn window_button_inset() -> f64 {
    if macos_major_version().is_some_and(|major| major >= 26) {
        23.0
    } else {
        17.0
    }
}
```

These values come from Anarlog. Anarlog uses y = 25 on macOS 26 and later and y = 19 before, with x = 12 (`reference-source/anarlog/plugins/windows/src/window/v1.rs:176-196`). Anarlog's header is 48 px high (`h-12`, `pt-[9px]`, `pl-[76px]` in `reference-source/anarlog/apps/desktop/src/sidebar/custom-sidebar-header.tsx:63-64`). Longhand's strip is 44 px, 4 px shorter, so each inset is 2 points less. This keeps the buttons in the vertical centre of the strip.

`macos_major_version()` reads `ProductVersion` from `/System/Library/CoreServices/SystemVersion.plist` and parses the first number (`src-tauri/src/window.rs:46-56`). Anarlog uses `tauri_plugin_os::version()` for the same value (`v1.rs:177-186`). Longhand reads the file directly, so it needs no `tauri-plugin-os` dependency. If the read or the parse fails, the function returns `None`, and the inset is 17.

**3. The page draws the strip (web side, macOS only).** `src/main.tsx:9` adds the `mac` class to `<html>` when `navigator.userAgent` includes "Mac". `src/App.tsx:67` puts an empty strip above the content:

```tsx
<div data-tauri-drag-region className="title-strip" />
```

`src/index.css:99-106` hides the strip by default and shows it at 44 px on macOS:

```css
.title-strip {
  display: none;
}
.mac .title-strip {
  display: block;
  flex-shrink: 0;
  height: 44px;
}
```

The strip has the window's colour, so the title area follows the page's theme. The logo and New Note are back in the Home view, below the strip.

**4. The drag permission.** A drag on the strip moves the window only if the capability grants `core:window:allow-start-dragging` (`src-tauri/capabilities/default.json:8`). `core:default` does not include it. `data-tauri-drag-region` applies only to the element that carries it, not to its children. The strip is empty, so the full strip starts a drag.

The user checked the notarized bundle on macOS 26 and said "Looks better".

## Why This Works

Tao's `inset_traffic_lights` (`~/.cargo/registry/src/*/tao-0.35.3/src/platform_impl/macos/view.rs:1152-1182`) does two things with the position:

- It sets the height of the title bar container view to `close_rect.size.height + y` and puts that container at the top of the window (`view.rs:1167-1171`).
- It moves only the x of each button: `rect.origin.x = x + (i as f64 * space_between)` (`view.rs:1177-1181`). It does not change the y origin of a button. Each button keeps its own origin inside the container.

So y sets the height of the container, not the gap above the buttons. The visible gap above a button is the container height minus the button's position and height inside the container. That position comes from AppKit, not from tao. Tao applies the inset again on each `draw_rect` call (`view.rs:346-349`), so AppKit layout does not reset it.

macOS 26 changed where the buttons are inside their frame. The same y therefore puts the buttons at a different height on macOS 26 than on macOS 15. That is why Anarlog uses two values, and why Longhand reads the major version. Per this session's conclusion, the change is in AppKit's button layout. The tao code does not show it, and no Apple document was checked.

## Prevention

- **When you change the strip height**, change `window_button_inset()` too. Change both insets by half the height change. For example, 48 px to 44 px is 2 points less for each inset. Keep the CSS height (`src/index.css:105`) and the comment in `src-tauri/src/window.rs:31-35` the same as the code. Check the result in the signed bundle (`bun tauri build --bundles app`). The buttons must be in the vertical centre of the strip.
- **When you support a new macOS version**, look at the buttons on that version. A new major version can move the buttons in their frame again. If they move, add a branch to `window_button_inset()`. Look at Anarlog's `v1.rs` first, because it can already hold a value for that version.
- **When you update tao or Tauri**, read `inset_traffic_lights` again. If tao starts to set the y of each button, or changes how it computes the container height, the insets change meaning.
- **Keep Windows unaffected.** Put every title bar change for macOS under `#[cfg(target_os = "macos")]` on the Rust side. Put every CSS rule for the strip under `.mac` on the web side. Do not show `.title-strip` without the `.mac` class. Do not remove `.title("Longhand")`. Run `scripts/check-windows.sh` after a change.
- **Keep the drag permission.** Do not remove `core:window:allow-start-dragging` from `src-tauri/capabilities/default.json`. Without it, a drag on the strip does nothing. If you put controls in the strip, remember that `data-tauri-drag-region` does not apply to children. Add the attribute to each child that must start a drag. Set `data-tauri-drag-region="false"` on a control that must not start one, as Anarlog does (`custom-sidebar-header.tsx:81`, `:109`).
- **Settings window.** `open_settings` (`src-tauri/src/window.rs:97-121`) uses a standard title bar. If it gets the Overlay style later, it needs the same inset and its own strip.

## What Is Not Verified

- The value 17 for macOS 15 and earlier comes from Anarlog's 19 minus 2. Nobody ran the app on macOS 15 or earlier.
- Nobody ran the app on Windows. The Windows lint (`scripts/check-windows.sh`) and the Windows CI job pass. The claim that Windows keeps its native title bar with the title "Longhand" comes from the code, not from a run.
- The cause of the macOS 26 change is this session's conclusion, from Anarlog's two values and the result on macOS 26.6.

## Related Issues

- `docs/plans/2026-09-24-1956-feat-ux-redesign-plan.md`, Completion record: the user-facing result of this fix.
- `docs/ideas/someday/settings-attached-to-main-window.md`: the Settings window, which keeps a standard title bar.
