---
title: Recording Widget - Plan
type: feat
date: 2026-10-10
topic: recording-widget
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Recording Widget - Plan

## Goal Capsule

- **Objective:** While a note is open (recording, paused or interrupted), the user always sees that state on screen, even when Longhand is not in front. The user can pause, continue or stop the note, or open it, without a switch to Longhand first.
- **Product authority:** This Product Contract. The idea file is `docs/ideas/next/recording-state-widget.md`.
- **Open blockers:** The Paused state comes from the pause-recording work (`docs/plans/2026-10-09-1504-feat-pause-recording-plan.md`, branch `worktree-pause-recording`). That work must merge to `main` before this work starts.

---

## Product Contract

### Summary

A small pill near the bottom centre of the screen, above all other windows, on macOS and Windows. It shows while a note is Recording, Paused or Interrupted. It shows the state, the elapsed time and a level meter, and it has Pause, Continue and Stop. A click on the pill opens the note. A Settings switch (on by default) sets whether the pill shows. A hover ×, a tray item and a shortcut change that for the current note only.

### Problem Frame

The user records a meeting, then switches to other apps. When Longhand is not in front, nothing on screen says that a note is still open. Two failures follow:

- The meeting ends and the note records for a long time after it. The user has done this several times.
- The user pauses the note for a break (pause-recording), the meeting starts again, and nobody continues the note. The audio after the break is lost.

The main window, the tray and the macOS microphone indicator do not prevent either failure. The microphone indicator turns off during a pause, so a paused note shows nothing at all. The user is in a meeting, so a reminder must be visible without being loud: U7's attention budget applies (`docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`, U7).

### Key Decisions

- **One widget for both failures, not a speech-detecting reminder.** Governs R1. (session-settled: user-directed — chosen over a reminder that listens for speech while paused: the user prefers one widget that solves both problems.)
- **Handy's overlay is the model for the window and the look.** A non-activating panel above full-screen apps and all Spaces, with a dot, a level meter and a timer. Handy's live text is left out because it distracts. Governs R2, R3, R6. (session-settled: user-directed — chosen over a top-centre pill, a small corner pill with controls on hover, and a draggable pill: shown as a rough sketch.)
- **The pill shows also when Longhand is in front.** Governs R1. (session-settled: user-directed — chosen over hiding while the recording note is visible and hiding while Longhand is active: always visible is simpler and predictable.)
- **Hidden from screen sharing and screenshots.** Participants in a call do not see it, which keeps to U7. Governs R4. (session-settled: user-approved — chosen over showing in captures.)
- **Pause, Continue and Stop on the pill; Stop has no confirmation.** Resume can continue a stopped note, so a Stop by mistake is recoverable. Governs R8, R9. (session-settled: user-directed — chosen over Pause/Continue only and no controls.)
- **Interrupted shows on the pill.** It is an abnormal state that the user must act on. Governs R7. (session-settled: user-approved — chosen over hiding the pill for Interrupted.)
- **Display: Handy's rule, plus recovery when a display goes away.** Governs R5. (session-settled: user-directed — chosen over following the active display, main display only, and all displays.)
- **A per-note override in both directions, until the note stops.** When the setting is on, the user can hide the pill for one note; when it is off, the user can show it for one note. Governs R11–R14. (session-settled: user-directed — chosen over "hidden until the state changes" and over a shortcut that flips the setting.)
- **A hover × to hide, so the user does not need to remember a shortcut.** Governs R12. (session-settled: user-directed — added after the shortcut, as a way to hide that needs no shortcut.)

### Requirements

**When it shows**

- R1. The pill shows while a note is Recording, Paused or Interrupted, also when Longhand is in front. It hides at Stop, and it does not show Processing, Saved or other states after Stop.
- R2. The pill stays above other windows, including full-screen apps, on every Space (macOS) and every virtual desktop (Windows). It never takes focus from the app in front: a click on a control does not activate Longhand.
- R3. The pill sits near the bottom centre of its display, above the Dock or taskbar. The user cannot move it in this version.
- R4. The pill does not appear in screen sharing, screen recordings or screenshots, on macOS and Windows.
- R5. The pill opens on the display under the pointer when it first shows for a note, and stays on that display. If that display goes away (a laptop lid closes, a monitor is unplugged) or the screen arrangement changes, the pill moves to the same position on a remaining display.

**What it shows**

- R6. Recording: a red dot, a level meter for the microphone, the elapsed time, Pause and Stop. The elapsed time is the note's recorded time, the same value as the recording surface shows.
- R7. Paused: the word "Paused" with a distinct colour (amber), the elapsed time without change, Continue and Stop. No level meter. Interrupted: the word "Interrupted", the elapsed time without change, and Stop.
- R8. UI text uses the terms in `CONCEPTS.md`: "Continue", not "Resume", for the end of a pause.

**What it does**

- R9. Pause, Continue and Stop on the pill do the same as the same actions in the main window. Stop has no confirmation.
- R10. A click on the pill, outside its controls, shows the main window with the open note in front. This works when the main window is closed or hidden.

**Turning it off and on**

- R11. A Settings switch, "Show recording widget", sets whether the pill shows for each new note. It is on by default and is local to this computer.
- R12. While the pill shows, a × appears at its corner when the pointer is over it. A click on the × hides the pill until the note stops. The note does not change.
- R13. A "Show/Hide recording widget" action shows a hidden pill or hides a shown pill, for the open note until it stops. The action is in the tray menu and has a shortcut in Settings. The shortcut has no default chord.
- R14. Each new note starts from the Settings switch again. A change to the switch during a note takes effect at the next note and does not undo a Show/Hide for the open note.

### Acceptance Examples

- AE1. **Covers R1, R7.** Given a note that records with Longhand behind a video call, when the user pauses it from the tray, then the pill changes to "Paused" with Continue and Stop, and stays on screen.
- AE2. **Covers R2, R9.** Given the pill shows over a full-screen video call, when the user clicks Pause on the pill, then the note pauses and the video call stays in front with keyboard focus.
- AE3. **Covers R5.** Given a laptop with an external display, the pill on the laptop display and a recording note, when the user closes the lid, then the pill moves to the external display.
- AE4. **Covers R12, R14.** Given the switch is on and the user hides the pill with the ×, when the user pauses and continues the note, then the pill stays hidden. When the user stops it and starts a new note, the pill shows.
- AE5. **Covers R13, R14.** Given the switch is off and a note records, when the user chooses "Show/Hide recording widget" in the tray, then the pill shows until that note stops. The next note shows no pill.
- AE6. **Covers R4.** Given the pill shows and the user shares the screen in a call, then the shared image does not contain the pill.
- AE7. **Covers R1, R7.** Given a note whose capture fails and cannot restart, then the pill shows "Interrupted" and Stop. A click on the pill opens the note, where the problem is described.

### Scope Boundaries

- No live transcript on the pill.
- No position setting, no dragging, and no Top/Bottom choice as Handy has.
- No pill for Processing, Saved, a failed processing or any state after Stop.
- No change to the pill as a pause gets longer (no growing colour, sound or motion).
- No Continue on the Interrupted pill. The user opens the note to see the cause.
- No hide that ends when the state changes. A hidden pill stays hidden for the whole note.
- Linux. Longhand ships for macOS and Windows only.

### Dependencies / Assumptions

- The pause-recording work merges first. On that branch, Paused is a flag on the capture manager, with `pause_recording` and `continue_recording` commands; it is not a `SessionState` variant.
- The level meter and time come from the existing `CaptureTick` (4 per second, with `peak` and `recorded_ms`). Whether 4 per second looks smooth enough on a small meter is a planning question.
- Tauri 2.11 can hide a window from screen capture on macOS and Windows. Not yet verified in the Tauri source.

### Outstanding Questions

**Deferred to Planning**

- How the pill window is made on Windows, where Handy uses a topmost window with no focus instead of a macOS panel.
- How Longhand detects a removed display or a changed screen arrangement while the pill shows. Handy repositions only when the overlay shows, so it has no example for R5.
- How the tray item's label shows the current state (for example "Hide Recording Widget" when the pill shows).
- Whether the Show/Hide action works only while a note is open, and what it does otherwise.

### Sources / Research

- Handy overlay window: `reference-source/Handy/src-tauri/src/overlay.rs` (panel level, style mask, `can_join_all_spaces`, `full_screen_auxiliary`, monitor under the cursor).
- Handy overlay UI: `reference-source/Handy/src/overlay/RecordingOverlay.tsx` and `RecordingOverlay.css`.
- Handy has no screen-capture exclusion and no handling for a display change while the overlay shows.
- Longhand tray "open Longhand" path, which shows the window and the open note: `src-tauri/src/tray.rs` (`open_longhand`).
- Per-computer settings: `LocalSettings` in `src-tauri/src/managers/settings.rs`.
- Platforms: `src-tauri/tauri.conf.json` bundle targets (`app`, `dmg`, `nsis`) and the macOS and Windows CI matrix in `.github/workflows/ci.yml`.
