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

- **Objective:** On Windows and macOS, a user can open Settings, see the installed version, check for updates, and see that an update waits, from the library window, with no menu bar in that window.
- **Means:** A footer component along the bottom of the library window that reads the installed version and the offered update from the update manager (KTD1, KTD2).
- **Product authority:** The maintainer settled this scope in the 2026-10-29 session. The Windows tray items and the unchanged macOS app menu are part of the settled scope. The Product Contract has precedence over the Planning Contract.
- **Execution profile:** One branch, `worktree-windows-app-menu` (worktree `.claude/worktrees/windows-app-menu`), rebased on `main`. It already holds commit `5b721aa`. One PR with a `feat:` title, as a patch release.
- **Stop conditions:** Stop and ask when the footer cannot follow the tray item's state without a change to the update flow (`src-tauri/src/managers/updates/flow.rs`).
- **Finish:** `ce-work` implements and verifies. The maintainer tests a Windows build on nebula and a macOS build before the PR.
- **Open blockers:** None.

---

## Product Contract

### Summary

A thin footer along the bottom of the note list in the library window, on Windows and macOS. Settings is on the left. The current version is on the right, and a click on it opens a menu with Check for Updates…. While an update waits, the version changes to an Update available badge that opens the update window. The Windows menu bar that commit `5b721aa` added is removed.

### Problem Frame

On Windows, the main window had no menu, so Settings and Check for Updates were reachable only through the tray icon or Ctrl+,. On Windows the tray menu opens with a right-click, and Windows often hides the icon in the notification-area overflow, so most users would not find either action. The tray menu also did not offer Check for Updates on Windows, although Windows has had in-app updates since PR #79.

A first fix (commit `5b721aa` on branch `worktree-windows-app-menu`, not pushed) added a native File and Help menu bar to the Windows main window. The maintainer tested it on Windows: it works, but a menu bar that holds four items and is always visible looks out of place in this window. macOS does not have the problem, because the app menu holds About, Check for Updates… and Settings….

### Key Decisions

- **A footer along the bottom of the library window, not a button in the top bar or an always-visible right panel.** It costs about one line of height and no list width, and the right panel keeps its current rule (it shows only for a selected note). (session-settled: user-directed — chosen over a Settings button and update badge in the top bar, and over a right panel that is always visible with a footer: the maintainer chose the status-bar shape.) Governs R1, R2.
- **The version text opens a menu, and the menu starts the check.** A click on a version label does not start a network check by itself, because the user would not expect a label to start work. (session-settled: user-directed — chosen over a direct check on click: the maintainer wants no action that the user does not see coming.) Governs R4.
- **The version menu holds only Check for Updates….** What's New and About stay in Settings, where they already are. (session-settled: user-directed — chosen over a menu that also holds What's New and About.) Governs R4.
- **The macOS app menu does not change.** (session-settled: user-directed — chosen over removing Check for Updates… from it.) Governs R8.
- **The Windows tray menu keeps Check for Updates… and the Update Available item.** (session-settled: user-directed — the maintainer kept the tray part of commit `5b721aa`.) Governs R9.
- **The footer shows no checking state.** The update window that a manual check opens already shows the progress and the result. Governs R4.

### Requirements

**Footer**

- R1. The library window shows a footer along its whole bottom edge, under the note list and the preview panel, on Windows and macOS. It is the app's footer: the text has the size of the top bar's buttons, and Settings has a gear icon (session-settled: user-directed after the first build). It shows whenever the note list shows: also with no notes, with a note selected, and while a note records. It does not show on a note's page.
- R2. The footer holds a Settings control on the left, which opens the Settings window as the other Settings entry points do.
- R3. The right side of the footer shows the installed version in muted text, for example "Version 1.2.14", while no update waits.

**Updates**

- R4. A click on the version text opens a small menu with one item, Check for Updates…. That item runs the same manual check as the macOS app menu and the tray menu.
- R5. While a newer version is known and not installed, the version text is replaced by an "Update available: <version>" badge. A click on the badge opens the update window for that version.
- R6. The badge follows the same state as the tray's Update Available item: it shows when that item shows, and it goes away when that item goes away.

**Keyboard**

- R7. The Settings control, the version text and the badge are buttons in the tab order after the note list. Enter, Space or Down Arrow opens the version menu, and Escape closes it and returns the focus to the version button.

**Menus**

- R8. The Windows main window has no menu bar. The macOS app menu stays as it is on `main`.
- R9. The tray menu on Windows and macOS holds Open, the recording widget item, Settings…, Check for Updates…, Update Available: <version>… while an update waits, and Quit Longhand.

**Closing the window**

- R10. On Windows, closing the main window quits Longhand by default, as is usual on Windows. While a note records, a close hides the window and the recording continues. macOS does not change: a close hides the window. (session-settled: user-directed.)
- R11. Settings, General, Window has a Windows-only switch, "Keep running in the system tray", off by default. When it is on, a close hides the window and Longhand keeps running in the system tray. The group footer says what a close does. The label follows the usual wording of other apps (Slack, Teams) and the Microsoft style guide term for Windows 11.

### Acceptance Examples

- AE1. **Covers R3, R4.** Given Longhand 1.2.14 with no update known, when the user clicks "Version 1.2.14" in the footer, then a menu opens with Check for Updates…, and no check starts until the user selects that item.
- AE2. **Covers R5, R6.** Given an automatic check found 1.2.15 and the user closed the update window without installing, then the footer shows "Update available: 1.2.15", and a click on it opens the update window for 1.2.15.
- AE3. **Covers R5, R6.** Given the footer shows "Update available: 1.2.15", when a check finds that no update waits any more, then the footer shows the installed version again.
- AE4. **Covers R1.** Given a note records and the user goes back to the note list, then the footer shows, and it changes only when an update becomes known or stops being known.

### Scope Boundaries

- The Settings window, its About pane and What's New do not change.
- No About or What's New item in the footer or its menu.
- No new notice or message: the footer shows a status, and the update window stays the only place that offers an install (see the notice rule in `CLAUDE.md`).
- The recording widget does not change.
- Linux is not a target.

### Dependencies / Assumptions

- A local build that is not a release build gets no updates (`CONCEPTS.md`, Release). The footer still shows its version and the menu, and Check for Updates… behaves as it does from the tray in such a build.

### Outstanding Questions

**Deferred to Implementation**

- The exact look of the footer, the badge and the menu, within the UI rules in `CLAUDE.md` (invoke `apple-design` first) and the plain-language rule for UI text. U3 states the constraints.

The planning questions about the update state (R6) and commit `5b721aa` are answered by KTD1, KTD2 and KTD4.

### Sources / Research

- `src-tauri/src/menu.rs`: the macOS app menu, and the Windows menu bar of commit `5b721aa`.
- `src-tauri/src/tray.rs`: the tray menu and the Update Available item (`set_update_item`).
- `src-tauri/src/managers/updates.rs`: `check_for_updates` and the state that the tray item follows.
- `src/home/Home.tsx`: the library window (top bar, note list, preview panel).
- `src/update/UpdateWindow.tsx`: the update window, which shows the up-to-date state of a manual check.
- Rough placement sketch from the session (private): https://claude.ai/artifact/WuKcZssd6vswyfFVWNpXrJ

---

## Planning Contract

Product Contract preservation: Product Contract unchanged.

### Key Technical Decisions

- KTD1. **The update manager announces the offered version with its own event and command.** The footer needs the same version as the tray item (`UpdateFlow::tray_version` in `src-tauri/src/managers/updates/flow.rs`). `UpdateStateChanged` does not carry it, and it fires only when the state or the focus flag changes, so the footer cannot derive the offer from it. The manager emits an `UpdateOfferChanged { version: Option<String> }` event at the point where it calls `crate::tray::set_update_item`, only when the version differs from the last emitted value, and a new `update_offer` command returns the current value for a page that opens later. One source feeds both the tray item and the footer. Governs R5, R6.
- KTD2. **The badge reuses the tray item's action.** A new `open_update_offer` command calls `UpdateManager::tray_open`, which already opens the update window for the offered release (flow tests `covers_ae3_the_tray_item_opens_the_offer_during_the_remind_delay` and `the_tray_item_does_nothing_while_installing`). The footer menu's item calls the existing `check_for_updates` command, which already brings an open update window to the front. Governs R4, R5.
- KTD3. **The footer reuses `Menu` from `src/session/SessionMenu.tsx` and `commands.buildInfo()`.** `Menu` with the `floating` prop set to the version button's ref is placed in a portal with `flip()`, so it opens upward near the window bottom, as the Add App menu in `src/settings/Recording.tsx` does. Without `floating`, the list column's `overflow-hidden` would clip it. `Menu` handles the arrow keys and Escape inside the menu, and the caller returns the focus in `onClose`, as `NewNoteButton` does. `buildInfo()` already gives the version that Settings > About shows. Governs R3, R4, R7.
- KTD4. **Commit `5b721aa` stays, and a new commit reverts its menu-bar part.** The tray change in `src-tauri/src/tray.rs` stays as it is (R9). The new commit restores `src-tauri/src/menu.rs`, `src-tauri/src/window.rs` and the `src/main.tsx` comment to their state on `main`. In `src-tauri/src/lib.rs` it reverts only the two lines that `5b721aa` changed: `#[cfg(target_os = "macos")]` goes back on `mod menu;`, and the `menu::handle_menu_events(&handle);` call is removed. The rest of `lib.rs` keeps U1's registrations. The `Ctrl+,` handler in `src/main.tsx` stays, because Windows still has no app menu. Governs R8, R9.
- KTD5. **The footer is a child of the library page in `src/home/Home.tsx`, below the row that holds the note list and the preview panel.** It spans the whole window (R1). Governs R1.

### Assumptions

- The footer shows `buildInfo().version` without the commit, for example "Version 1.2.14". A local build shows its local version (0.1.0).
- No new strings appear in Settings, the tray or the update window.

---

## Implementation Units

### U1. Announce the offered update to pages

- **Goal:** Pages can read and follow the version that the tray's Update Available item shows.
- **Requirements:** R5, R6.
- **Dependencies:** None.
- **Files:** `src-tauri/src/managers/updates.rs`, `src-tauri/src/commands/updates.rs`, `src-tauri/src/lib.rs` (command and event registration), `src/bindings.ts` (generated).
- **Approach:** Add the `UpdateOfferChanged` event and keep the last emitted version in the manager, so the event fires only on a change (KTD1). Add the `update_offer` command, which reads `tray_version()` under the flow lock, and the `open_update_offer` command, which calls `tray_open` (KTD2). Register both with tauri-specta and regenerate the bindings as the repo does for other commands.
- **Patterns:** `UpdateStateChanged` and the `update_state` / `check_for_updates` commands in `src-tauri/src/commands/updates.rs`; the queued-text dedupe in `tray::set_update_item`.
- **Test scenarios:** In `src-tauri/src/managers/updates.rs` tests: a change from no offer to 1.2.15 reports a change; the same version again reports none; a change back to no offer reports a change. Keep the decision in a small pure function so the test needs no app handle.
- **Verification:** `cargo test -p longhand --lib updates`; the bindings contain `updateOffer`, `openUpdateOffer` and `updateOfferChanged`.

### U2. Remove the Windows menu bar

- **Goal:** The Windows main window has no menu bar; the macOS menu and the tray change stay.
- **Requirements:** R8, R9.
- **Dependencies:** None.
- **Files:** `src-tauri/src/menu.rs`, `src-tauri/src/window.rs`, `src-tauri/src/lib.rs`, `src/main.tsx`.
- **Approach:** Restore `menu.rs`, `window.rs` and `src/main.tsx` to `main`, and revert the two `5b721aa` lines in `lib.rs` (KTD4). Keep `src-tauri/src/tray.rs` from `5b721aa`.
- **Test scenarios:** None new; the existing `menu.rs` tests return with the file.
- **Verification:** `git diff main -- src-tauri/src/menu.rs src-tauri/src/window.rs src/main.tsx` is empty; `scripts/check-windows.sh` passes; `cargo clippy -p longhand --all-targets -- -D warnings` passes.

### U3. Library footer

- **Goal:** The footer along the bottom of the library window with Settings, the version menu and the update badge.
- **Requirements:** R1, R2, R3, R4, R5, R6, R7; AE1, AE2, AE3, AE4.
- **Dependencies:** U1.
- **Files:** `src/home/LibraryFooter.tsx` (new), `src/home/LibraryFooter.test.tsx` (new), `src/home/Home.tsx`.
- **Approach:** Invoke `apple-design` before the visual work. The component reads `buildInfo()` and `updateOffer()` at mount and follows `updateOfferChanged`. Settings calls `commands.openSettings(null)`. The version is a quiet text button with `aria-haspopup="menu"` and `aria-expanded`. Enter and Space open the menu as a button click, and its own ArrowDown key handler opens it too, because `Menu` handles arrows only once it is open. It opens a floating `Menu` (KTD3) with one item, "Check for Updates…", which calls `commands.checkForUpdates()`. With an offer, the badge "Update available: <version>" replaces the version button and calls `commands.openUpdateOffer()`. Settings never truncates; the right side truncates first. No motion beyond the existing `Menu` behavior, and no change while a note's page shows (the footer is not on that page). Place it in `Home.tsx` below the list and preview row (KTD5).
- **Patterns:** `src/settings/Recording.tsx` (`<Menu floating={addButton}>`), `src/home/NewNoteButton.tsx` (focus return in `onClose`), `src/settings/About.tsx` (`buildInfo`), `src/home/LibraryNotices.test.tsx` (`mockTauri` and `tauri.emit`).
- **Test scenarios** in `src/home/LibraryFooter.test.tsx`:
  - Covers AE1: shows "Version 1.2.14"; a click opens a menu with "Check for Updates…"; no `check_for_updates` call happens until the item is selected; then exactly one call.
  - Covers AE2: with `update_offer` returning "1.2.15", the badge shows "Update available: 1.2.15" and the version button does not; a click calls `open_update_offer`.
  - Covers AE3: an `update-offer-changed` event with no version replaces the badge with the version again.
  - Covers R2: Settings calls `open_settings` with no pane.
  - Covers R7: Enter, and separately Down Arrow, on the version button opens the menu; Escape closes it and the focus returns to the version button.
  - A failed `build_info` call leaves the footer without version text and with Settings still working.
- **Verification:** `npx vitest run src/home/LibraryFooter.test.tsx src/home/Home.test.tsx`.


### U4. Quit on close on Windows

- **Goal:** R10, R11.
- **Files:** `src-tauri/src/window.rs` (`close_action`), `src-tauri/src/tray.rs` (`is_recording`), `src-tauri/src/managers/settings.rs` (local `keep_running_in_tray`, no migration: a missing value reads as off), `src-tauri/src/commands/settings.rs` (`set_keep_running_in_tray`, refused on macOS), `src/settings/Status.tsx`.
- **Test scenarios:** `close_action` for each input; the setting is local, off by default and kept when stored; the switch and the footer text on Windows.
---

## Verification Contract

| Check | Command | When |
| --- | --- | --- |
| Rust tests | `cargo test -p longhand --lib` | After U1 and U2 |
| Lint | `cargo clippy -p longhand --all-targets -- -D warnings`, `cargo fmt --all --check` | Before each push |
| Windows lint | `scripts/check-windows.sh` | After U2 |
| Frontend | `npm run typecheck`, `npm test` | After U3 |
| Windows app | Build on nebula (`C:\Users\erv\build-menu.ps1`, `--no-bundle`) and run `target\release\longhand.exe` | Before the PR |
| macOS app | Signed local build per `docs/development-workflow.md#manual-tests` | Before the PR |

Manual checks on both platforms: no menu bar on the Windows main window; the macOS app menu unchanged; the footer along the bottom of the window with Settings and the version; on Windows a close quits, with the switch on it hides to the tray, and during a recording it hides; the version menu runs a check that opens the update window; the tray menu has Check for Updates….

---

## Definition of Done

- R1–R11 hold, and AE1–AE4 pass as tests or manual checks.
- All Verification Contract checks pass, and the maintainer has checked the Windows and macOS builds.
- The branch holds no code from the removed Windows menu bar, and no experimental code from abandoned approaches.
- The PR title is a `feat:` Conventional Commit without `+`, and it names the footer, the Windows tray change and the close behaviour.
