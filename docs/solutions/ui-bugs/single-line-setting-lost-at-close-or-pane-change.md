---
title: A single-line setting typed with no Return was lost at a window close or a pane change
date: 2026-10-16
category: ui-bugs
module: CommitField (src/settings/ui.tsx), AudioRemoval.tsx, EndpointForm.tsx, SummaryPrompts.tsx
problem_type: ui_bug
component: frontend_stimulus
severity: medium
symptoms:
  - Base URL, Model, Context limit, a prompt Name or the days before audio removal came back with the old value after the Settings window closed
  - A change from another computer replaced the text that the user was typing in such a field
root_cause: missing_workflow_step
resolution_type: code_fix
framework_version: react 18, tauri 2 (WKWebView, WebView2)
tags:
  - commit-on-blur
  - unmount
  - close-save
  - stale-closure
  - wkwebview-focus
---

# A single-line setting typed with no Return was lost at a window close or a pane change

## Problem

`CommitField` saved only on blur and Return. No blur comes in three cases:

- A window close destroys the page and does not move the focus.
- React fires no blur when a component unmounts (a pane change, or a `key` change at a prompt change).
- A click on a button in WKWebView does not move the focus (`wkwebview-click-does-not-focus-button-breaks-shortcut-capture.md`).

So the typed text stayed only in React state and was lost. A new `value` prop also always replaced the draft, so a change from another computer removed the text that the user was typing.

## Solution

- While the draft differs from the value, the field registers a close save with the close coordinator (`closeSaves.ts`). At unmount it commits the draft, and the registration stays until that commit ends.
- `onCommit(v, how)` returns `Promise<boolean>`. `how` is `"user"` (Return, blur) or `"unattended"` (close, unmount). Commits of one field run in series. An unattended commit never sends again a text that a commit sent.
- A new value replaces the draft only when the draft is the value before or a text that a commit saved.
- An unattended commit must not ask the user anything: a days change that would remove audio saves nothing.
- An unmount commit runs with the `onCommit` of the field's last render. That closure can be older than the parent's newest state: after the switch turned audio removal off, the old `commitDays` saved the typed days and turned removal on again. The parent keeps the setting of its last render in a ref (assigned during render), and the unattended commit reads it.

## Prevention

- A field that saves on blur needs a save at close and at unmount too.
- A callback that runs at unmount reads the parent's latest state from a ref, not from the closure.
- An unattended save never asks a question and never does a destructive change.
- The close coordinator is module state. A test whose save never ends keeps its registration into later tests: let the save end at the end of the test.
