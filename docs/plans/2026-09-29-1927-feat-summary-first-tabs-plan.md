---
title: Summary-First Note Tabs - Plan
type: feat
date: 2026-09-29
topic: summary-first-tabs
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Summary-First Note Tabs - Plan

## Goal Capsule

- **Objective:** A user who opens a saved note sees its summary first, without a tab click. The recording page shows only what a participant needs.
- **Means:** One tab list in Summary-first order. The recording phase filters Summary out of it (KTD1, KTD2).
- **Product authority:** This Product Contract. The user confirmed the scope on 2026-09-29. The Requirements win on product behavior, and the KTDs win on mechanism.
- **Open blockers:** None.
- **Execution profile:** One branch and one PR, in a worktree made from `origin/main`. Commit per unit. No signed build is needed: the change has no microphone or Keychain behavior. Check the layout with the Playwright preview (Verification Contract).
- **Stop conditions:** Stop and ask if a tab change at Stop cannot keep the level bars visible on Transcript, or if a caller outside `src/session/` reads the tab order.
- **Product Contract preservation:** The plan `docs/plans/2026-09-24-1956-feat-ux-redesign-plan.md` states the old order (R10, R13, the tab-order note). This plan supersedes that order. The old plan stays as history.

## Product Contract

### Summary

The note page tabs read Summary, My Notes, Transcript. Summary is not shown while the note records. It appears at Stop and shows the processing stages, as it does now. A saved note opens on Summary.

### Problem Frame

Today the tabs read My Notes, Transcript, Summary, in the order the parts come into being. A user who returns to an old note almost always wants the summary. The summary is the last tab, and while the note records, Summary shows only the line "The summary is written after you stop recording." That tab has nothing to offer during a recording.

### Key Decisions

- **Summary is the first tab, then My Notes, then Transcript.** Governs R1. (session-settled: user-directed — chosen over keeping My Notes, Transcript, Summary: reading the summary is the common reason to return to a note.)
- **Summary appears at Stop, not when processing finishes.** The processing stages stay on Summary. Governs R2, R3, R5. (session-settled: user-directed — chosen over hiding Summary until processing is done: the user said the tab can show up as soon as processing starts.)

### Requirements

**Tabs**

- R1. The tabs read Summary, My Notes, Transcript, left to right. Left and Right arrow keys move through the tabs that show, in that order, and wrap at the ends.
- R2. Summary does not show while the note records. The recording page shows My Notes and Transcript.
- R3. Summary shows for a note that is processing, saved, or missing from the list.
- R4. A recording opens on My Notes. A processing or saved note opens on Summary.

**Moves**

- R5. Stop moves the page to Summary. When processing finishes, the page moves to Summary unless the user is typing in My Notes. Both moves are as they are today.
- R6. Resume Recording on a saved note hides Summary and moves the page to My Notes. The next Stop shows Summary again.
- R7. A note whose summary failed or is empty keeps the Summary tab, because the Regenerate Summary button is there.
- R8. The page never shows a hidden tab as selected. When the selected tab is hidden, My Notes shows.

**Text**

- R9. The line "The summary is written after you stop recording." goes away, because the tab does not show during a recording.

### Scope Boundaries

- The Transcript tab, the processing stages, and the tab that a recording opens on do not change.
- The width of the tab control changes at Stop, from two segments to three. The change has no motion, as the page header comment states for all tab changes.

## Planning Contract

### Key Technical Decisions

- KTD1. **Derive the shown tab.** `SessionPage` keeps the tab the user chose in state. It shows that tab when the visible list holds it, and My Notes otherwise. This covers R8 in the render before the phase effect runs. It replaces no existing move: the phase effect still sets the tab for Stop, completion and resume.
- KTD2. **One ordered list, filtered by phase.** The module-level `tabs` array gets the new order. `SessionPage` passes the visible list to `Tabs`, and `Tabs` uses it for both the buttons and the arrow-key cycle. `tabMarkdown` and `CopyTab` do not change: they key on the tab name, not its position.
- KTD3. **The tab control keeps equal segment widths.** `auto-cols-fr` gives the segments one width when there are two or three. The header grid has a centered middle column, so the control grows on both sides at Stop.

### Assumptions

- Wrap-around on Left and Right stays, as in the current handler.
- The phase `missing` shows Summary, with the "no longer in the list" line, as it does now.

## Implementation Units

### U1. Tab order, visibility and derived selection

- **Goal:** The tabs read Summary, My Notes, Transcript. Summary is hidden while the note records. The selected tab is never hidden.
- **Requirements:** R1, R2, R3, R4, R5, R6, R7, R8. Honors the Key Decisions.
- **Dependencies:** None.
- **Files:** `src/session/SessionPage.tsx`, `src/session/SessionPage.test.tsx`.
- **Approach:**
  1. Reorder the `tabs` array to Summary, My Notes, Transcript. Replace its comment ("The tabs follow the order things happen…") with the reason: the summary is what a returning user reads first.
  2. Give `Tabs` a `tabs` prop. Use it for the buttons and for the arrow-key cycle, so the cycle covers only the tabs that show (KTD2).
  3. In `SessionPage`, filter Summary out of the list when `phase === "recording"`.
  4. Rename the tab state to the user's chosen tab. Derive `tab` from it: the chosen tab if it is visible, otherwise `"notes"` (KTD1). Every existing use of `tab` (Copy, content, the My Notes panel) keeps working.
  5. Keep the phase effect as it is. It already sets the tab on recording, Stop and completion.
  6. Update the file-header comment: list the tab order and say that Summary shows from Stop.
- **Patterns to follow:** The existing `Tabs` keyboard handler and `data-tab` focus move.
- **Test scenarios:**
  - Saved note: the tabs read `["Summary", "My Notes", "Transcript"]`, and Summary is selected.
  - Recording: the tabs read `["My Notes", "Transcript"]`, My Notes is selected, and no tab named Summary exists.
  - Stop: the page rerenders from a recording to a processing note. The tabs read `["Summary", "My Notes", "Transcript"]` and Summary is selected. Covers R5.
  - Processing note: Summary is selected and shows the stage rows. My Notes stays editable.
  - Completion: a processing note with the user on Transcript becomes saved. Summary is selected. Covers R5.
  - Completion while typing: the focus is in the My Notes editor when the note becomes saved. My Notes stays selected.
  - Arrow keys on a saved note: Right from Summary selects My Notes. Left from Summary wraps to Transcript. Right from Transcript wraps to Summary. Focus follows the selected tab.
  - Arrow keys while recording: Right from My Notes selects Transcript. Right from Transcript wraps to My Notes.
  - Resume: a saved note on Summary becomes a recording. Summary disappears and My Notes is selected. Covers R6.
  - Hidden selected tab: a saved note on Summary rerenders as a recording. The first render after the change already shows My Notes as selected (KTD1). Covers R8.
  - Failed summary: a stub summary keeps the Summary tab and its Regenerate Summary button. Covers R7.
  - The Transcript tab still shows the level bars while recording, and none after Stop.
- **Verification:** `bun run typecheck` and `bun test ./src/` pass. The Playwright preview shows the two-segment control while recording and the three-segment control after Stop, with no clipped label.

### U2. Remove the recording-time Summary text

- **Goal:** No dead text and no test for a tab that cannot show.
- **Requirements:** R9.
- **Dependencies:** U1.
- **Files:** `src/session/SessionPage.tsx`, `src/session/SessionPage.test.tsx`.
- **Approach:**
  1. In `content()`, the recording branch returns the live transcript only. Remove the `which === "summary"` arm and its `AfterStop` line. Keep `AfterStop` for the "This note is no longer in the list" line.
  2. Change the test "while recording, Transcript and Summary say what comes after Stop" to cover Transcript only.
  3. Change the test "the tabs follow the order things happen…" to name the new order and reason.
- **Test scenarios:**
  - Recording, Transcript tab: the text "transcript is ready after you stop recording" shows when the note has no live text.
  - Recording: no text that contains "summary is written after you stop recording" exists on the page.
- **Verification:** `bun run typecheck` and `bun test ./src/` pass. `rg "written after you stop" src` finds no match.

## Verification Contract

| Check | Command or step | Applies to |
|---|---|---|
| Types | `bun run typecheck` | U1, U2 |
| Tests | `bun test ./src/` (not bare `bun test`) | U1, U2 |
| Layout | Playwright preview with `mockIPC`: a recording page and a saved page, each screenshot at the app's minimum window width | U1 |

## Definition of Done

- The tabs read Summary, My Notes, Transcript, and Summary shows from Stop (R1 to R9).
- `bun run typecheck` and `bun test ./src/` pass.
- The screenshots show no clipped tab label at the minimum window width.
- The diff contains no abandoned-attempt code.
