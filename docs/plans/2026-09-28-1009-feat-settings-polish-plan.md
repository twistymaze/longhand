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
- **Product authority:** This Product Contract. The user confirmed the scope on 2026-09-28. The Requirements win on product behaviour; the KTDs win on mechanism.
- **Open blockers:** None.
- **Execution profile:** One branch and one PR, in a worktree made from `origin/main`. Commit per unit. The Dock behaviour needs a manual test in a signed build (R3, R4).
- **Stop conditions:** Stop and ask if the bulk delete needs to change the refusal rules of `delete_refusal` for the single-model delete, or if a Settings field change needs a schema migration.
- **Product Contract preservation:** changed: R6 — "Delete all" is also off while a note is transcribing (confirmed with the user at plan scoping); AE9 added; the Outstanding Questions are resolved in R6 (button placement) and KTD4 (note count).

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
- **"Delete all" can delete models that are in use, after a warning.** It never deletes a model while a recording, a download, or a transcription is in progress, because a running step holds its model and would fail instead of wait. Governs R6, R8, R9. (session-settled: user-approved — chosen over allowing "Delete all" while a note is transcribing: that note would fail instead of wait)

### Requirements

**General tab**

- R1. The General tab shows its groups in this order: Window, Recording, Processing, Alerts.
- R2. On macOS, the Window group has a "Hide from Dock" switch, and its default is off. The switch does not show on other platforms.
- R3. When the switch is on, Longhand has no Dock icon while no Longhand window is visible. When the switch is off, the Dock icon stays after the main window closes. The menu-bar icon stays in both modes.
- R4. A change to the switch takes effect as follows. Off: the Dock icon comes back at once. On: the Dock icon goes when the last visible window closes, not while a window is open.
- R5. The Window group footer text describes the close behaviour correctly for the current platform and switch value. It says "menu bar" on macOS and "system tray" on Windows.

**Transcription tab: model deletion**

- R6. The Transcription tab has one "Delete models…" button next to "Download required models" in the Downloads group. It is off while a recording is in progress, while a download is in progress, and when no model is downloaded. In the dialog, "Delete all" is off while a note is transcribing, and the dialog says why.
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
- AE9. **Covers R6.** Given a note is transcribing and unused models exist, when the user opens the dialog, then "Delete unused" is available, and "Delete all" is off with a line that says a note is transcribing.

### Scope Boundaries

- The per-model Delete… buttons in the transcription model list do not change.
- No other General tab content changes, apart from the group order, the new switch, and the Window footer text.
- The close behaviour on Windows and Linux does not change.
- No "reset Longhand" or uninstall feature. "Delete all" removes models only.

### Dependencies / Assumptions

- The delete rules of KTD12 in `docs/plans/2026-09-25-1121-feat-recognition-models-and-live-transcript-plan.md` define "unused" (R7). "Delete all" overrides them, except for the recording in progress and downloads in progress.
- The queue skips a note whose model is missing (KTD13 of the same plan), so the notes that wait after "Delete all" do not stop other work.
- A click on the Dock icon already shows the main window (`RunEvent::Reopen` in `src-tauri/src/lib.rs`).

### Sources / Research

- `src/settings/Status.tsx` — General pane groups (Recording, Window, Processing, Alerts) and the Window footer text.
- `src-tauri/src/window.rs` — `hide_main_window`, `hide_dock_icon_if_no_window`, and `front`, which set the activation policy. The close handler hides the window on every platform.
- `src-tauri/src/lib.rs` — the Accessory policy before the event loop, and the `Reopen` handler.
- `src-tauri/src/tray.rs` — the tray icon exists on every platform.
- `src/settings/Models.tsx` — the Downloads and Supporting models groups.
- `src/settings/RecognitionModelPicker.tsx` — the hover class is on the radio button only, and the model action renders in a sibling element.
- `src-tauri/src/managers/models.rs` — `delete_refusal`.
- `src/settings/LoginItem.tsx` and `Switch` in `src/settings/ui.tsx` — the pattern for a switch row in the Window group.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **One pure function decides the Dock policy, and every policy site calls it.** The function takes the `hide_from_dock` value and whether a window is visible, and returns Regular or Accessory. It follows `toggle_action` in `src-tauri/src/window.rs`, so unit tests can cover it without a Tauri mock. The sites are the startup call in `src-tauri/src/lib.rs`, `hide_dock_icon_if_no_window`, and `front` in `src-tauri/src/window.rs`. Governs R3.
- KTD2. **The startup policy reads the setting before the event loop.** With the switch off, the app starts as Regular, so a login launch also shows the Dock icon. With it on, startup stays Accessory, as today. Governs R3; covers AE3.
- KTD3. **The setter applies the "off" change at once.** `set_hide_from_dock(false)` on macOS sets the policy to Regular. `set_hide_from_dock(true)` changes nothing until the next close, because `hide_dock_icon_if_no_window` then reads the new value. The setter returns an error on other platforms, as `set_system_audio_default` does. Governs R4.
- KTD4. **The backend owns the delete rules, and the dialog reads them.** A new command returns each downloaded model with its size and an `unused` flag, which is `delete_refusal(...).is_none()`. It also returns the count of unfinished notes (every session state except Saved and Discarded, the same rule as `delete_refusal`) and the reason that "Delete all" is off (recording, download, or transcription), if any. A live model that `ModelsManager::live_model()` still holds after Stop gives the recording reason. The frontend does not copy the rules. Governs R6, R7, R8.
- KTD5. **One bulk command deletes by scope: `unused` or `all`.** `unused` deletes only models whose `delete_refusal` is `None` at the time of the call. `all` checks for a recording, a live model (`ModelsManager::live_model()` is `Some`, which stays true while the live worker saves its last part after Stop), a download, and a session in Processing, and refuses when one exists. Then it deletes every downloaded model and does not call `delete_refusal`. Both scopes call `unload_engine` before `delete_files`, as `delete_model` does. Both continue after a failure and return the models that they could not delete. The single `delete_model` command does not change. Governs R6, R8, R10.
- KTD6. **`ConfirmDialog` gets an optional second action.** The dialog keeps Cancel focused and Escape to cancel. The second action is a button between Cancel and the primary action. Both the second action and the primary action can be off with a reason line, and `busy` turns off both. `DeleteSession` and `useConfirm` do not change. Governs R7. (session-settled: user-approved — chosen over a new dialog component: one dialog pattern stays in the app)
- KTD7. **"Show models" uses the `LicenceRow` disclosure pattern from `src/settings/About.tsx`.** It is a button with `aria-expanded` and a chevron that turns. The list shows only when it is open. Governs R11.
- KTD8. **The hover class moves from the radio button to the row wrapper.** The wrapper in `RecognitionModelPicker` gets `hover:bg-row-hover` when it is not selected, and the button keeps only its focus and active styles. Governs R12.

### High-Level Technical Design

The Dock policy decision of KTD1:

| `hide_from_dock` | Another window visible | Policy |
|---|---|---|
| off | any | Regular |
| on | yes | Regular |
| on | no | Accessory |

The startup call uses "no window visible". A setter call with "off" applies the first row at once (KTD3).

### Assumptions

- `recordingStatus` and the `recordingChanged` event give the Settings window the recording state without `useCapture`. `src/recording/useCapture.ts` shows the pattern.
- `bun run build` fails on every full `Settings` object literal in the tests until they get the new field (`docs/solutions/workflow-issues/parallel-branches-bump-same-schema-version.md`).

### Sequencing

U1 comes before U2, because U2 needs the setter and the regenerated bindings. U3 and U4 come before U5. U6 has no dependency.

---

## Implementation Units

### U1. Hide-from-Dock setting and Dock policy

**Goal:** Store the setting and apply it at every point where the app sets the activation policy.

**Requirements:** R2, R3, R4; KTD1, KTD2, KTD3.

**Dependencies:** None.

**Files:**
- `src-tauri/src/managers/settings.rs`
- `src-tauri/src/commands/settings.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/window.rs`
- `src/bindings.ts` (generated)
- `src/settings/EndpointForm.test.tsx`, `src/settings/SummaryPrompts.test.tsx`, `src/settings/Recording.test.tsx` (fixture field)

**Approach:**
1. Add `hide_from_dock: bool` to `Settings`, with the default `false` in `impl Default`. No schema bump (`#[serde(default)]`).
2. Add the pure policy function next to `toggle_action` and use it in `hide_dock_icon_if_no_window`, `front`, and the startup call.
3. At startup, read the setting before `app.set_activation_policy`.
4. Add `set_hide_from_dock` after the `set_system_audio_default` pattern, register it in `collect_commands!`, and regenerate the bindings.

**Patterns to follow:** `toggle_action` and its tests in `src-tauri/src/window.rs`; `set_system_audio_default` in `src-tauri/src/commands/settings.rs`.

**Test scenarios:**
- Covers AE1. Setting off, no other window visible: the policy is Regular.
- Covers AE2. Setting on, no other window visible: the policy is Accessory.
- Setting on, another window visible: the policy is Regular.
- Covers AE3. Startup with the setting on gives Accessory; with the setting off, it gives Regular.
- A settings file without `hide_from_dock` reads as `false`.

**Verification:** The Rust tests pass. `src/bindings.ts` contains `setHideFromDock` and has no diff after regeneration.

### U2. General tab order, switch, and footer

**Goal:** Show Window first, add the switch on macOS, and make the footer correct for each platform.

**Requirements:** R1, R2, R5; KTD3.

**Dependencies:** U1.

**Files:**
- `src/settings/Status.tsx`
- `src/settings/SettingsWindow.tsx`
- `src/settings/Status.test.tsx`

**Approach:**
1. Move the Window group above Recording.
2. Pass `settings` and `onSettings` to `StatusPane`.
3. Add a "Hide from Dock" `Switch` row in the Window group when `isMac`, after the `SystemAudio` pattern in `src/settings/Recording.tsx`, with `ErrorText` on failure.
4. Write the footer from the platform and the switch value. On macOS with the switch off: the window closes and Longhand stays in the Dock and the menu bar. With it on: Longhand stays only in the menu bar. On Windows: Longhand stays in the system tray. The show-shortcut sentence stays.

**Patterns to follow:** `SystemAudio` in `src/settings/Recording.tsx`; `renderPane` in `src/settings/Status.test.tsx`.

**Test scenarios:**
- The group headers show in the order Window, Recording, Processing, Alerts.
- On macOS, the switch shows, is off by default, and a click calls `set_hide_from_dock` with `true`.
- A failed setter shows the error text, and the switch keeps its previous value.
- On a platform other than macOS, the switch does not show, and the footer says "system tray".
- On macOS, the footer text changes when the switch value changes.

**Verification:** The component tests pass. The General tab matches R1 in the preview page.

### U3. Model deletion summary and bulk delete commands

**Goal:** Give the frontend the data for the dialog, and delete by scope with the rules in the backend.

**Requirements:** R6, R7, R8, R10; KTD4, KTD5.

**Dependencies:** None.

**Files:**
- `src-tauri/src/managers/models.rs`
- `src-tauri/src/commands/models.rs`
- `src-tauri/src/lib.rs`
- `src/bindings.ts` (generated)

**Approach:**
1. Add a pure function that takes the statuses, the settings, the live model, and the sessions, and returns the summary of KTD4. Keep it next to `delete_refusal` so that it uses the same inputs.
2. Add the summary command, with the session read and `live_model` from `delete_model`.
3. Add the bulk delete command with the refusals and the error collection of KTD5.
4. Register both commands and regenerate the bindings.

**Patterns to follow:** `delete_model` in `src-tauri/src/commands/models.rs`; the `delete_refuses_*` tests in `src-tauri/src/managers/models.rs`.

**Test scenarios:**
- Covers AE5. The selected model, its artifacts, and one other downloaded transcription model: only the other model is unused, and the unused size equals its size.
- Covers AE6. Only the models of the current choices are downloaded: no model is unused.
- A model that a Queued note needs is not unused, and the unfinished-note count includes that note.
- Saved and Discarded notes do not count as unfinished.
- Covers AE9. A session in Processing: the summary gives the transcription reason for "Delete all".
- A recording in progress, or a download in progress: "Delete all" is refused with that reason.
- The live model is set and no recording is in progress (the live worker saves its last part after Stop): "Delete all" is refused with the recording reason.
- The `unused` scope does not delete a model that became needed after the summary was made.
- A failure to delete one model does not stop the others, and the result names the failed model.

**Verification:** The Rust tests pass, and the bindings include both commands.

### U4. Second action in ConfirmDialog

**Goal:** Let one dialog offer two destructive choices.

**Requirements:** R7; KTD6.

**Dependencies:** None.

**Files:**
- `src/lib/ConfirmDialog.tsx`
- `src/lib/ConfirmDialog.test.tsx` (new)

**Approach:** Add optional `secondary` props (label, `onClick`, `disabled`, reason line), and optional `disabled` and reason props for the primary action. With none of them, the dialog renders as today.

**Patterns to follow:** the current `ConfirmDialog`. `docs/solutions/ui-bugs/wkwebview-click-does-not-focus-button-breaks-shortcut-capture.md` for focus in WKWebView.

**Test scenarios:**
- Without `secondary`, the dialog has Cancel and one action, and Cancel has focus.
- With `secondary`, both actions show, and each calls its own handler.
- A disabled secondary action cannot be clicked, and its reason line shows.
- A disabled primary action cannot be clicked, and its reason line shows.
- `busy` turns off both actions.
- Escape calls `onCancel` in both forms.

**Verification:** The new tests and the existing `DeleteSession` tests pass.

### U5. Transcription tab delete flow and collapsed list

**Goal:** Replace the Supporting models list with the "Delete models…" flow and a collapsed list.

**Requirements:** R6, R7, R8, R9, R10, R11; KTD4, KTD6, KTD7.

**Dependencies:** U3, U4.

**Files:**
- `src/settings/Models.tsx`
- `src/settings/Models.test.tsx`

**Approach:**
1. Put "Delete models…" next to "Download required models" in the Downloads group. Turn it off per R6, from `active`, the recording state, and whether any model is downloaded.
2. On click, turn off the button, load the summary, and open `ConfirmDialog`. If the summary fails to load, show the error in `ErrorText` and do not open the dialog, as `remove()` and `download()` do. Show one choice or two per R7, with "Delete unused" as the second action. Put the R8 warning and the note count in the message.
3. After the bulk delete, call `actions.refresh()` and show the models that failed in `ErrorText`.
4. Move the Supporting models rows behind a "Show models" disclosure, closed by default, and keep the rows and the footer.

**Patterns to follow:** `remove()` and `download()` in `src/settings/Models.tsx`; `LicenceRow` in `src/settings/About.tsx`; `open()` in `src/settings/Models.test.tsx`.

**Test scenarios:**
- Covers AE5. The summary has unused models: the dialog shows both choices with their sizes.
- Covers AE6. The summary has no unused models: the dialog shows only "Delete all", with the warning.
- Covers AE7. "Delete all" with two unfinished notes: the warning says two notes will wait; after confirmation, the command runs with `all`, and the tab refreshes.
- Covers AE8. A recording is in progress: the button is off.
- A download is in progress, or no model is downloaded: the button is off.
- Covers AE9. The summary gives the transcription reason: "Delete all" is off, the reason shows, and "Delete unused" still works.
- A partial failure shows the name of each failed model.
- The summary command fails: the error shows, and the dialog does not open.
- The supporting models list is hidden at first, shows after a click on "Show models", and each row still has its Delete… button.

**Verification:** The component tests pass. The flow works in the preview page with mocked commands.

### U6. Full-row hover highlight

**Goal:** Make the hover highlight cover the whole row of a model that is not selected.

**Requirements:** R12; KTD8.

**Dependencies:** None.

**Files:**
- `src/settings/RecognitionModelPicker.tsx`
- `src/settings/ModelPicker.test.tsx` or `src/settings/Models.test.tsx`

**Approach:** Move `hover:bg-row-hover` to the wrapper element for rows that are not selected, and keep `rounded-lg` and the selected background.

**Test scenarios:**
- A row that is not selected has the hover class on its wrapper, and its radio button does not.
- The selected row has no hover class.

**Verification:** In the preview page, a hover over a downloaded row that is not selected highlights the Delete… area too.

---

## Verification Contract

| Check | Command | When |
|---|---|---|
| Types and build | `bun run build` | Every unit |
| Frontend tests | `bun run test` | Every unit |
| Rust format | `cargo fmt --all --check` | U1, U3 |
| Rust lint | `cargo clippy --workspace --all-targets -- -D warnings` | U1, U3 |
| Windows check | `scripts/check-windows.sh` | U1, U3 |
| Rust tests | `cargo test --workspace --features nemo-runtime/test-helper` | U1, U3 |
| Bindings | `git diff --exit-code src/bindings.ts` after regeneration | U1, U3 |
| Provenance | `scripts/check-provenance.sh` | Before push |
| Manual Dock test | Signed build, per [Manual tests](../development-workflow.md#manual-tests) | After U2 |

The manual test checks AE1 to AE4 in a signed build, not in `tauri dev`. It includes a launch at login with the switch on and with it off.

---

## Definition of Done

- R1 to R12 are true, and AE1 to AE9 pass as tests or as manual checks.
- Every check in the Verification Contract passes locally, and CI is green on the PR.
- The user did the manual Dock test in a signed build.
- The diff has no code left from approaches that did not work.
