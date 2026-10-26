---
title: "A frame around the note editor needs px-3, or it shows a horizontal scroll bar"
date: 2026-10-26
category: ui-bugs
module: "Import sheet Notes field (src/home/ImportSheet.tsx)"
problem_type: ui_bug
component: frontend
severity: low
symptoms:
  - "The Notes field of the import sheet showed a horizontal scroll bar under an empty editor"
  - "The gap between the Tags row and the Notes field was smaller before the first click in the field than after it"
  - "Headless WebKit screenshots of the same sheet showed no scroll bar"
root_cause: unverified_assumption
resolution_type: code_fix
framework_version: "React 19 / Tailwind 4 / WKWebView on macOS"
related_components:
  - testing_framework
tags:
  - note-editor
  - scroll-bar
  - layout
  - webkit
  - playwright
---

# A frame around the note editor needs px-3, or it shows a horizontal scroll bar

## Problem

The compact Notes field in the import sheet (PR #72) put `NoteEditor` in a scrolling frame with `px-2` side padding, to match the sheet's other fields. On the user's Mac the empty field showed a horizontal scroll bar, and the layout of the sheet moved when the field got the focus.

## Symptoms

- A horizontal scroll bar under the placeholder text of an empty Notes field.
- The Notes frame sat closer to the Tags row before the first click in the field than after it.
- The headless WebKit screenshots that checked the layout before the manual test showed neither problem.

## What Didn't Work

- **Screenshots in headless WebKit.** Playwright's headless WebKit draws overlay scroll bars, which do not show when idle. The user's Mac has "Show scroll bars: Always", so the classic scroll bar took space in the frame. Injecting `::-webkit-scrollbar` styles did not make headless WebKit reserve space either, so the layout shift could not be reproduced there.

## Solution

Measure the overflow instead of trusting a screenshot. In the preview page, `frame.scrollWidth` was 298 and `frame.clientWidth` was 295, and the empty `p.is-editor-empty` ended 2px past the right edge of the frame.

The cause is the hover backdrop of the editor's blocks. Each top-level block extends past the text column and pads back (`src/editor/styles/note-typography.css`, `.note-typography.prosemirror-editor > *`):

```css
margin-inline: -0.75rem;
padding-inline: 0.75rem;
```

The frame must have at least 0.75rem side padding to hold that margin, as `src/settings/MarkdownField.tsx` does with `px-3`. The fix changed the Notes frame from `px-2` to `px-3` and added `overflow-x-hidden` (`src/home/ImportSheet.tsx`). After the fix, `scrollWidth` equalled `clientWidth` (295).

## Why This Works

With `px-2` (0.5rem), each block was about 0.25rem wider than the frame's content box. A frame with `overflow-y: auto` also computes `overflow-x` as `auto`, so the extra width gave a horizontal scroll bar. When the browser shows that scroll bar, and then removes it after a new layout, the height of the frame changes, so the rows around it move. With `px-3` nothing overflows. `overflow-x-hidden` keeps a later wide block from bringing the scroll bar back.

A side effect: the text in the Notes field starts 0.25rem to the right of the text in the `px-2` fields above it. Settings has the same offset between `MarkdownField` and its inputs.

## Prevention

- Give every frame that holds `NoteEditor` side padding of at least 0.75rem (`px-3`), or reuse the `MarkdownField` frame.
- In a visual check, read `scrollWidth` against `clientWidth` (and `scrollHeight` against `clientHeight`) of each scroll container. Headless screenshots do not show overlay scroll bars, so they cannot show an overflow.
- Give the user a signed build for a layout change: the Mac's scroll-bar setting can show what the headless check cannot.

## Related

- [The note editor prevents the default of every Return and Escape](note-editor-prevents-default-of-every-return-and-escape.md): the other lesson from putting `NoteEditor` in the import sheet.
