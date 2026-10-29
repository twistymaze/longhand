---
title: Library Footer for Settings and Updates - Plan
type: feat
date: 2026-10-29
topic: app-footer
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Library Footer for Settings and Updates - Plan

## Goal Capsule

- **Objective:** On Windows and macOS, a user can open Settings, see the installed version, check for updates, and see that an update waits, from the library window, with no menu bar and no hidden control.
- **Product authority:** The maintainer settled this scope in the 2026-10-29 session. The Windows tray items and the unchanged macOS app menu are part of the settled scope.
- **Open blockers:** None.

---

## Product Contract

### Summary

A thin footer along the bottom of the note list in the library window, on Windows and macOS. Settings is on the left. The current version is on the right, and a click on it opens a menu with Check for Updates…. While an update waits, the version changes to an Update available badge that opens the update window. The Windows menu bar that commit `5b721aa` added is removed.

### Problem Frame

On Windows, the main window had no menu, so Settings and Check for Updates were reachable only through the tray icon or Ctrl+,. On Windows the tray menu opens with a right-click, and Windows often hides the icon in the notification-area overflow, so most users would not find either action. The tray menu also did not offer Check for Updates on Windows, although Windows has had in-app updates since PR #79.

A first fix (commit `5b721aa` on branch `worktree-windows-app-menu`, not pushed) added a native File and Help menu bar to the Windows main window. The maintainer tested it on Windows: it works, but a menu bar that holds four items and is always visible looks out of place in this window. macOS does not have the problem, because the app menu holds About, Check for Updates… and Settings….

### Key Decisions

- **A footer under the note list, not a button in the top bar or an always-visible right panel.** It costs about one line of height and no list width, and the right panel keeps its current rule (it shows only for a selected note). (session-settled: user-directed — chosen over a Settings button and update badge in the top bar, and over a right panel that is always visible with a footer: the maintainer chose the status-bar shape.) Governs R1, R2.
- **The version text opens a menu, and the menu starts the check.** A click on a version label does not start a network check by itself, because the user would not expect a label to start work. (session-settled: user-directed — chosen over a direct check on click: the maintainer wants no action that the user does not see coming.) Governs R4.
- **The version menu holds only Check for Updates….** What's New and About stay in Settings, where they already are. (session-settled: user-directed — chosen over a menu that also holds What's New and About.) Governs R4.
- **The macOS app menu does not change.** (session-settled: user-directed — chosen over removing Check for Updates… from it.) Governs R7.
- **The Windows tray menu keeps Check for Updates… and the Update Available item.** (session-settled: user-directed — the maintainer kept the tray part of commit `5b721aa`.) Governs R8.
- **The footer shows no checking state.** The update window that a manual check opens already shows the progress and the result. Governs R4.

### Requirements

**Footer**

- R1. The library window shows a footer along the bottom of the note list on Windows and macOS. It is always visible, also with no notes, with a note selected, and while a note records.
- R2. The footer holds a Settings control on the left, which opens the Settings window as the other Settings entry points do.
- R3. The right side of the footer shows the installed version in muted text, for example "Version 1.2.14", while no update waits.

**Updates**

- R4. A click on the version text opens a small menu with one item, Check for Updates…. That item runs the same manual check as the macOS app menu and the tray menu.
- R5. While a newer version is known and not installed, the version text is replaced by an "Update available: <version>" badge. A click on the badge opens the update window for that version.
- R6. The badge follows the same state as the tray's Update Available item: it shows when that item shows, and it goes away when that item goes away.

**Menus**

- R7. The Windows main window has no menu bar. The macOS app menu stays as it is on `main`.
- R8. The tray menu on Windows and macOS holds Open, the recording widget item, Settings…, Check for Updates…, Update Available: <version>… while an update waits, and Quit Longhand.

### Acceptance Examples

- AE1. **Covers R3, R4.** Given Longhand 1.2.14 with no update known, when the user clicks "Version 1.2.14" in the footer, then a menu opens with Check for Updates…, and no check starts until the user selects that item.
- AE2. **Covers R5, R6.** Given an automatic check found 1.2.15 and the user closed the update window without installing, then the footer shows "Update available: 1.2.15", and a click on it opens the update window for 1.2.15.
- AE3. **Covers R5, R6.** Given the footer shows "Update available: 1.2.15", when a check finds that no update waits any more, then the footer shows the installed version again.
- AE4. **Covers R1.** Given a note records, then the footer stays visible and changes only when an update becomes known.

### Scope Boundaries

- The Settings window, its About pane and What's New do not change.
- No About or What's New item in the footer or its menu.
- No new notice or message: the footer shows a status, and the update window stays the only place that offers an install (see the notice rule in `CLAUDE.md`).
- The recording widget does not change.
- Linux is not a target.

### Dependencies / Assumptions

- A local build that is not a release build gets no updates (`CONCEPTS.md`, Release). The footer still shows its version and the menu, and Check for Updates… behaves as it does from the tray in such a build.

### Outstanding Questions

**Deferred to Planning**

- How the footer reads the installed version and the waiting update, and how it stays in step with the tray item (R6).
- The exact look of the footer, the badge and the menu, within the UI rules in `CLAUDE.md` (invoke `apple-design` first) and the plain-language rule for UI text.
- How to rework commit `5b721aa`: keep the tray change (R8), and remove the Windows menu bar (R7) and the menu code that only it uses.

### Sources / Research

- `src-tauri/src/menu.rs`: the macOS app menu, and the Windows menu bar of commit `5b721aa`.
- `src-tauri/src/tray.rs`: the tray menu and the Update Available item (`set_update_item`).
- `src-tauri/src/managers/updates.rs`: `check_for_updates` and the state that the tray item follows.
- `src/home/Home.tsx`: the library window (top bar, note list, preview panel).
- `src/update/UpdateWindow.tsx`: the update window, which shows the up-to-date state of a manual check.
- Rough placement sketch from the session (private): https://claude.ai/artifact/WuKcZssd6vswyfFVWNpXrJ
