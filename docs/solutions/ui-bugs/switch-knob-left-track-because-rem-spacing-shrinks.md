---
title: "The switch knob went past its track because rem spacing shrinks with the 13px root font"
date: 2026-09-30
category: ui-bugs
module: Settings switch (src/settings/ui.tsx Switch, src/index.css)
problem_type: ui_bug
component: frontend
symptoms:
  - "In every Settings switch, the knob of an on switch went past the right edge of the track"
  - "The off state looked correct, so the defect showed only after a click"
root_cause: wrong_api
resolution_type: code_fix
severity: low
tags: [tailwind, rem, root-font-size, switch, settings, playwright, webkit]
---

# The switch knob went past its track because rem spacing shrinks with the 13px root font

## Problem
The `html` rule sets the root font to 13px (`src/index.css:85`), so that rem-based spacing follows macOS body text. Tailwind spacing utilities use rem (`w-8` is `0.25rem * 8`). With a 13px root, `w-8` is 26px, not 32px. The Switch track used `w-8`, but its knob (`size-[14px]`), inset (`left-[2px]`) and travel (`translate-x-[14px]`) used fixed px. The on knob ended at 2 + 14 + 14 = 30px, past the 26px track.

## Solution
Give each part of a control that has px-sized parts a px size. The track is now `h-[18px] w-[32px]` (`src/settings/ui.tsx:184`), so the knob stops 2px from the right edge. The fix was checked in a WebKit render with Playwright (auto memory [claude]: "Playwright UI review").

## Prevention
- In this app, do not mix rem-based Tailwind spacing (`w-8`, `p-2`) and arbitrary px values (`w-[14px]`) inside one control whose parts must line up. Use one unit for all parts.
- Do the arithmetic at 13px: one Tailwind spacing step is 3.25px, not 4px.
- Check both states of a toggle in a render; one state can look correct when the other is wrong.
