---
title: Settings polish - Plan
type: feat
date: 2026-09-28
topic: settings-polish
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Settings polish - Plan

## Goal Capsule

- **Objective:** A user can keep Longhand in the macOS Dock after the window closes, or hide it there, can find the Window settings first on the General tab, and can recover model disk space with one button, without the need to understand supporting models.
- **Product authority:** This Product Contract. The user confirmed the scope on 2026-09-28.
- **Open blockers:** None.

---

## Product Contract

### Summary

The General tab shows Window first and adds a "Hide from Dock" switch on macOS. The switch is off by default, so Longhand stays in the Dock after the window closes. The Transcription tab replaces the Supporting models list with one "Delete models…" button and a list that stays collapsed until the user opens it. The hover highlight on a transcription model row also covers that model's Delete… button.

### Problem Frame

Today, on every platform, the close button hides the main window. On macOS, Longhand then removes its Dock icon and stays only in the menu bar. Users who expect the standard macOS behaviour must find the menu-bar icon to bring the window back. On Windows, the hidden window has no taskbar button, and Longhand stays in the system tray, but the Window footer says "menu bar" there. The Transcription tab shows a list of supporting models (speaker segmentation, speaker embedding, and transcription model files). A user who does not know these terms sees technical detail that they cannot act on. In the transcription model list, the hover highlight stops above the Delete… button of a downloaded model that is not selected, so the row looks broken.

### Key Decisions

- **The switch is "Hide from Dock", and its default is off.** Staying in the Dock is the standard macOS behaviour, so the switch names the change from that behaviour. Existing installs get the same default. Nobody uses Longhand yet, so the change needs no migration. Governs R2, R3. (session-settled: user-directed — chosen over a "Keep in Dock" switch with the default on: the switch must name the change from standard macOS behaviour)
- **The switch is macOS only.** Windows keeps its current behaviour: the close button hides the window to the system tray, and the Minimize button keeps the taskbar button. That is the usual pattern for Windows tray apps. Governs R2. (session-settled: user-approved — chosen over a "Hide from taskbar" switch on Windows: Minimize already gives Windows users the other choice)
- **When the switch is on, the Dock icon is absent every time no window is visible.** This applies after a close, after a launch at login, and after the Settings window closes. Governs R3.
- **One "Delete models…" button opens a dialog that offers the correct choices.** It replaces a list of per-model rows as the primary control. Governs R6, R7, R8. (session-settled: user-approved — chosen over two separate buttons and over an unused-only button: the two reasons to delete are to recover space and to clean up before the user removes the app)
- **"Unused" includes transcription models that are not selected.** The reason to delete unused models is to recover disk space, and a transcription model that is not selected is the largest unused item. Governs R7.
- **"Delete all" can delete models that are in use, after a warning.** It never deletes a model while a recording or a download is in progress. Governs R6, R8, R9.

### Requirements

**General tab**

- R1. The General tab shows its groups in this order: Window, Recording, Processing, Alerts.
- R2. On macOS, the Window group has a "Hide from Dock" switch, and its default is off. The switch does not show on other platforms.
- R3. When the switch is on, Longhand has no Dock icon while no Longhand window is visible. When the switch is off, the Dock icon stays after the main window closes. The menu-bar icon stays in both modes.
- R4. A change to the switch takes effect as follows. Off: the Dock icon comes back at once. On: the Dock icon goes when the last visible window closes, not while a window is open.
- R5. The Window group footer text describes the close behaviour correctly for the current platform and switch value. It says "menu bar" on macOS and "system tray" on Windows.

**Transcription tab: model deletion**

- R6. The Transcription tab has one "Delete models…" button near "Download required models". It is off while a recording is in progress, while a download is in progress, and when no model is downloaded.
- R7. The button opens a dialog. When one or more downloaded models are unused, the dialog offers "Delete unused" and "Delete all", each with the disk space that it recovers. When no downloaded model is unused, the dialog asks the user only to confirm "Delete all". An unused model is a downloaded model that the app can delete today without an override: it is not selected, not part of a pending configuration, not in use by a recording, and not needed by an unfinished note.
- R8. The "Delete all" choice deletes every downloaded model. Before the deletion, it tells the user that Longhand cannot transcribe new notes until the user downloads the models again. It also shows how many unfinished notes will wait for the models.
- R9. After "Delete all", the transcription model choice and the other options stay as they are. The General tab and the Transcription tab show that models are missing, and "Download required models" restores them.
- R10. After a deletion, the tab shows the new model state. A failure to delete one model does not stop the deletion of the others, and the tab shows which models it could not delete.
- R11. The list of individual supporting models is collapsed by default behind a "Show models" control. When it is open, the list shows each downloaded supporting model with its size and its Delete… button, as today.

**Transcription tab: hover highlight**

- R12. In the transcription model list, the hover highlight of a model that is not selected covers the full row, including the Delete… button area and any download status.

### Acceptance Examples

- AE1. **Covers R3.** Given "Hide from Dock" is off, when the user closes the main window, then the Dock icon stays, and a click on it shows the window.
- AE2. **Covers R3, R4.** Given the main window is open and "Hide from Dock" is off, when the user turns the switch on, then the Dock icon stays until the user closes the last window, and then it goes.
- AE3. **Covers R3.** Given "Hide from Dock" is on, when Longhand launches at login with its window hidden, then no Dock icon shows.
- AE4. **Covers R4.** Given "Hide from Dock" is on and no window is visible, when the user opens Settings from the menu bar and turns the switch off, then the Dock icon shows at once and stays after Settings closes.
- AE5. **Covers R7.** Given the selected model and its supporting models are downloaded, and one other transcription model is downloaded, when the user clicks "Delete models…", then the dialog offers "Delete unused" with the size of the other transcription model, and "Delete all" with the total size.
- AE6. **Covers R7, R8.** Given only the models of the current choices are downloaded, when the user clicks "Delete models…", then the dialog asks the user only to confirm "Delete all" and shows the warning.
- AE7. **Covers R8, R9.** Given two notes wait in the queue, when the user confirms "Delete all", then all models are deleted, the warning said that two notes will wait, the notes stay in the queue, and General shows that models are missing.
- AE8. **Covers R6.** Given a recording is in progress, then "Delete models…" is off.

### Scope Boundaries

- The per-model Delete… buttons in the transcription model list do not change.
- No other General tab content changes, apart from the group order, the new switch, and the Window footer text.
- The close behaviour on Windows and Linux does not change.
- No "reset Longhand" or uninstall feature. "Delete all" removes models only.

### Dependencies / Assumptions

- The delete rules of KTD12 in `docs/plans/2026-09-25-1121-feat-recognition-models-and-live-transcript-plan.md` define "unused" (R7). "Delete all" overrides them, except for the recording in progress and downloads in progress.
- The queue skips a note whose model is missing (KTD13 of the same plan), so the notes that wait after "Delete all" do not stop other work.
- A click on the Dock icon already shows the main window (`RunEvent::Reopen` in `src-tauri/src/lib.rs`).

### Outstanding Questions

**Deferred to Planning**

- Where the "Delete models…" button sits: in the Downloads group next to "Download required models", or at the top of the collapsed group.
- The count of unfinished notes that R8 shows: which session states count, per KTD12's list of unfinished states.

### Sources / Research

- `src/settings/Status.tsx` — General pane groups (Recording, Window, Processing, Alerts) and the Window footer text.
- `src-tauri/src/window.rs` — `hide_main_window`, `hide_dock_icon_if_no_window`, and `front`, which set the activation policy. The close handler hides the window on every platform.
- `src-tauri/src/lib.rs` — the Accessory policy before the event loop, and the `Reopen` handler.
- `src-tauri/src/tray.rs` — the tray icon exists on every platform.
- `src/settings/Models.tsx` — the Downloads and Supporting models groups.
- `src/settings/RecognitionModelPicker.tsx` — the hover class is on the radio button only, and the model action renders in a sibling element.
- `src-tauri/src/managers/models.rs` — `delete_refusal`.
- `src/settings/LoginItem.tsx` and `Switch` in `src/settings/ui.tsx` — the pattern for a switch row in the Window group.
