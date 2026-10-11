---
title: "A grid column with no template grows to fit a line that does not wrap"
date: 2026-09-26
category: ui-bugs
module: Settings panes (src/settings/ui.tsx Pane, src/settings/About.tsx)
problem_type: ui_bug
component: frontend
symptoms:
  - "The Settings pane scrolled horizontally in a narrow window when the About pane showed a long model name"
  - "In a wide window the Settings groups ran past the right edge of the centred column, so the content looked off-centre"
  - "Placeholder data did not show the problem; only the longest real model name did"
root_cause: wrong_api
resolution_type: code_fix
severity: medium
related_components:
  - testing_framework
tags: [css-grid, tailwind, truncate, min-width-0, overflow-wrap, overflow, settings, playwright]
---

# A grid column with no template grows to fit a line that does not wrap

## Problem

In Settings > About, one licence row did not wrap: "Multitalker Parakeet Streaming 0.6B · Speech recognition · NVIDIA Open Model License". The row used Tailwind `truncate` (`overflow: hidden`, `text-overflow: ellipsis`, `white-space: nowrap`). The row still made every Settings group wider than the pane.

The cause was the shared `Pane` grid in `src/settings/ui.tsx`. It used `grid gap-6` with no column template. The single implicit column is then an `auto` track. The automatic minimum size of a grid item is its min-content contribution. Here the grid item is a `<section>` with `overflow: visible` (`src/settings/ui.tsx:36`), so the full width of the nowrap text counts. It counts although a descendant has `overflow: hidden`, because only the grid item's own overflow changes its automatic minimum. In `About.tsx` the role and licence summary span also had `shrink-0`, so the flex row could not shrink it.

## Symptoms

- Narrow window: the Settings pane scrolls horizontally. A probe at a 760 px window width gave `main` clientWidth 585 and scrollWidth 595. The right edge of every group section was at 771 px.
- Wide window: the groups are wider than the centred `mx-auto max-w-[40rem]` column (`src/settings/ui.tsx:18`), so they extend past its right edge. The content looks off-centre.
- Every pane that uses `Pane` can have the problem when one of its groups holds a long nowrap line, not only About.

## What Didn't Work

- Placeholder data did not show the defect. Short names fit in all window widths. Only the longest real model name from `crates/models/catalog.json` showed it.
- `truncate` on the text alone does not help. `overflow: hidden` on a descendant does not reduce the grid item's min-content contribution.
- `break-words` (`overflow-wrap: break-word`) alone is not sufficient for the details list. It breaks a long word at layout time, but it does not reduce the min-content size. A long URL still makes an `auto` or `1fr` track wider. `overflow-wrap: anywhere` reduces the min-content size.
- `1fr` alone is not sufficient. `1fr` is `minmax(auto, 1fr)`, so its minimum is still the min-content contribution.

## Solution

Fixed in archived PR 13.

`Pane`, `src/settings/ui.tsx:20-21`:

```tsx
// Before
<div className="grid gap-6">{children}</div>

// After
{/* minmax(0, 1fr): a group never grows past the pane to fit a line that does not wrap. */}
<div className="grid grid-cols-[minmax(0,1fr)] gap-6">{children}</div>
```

Collapsed licence summary, `src/settings/About.tsx:103`:

```tsx
// Before
<span className="shrink-0 truncate ...">
// After
<span className="min-w-0 truncate text-[0.85rem] text-label-secondary">
```

Details list and values, `src/settings/About.tsx:114` and `:78`:

```tsx
// Before
<dl className="grid grid-cols-[auto_1fr] ...">
<dd className="break-words ...">
// After
<dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 ...">
<dd className="min-w-0 [overflow-wrap:anywhere] select-text">
```

A later change in the same PR shows the full name when a row is open. The name wraps with `[overflow-wrap:anywhere]`, and the role and licence move into the details (`src/settings/About.tsx:98-105`). The link text in the details also uses `[overflow-wrap:anywhere]` (`src/settings/About.tsx:64`).

Verification: a WebKit probe found no horizontal overflow at 600, 760 and 1000 px with a row open. The left and right gaps of the column were equal within 1 px at 760, 1000 and 1300 px.

## Why This Works

- `minmax(0, 1fr)` sets the track minimum to 0. The track width is then the free space of the container, and the content cannot make it wider. The nowrap text overflows its own box, where `truncate` clips it and shows the ellipsis.
- A flex item has `min-width: auto` by default, and `shrink-0` stops all shrinking. `min-w-0` lets the summary span shrink below its text width, so `truncate` has a width to clip to.
- `overflow-wrap: anywhere` lets the browser break at any character when it calculates min-content size. A long URL or name then has a small min-content size, and the `dd` column does not grow.

## Prevention

- For a grid column that holds text, write `minmax(0,1fr)`, not `1fr` and not an implicit column. Use `grid-cols-[minmax(0,1fr)]` for a one-column grid and `grid-cols-[auto_minmax(0,1fr)]` for a label and value grid.
- Give each flex child that truncates `min-w-0`. Do not put `shrink-0` on text that can be long.
- For long unbroken text (URLs, model names, file paths), use `[overflow-wrap:anywhere]`, not `break-words`.
- Test a layout with the longest real data, at the narrowest window width. Placeholder data can hide the defect.
- Use a WebKit Playwright probe on the mock preview page (vite + `mockIPC`, see the "Playwright UI review" memory). Compare `main.scrollWidth` with `main.clientWidth`, and list the elements whose right edge is past the right edge of `main`:

  ```js
  const m = document.querySelector("main"), r = m.getBoundingClientRect().right;
  [...m.querySelectorAll("*")].filter(e => e.getBoundingClientRect().right > r + 0.5)
    .map(e => [e.tagName, e.className, Math.round(e.getBoundingClientRect().right)]);
  ```

Places in `src/` where the same problem can occur. The code does not show a defect in them now:

- `src/home/Home.tsx:215`: `grid gap-px` with an implicit column. Each item is a `SessionRow` with nowrap `truncate` title and details (`src/home/SessionRow.tsx:145-147`). The `min-w-0` on the flex child (`src/home/SessionRow.tsx:136`) lets it shrink inside the row, but it probably does not reduce what the row contributes to the implicit grid track.
- `src/session/TranscriptTab.tsx:109`: `grid-cols-[4.5rem_1fr]`. The transcript text wraps at spaces, but a long unbroken token, such as a URL, can make the `1fr` track wider.
- `src/session/SessionPage.tsx:450`: `grid-cols-[1fr_auto_1fr]` header. The side cells hold short buttons now, so the risk is low.
- Other one-column grids with an implicit column: `src/session/RecordingBar.tsx:283`, `src/session/SummaryTab.tsx:54`, `src/session/ProcessingStatus.tsx:146`, `src/lib/ConfirmDialog.tsx:44`. The risk applies if one of them gets nowrap or long unbroken text.

## Related Issues

- `docs/solutions/ui-bugs/wkwebview-click-does-not-focus-button-breaks-shortcut-capture.md` and `docs/solutions/ui-bugs/tauri-csp-img-src-blocks-inlined-app-icon.md`: other Settings window defects that only a check in WebKit or the signed app showed.
