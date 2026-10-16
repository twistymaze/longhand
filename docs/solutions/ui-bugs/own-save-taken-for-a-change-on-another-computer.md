---
title: An own save was taken for a change on another computer, and the pending save of the next text was dropped
date: 2026-10-16
category: ui-bugs
module: Settings own saves (src/settings/ownSaves.ts, Background.tsx, SummaryPrompts.tsx, Models.tsx)
problem_type: ui_bug
component: frontend_stimulus
severity: high
symptoms:
  - Text typed in the Background or a prompt while an earlier save ran was not saved, with no message
  - The notice "changed on another computer" showed for a moment after an own save
  - A change on another computer back to a text that this window saved earlier did not show, and the next edit wrote over it
root_cause: logic_error
resolution_type: code_fix
framework_version: react 18, tauri 2
tags:
  - own-save
  - stale-props
  - shared-settings
  - draft
  - changed-elsewhere
---

# An own save was taken for a change on another computer, and the pending save of the next text was dropped

## Problem

A Settings text field keeps a draft and a base: the saved text that the draft started from. When the saved text differs from the base and is not a text that this window sent, another computer changed it. The field then shows a notice and cancels the pending save, so the draft does not write over the other computer's text.

Two rules were wrong:

1. When a save returned, the field moved the base to the sent text at once. But the saved value comes from a props refresh (`onSettings` starts a read; the prompt list is read again), so for a short time the props still had the old text. Old saved text, new base, and a draft typed during the save: the field counted the draft as changed on another computer and cancelled its pending save. The refresh then cleared the notice, but nothing was pending, and a close saved nothing.
2. The set of sent texts was never cleared. A text sent once counted as an own save for the life of the pane, so another computer's change back to it was ignored.

## Solution

`OwnSaves` (`src/settings/ownSaves.ts`) keeps an ordered list of sent values that the saved value did not show yet, and a count of saves in flight:

- `sent(v)` at the start of a save, `settled()` at its end (success or failure; a failed save keeps its value, because the backend can have written it).
- `confirm(saved)` for each new saved value: when it is in the list, remove the oldest equal entry and every entry before it, and return true. When it is not in the list and no save is in flight, clear the list.
- The base of a draft moves only when `confirm` returns true, in the effect that sees the new saved value, not when the save returns.

Background, each prompt field, and the Transcription pane (chosen configuration key) use it. The prompt list also keeps only its latest read, because an older read that ends late would show old texts.

## Prevention

- After a save, do not assume that the props already show it. Compare with the saved value only when it arrives.
- A record of "my own writes" must forget each write when the saved value shows it. A record that only grows hides later changes from other sources.
- A list that is read again after each save needs a latest-read guard.
