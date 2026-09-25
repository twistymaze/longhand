---
title: Custom Shortcuts - Plan
type: feat
date: 2026-09-25
topic: custom-shortcuts
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Custom Shortcuts - Plan

## Goal Capsule

- **Objective:** A user starts and stops a recording with keys that they choose, and brings Longhand forward from the menu bar with a key when they want to. A user who does not want a global key for recording can have none.
- **Means:** Two shortcut rows in the General pane of Settings, each set by pressing the keys, following Handy's shortcut input.
- **Product authority:** The Product Contract wins on product behavior. The walking-skeleton plan (`docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`) still governs what a press of the recording shortcut does (R6c, KD14).
- **Open blockers:** None.

---

## Product Contract

### Summary

The General pane of Settings gets two shortcut rows. The user sets each one by pressing the keys. The recording shortcut keeps its default chord and its start and stop behavior, and the user can change it, reset it or clear it. A new "Show Longhand" shortcut has no chord by default. When set, it shows the main window, and a second press hides the window again.

### Problem Frame

The recording shortcut is fixed at `CommandOrControl+Alt+Shift+L`. It is registered once at launch from settings, and no screen changes it. A user whose other apps already use that chord, or who wants a shorter chord, has no choice. When the main window is closed, Longhand stays in the menu bar. The only way to bring the window back is the menu-bar item, or the recording shortcut, which also starts a recording.

### Key Decisions

- **The recording shortcut can be cleared.** A user who starts recordings only from New Note can have no global key. Governs R3, R13. (session-settled: user-directed — chosen over "change only, never empty": the user wants the choice to have no recording shortcut.)
- **A cleared recording shortcut does not make Longhand "Not ready".** No shortcut is a choice, not a fault. A chord that fails to register is still a fault. Governs R13. (session-settled: user-approved — chosen over treating a cleared shortcut as Not ready: readiness is about whether a recording can start, and New Note still starts one.)
- **The show shortcut toggles.** One key brings the window forward and puts it away again. Governs R6. (session-settled: user-approved — chosen over a show-only shortcut that does nothing when the window is in front.)
- **The show shortcut has no chord by default.** A global chord that the user did not ask for can take a key from another app. Governs R5.
- **Chord rules follow Handy.** A chord needs a modifier, the two shortcuts cannot share a chord, and a chord that the system refuses leaves the previous chord in place. Governs R8–R10.

### Requirements

**The recording shortcut**

- R1. The General pane shows the recording shortcut with its current chord. The user changes it by selecting the row's control and pressing the new keys. The new chord works at once, with no relaunch.
- R2. A reset control returns the recording shortcut to the default chord, `CommandOrControl+Alt+Shift+L`.
- R3. The user can clear the recording shortcut. With no chord, no global key starts or stops a recording. New Note in the main window still starts one.
- R4. A press of the recording shortcut does what it does today: it starts a new recording, or stops the recording in progress (walking-skeleton R6c).

**The show shortcut**

- R5. A second row, "Show Longhand", has no chord by default. The user can set, change and clear it the same way as the recording shortcut. It has no reset control, because its default is no chord.
- R6. When the main window is visible and is the focused window of the frontmost app, a press hides the main window. At any other time, a press shows the main window, unminimizes it and focuses it. This includes when the window is closed, minimized, or behind another app or the Settings window.
- R7. Showing or hiding the main window has no effect on a recording. The window shows the page that it showed when it was hidden.

**Setting a chord**

- R8. A chord has one or more modifier keys (Command, Control, Option or Alt, Shift) and one other key. The row refuses a chord with no modifier and says why.
- R9. The two shortcuts cannot have the same chord. The row refuses the duplicate and says why.
- R10. When the system or another app refuses a chord, the row says that the chord is not available. The previous chord stays saved and active.
- R11. While the user sets a chord, both shortcuts are paused, so the keys pressed do not start a recording or show the window. Escape, or a click away from the control, cancels and keeps the previous chord.
- R12. Both chords are saved and are registered again at each launch. Each row shows its chord in the form that the platform uses (symbols on macOS, `Ctrl+Alt+…` on Windows).

**Status and text that names the shortcut**

- R13. "Ready to record" depends only on the microphone and the destination when the recording shortcut is cleared. The shortcut row then says "Off" in a neutral tone. A saved chord that fails to register at launch still shows as a problem and makes the status "Not ready", as it does today.
- R14. When the notes list is empty, its line names the recording shortcut if one is set. If none is set, the line names the New Note button.

### Acceptance Examples

- AE1. **Covers R1, R11.** **Given** the recording shortcut is `⌘⌥⇧L`, **when** the user selects its control and presses `⌃⌥R`, **then** the row shows `⌃⌥R`, `⌃⌥R` starts a recording from another app, `⌘⌥⇧L` does nothing, and no recording started while the user pressed the keys.
- AE2. **Covers R3, R13, R14.** **Given** the user clears the recording shortcut and there are no notes, **when** they look at Settings and Home, **then** the shortcut row says "Off", the status says "Ready", and the empty notes list names New Note.
- AE3. **Covers R6.** **Given** a "Show Longhand" chord is set and the main window is closed, **when** the user presses it from another app, **then** the main window comes forward with focus. **When** they press it again, **then** the window hides and Longhand stays in the menu bar.
- AE4. **Covers R6.** **Given** the main window is open but behind another app, **when** the user presses the show chord, **then** the window comes forward. It does not hide.
- AE5. **Covers R7, R4.** **Given** a recording is in progress and the main window is in front, **when** the user presses the show chord, **then** the window hides and the recording continues. A press of the recording shortcut then stops the recording.
- AE6. **Covers R9.** **Given** the recording shortcut is `⌘⌥⇧L`, **when** the user presses `⌘⌥⇧L` in the "Show Longhand" control, **then** the control refuses it and the show shortcut keeps its previous value.
- AE7. **Covers R10.** **Given** another app holds `⌘⇧Space`, **when** the user presses it in the recording shortcut control, **then** the row says the chord is not available and the previous chord still starts a recording.

### Scope Boundaries

- One recording shortcut for each destination folder. This stays parked (`docs/ideas/shortcut-per-folder.md`).
- A chord made only of modifier keys, or one that uses the fn key. These need Handy's second key-capture method, which reads keys in the backend.
- Shortcuts for other actions, such as opening Settings or adding a note.
- A shortcut that works only while Longhand is the frontmost app. Both shortcuts are global.

### Dependencies / Assumptions

- The user is the only user and asked for this directly. There is no other evidence of need to find.
- A chord that another app holds is refused at registration on macOS and on Windows. On macOS, some system chords can be registered but never reach Longhand. Planning checks what `tauri-plugin-global-shortcut` reports in each case.

### Outstanding Questions

**Deferred to Planning**

- Which of Handy's two capture methods to borrow: key events in the web view (`GlobalShortcutInput.tsx`) or in the backend (`HandyKeysShortcutInput.tsx`). The web-view method covers R8's chord rules.
- How the backend tells "visible and focused" from "visible but behind another app" on each platform, for R6.

### Sources / Research

- `src-tauri/src/shortcut.rs` — today's registration, once at launch, and the start or stop toggle.
- `src-tauri/src/managers/settings.rs` — the `shortcut` field and `DEFAULT_SHORTCUT`.
- `src-tauri/src/commands/readiness.rs` — `armed` requires `shortcut.registered` today.
- `src/settings/Status.tsx` — the General pane's shortcut row, which only shows status today.
- `src-tauri/src/window.rs` — `show_main_window`, which the show shortcut can reuse.
- `reference-source/Handy/src-tauri/src/shortcut/mod.rs` — `change_binding`, `reset_binding`, `suspend_all_bindings` and `resume_all_bindings`, and the restore of the previous binding after a failed change.
- `reference-source/Handy/src/components/settings/GlobalShortcutInput.tsx` — the press-the-keys control.
