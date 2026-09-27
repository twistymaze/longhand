---
title: Library and Desktop UI Polish - Plan
type: feat
date: 2026-09-26
topic: ui-polish
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Library and Desktop UI Polish - Plan

## Goal Capsule

- **Objective:** Longhand users can browse notes without hover-driven layout changes, adjust interface size, choose login startup, and see consistent branding without alert-sound nagging.
- **Product authority:** This Product Contract records the user-confirmed scope, including the supplied icon-generation script and the subsequent instruction to keep generated icons consistent with the SVG.
- **Open blockers:** None.

---

## Product Contract

### Summary

Make the library use stable selection and deliberate opening, remove alert-audibility warnings, and add keyboard zoom and optional login startup. Use a shared vector mark for Longhand's branding and strengthen the home header's visual hierarchy. These changes stay in one plan at the user's explicit request.

### Key Decisions

- **Click to preview, double-click to open.** Governs R3–R7. (session-settled: user-directed — chosen over hover previews and single-click opening: actual use proved too finicky.)
- **Trust the user's sound setup.** Governs R1–R2. (session-settled: user-directed — chosen over muted-output and untested-sound warnings: audibility is the user's responsibility.)
- **Keep machine preferences local.** Governs R11–R14; the confirmed defaults preserve zoom across launches and start quietly at login.
- **One vector source for the mark.** Governs R15–R17. (session-settled: user-directed — chosen over leaving the old app icon if the mark changes: all variants must remain consistent.)

### Requirements

**Alert sound**

- R1. Remove all proactive warnings or reminders about whether the alert sound can be heard, including muted output, an untested sound, and cautionary Settings copy about notification audibility.
- R2. Keep the existing failure alert and optional Play Sound action; actual recording failures and errors from a user-requested action remain reportable.

**Library selection and summary**

- R3. A single click on a note selects it and displays its summary sidebar; a double-click opens that note's detail page.
- R4. Hovering or leaving a note never changes the selection, sidebar content, or sidebar visibility.
- R5. Once the library is loaded, exactly one existing note is selected whenever any notes exist, including when the only note is recording, interrupted, processing, or lacks a summary.
- R6. Preserve a valid selection through navigation, list refreshes, and newly arriving notes; when no valid selection exists, select the most recent note by recording start time.
- R7. On a fresh launch, select the most recent note; after deleting or otherwise losing the selected note, apply R6 to the remaining notes.
- R8. The selected note's sidebar shows the available summary or its current status when a summary is unavailable, including while a note is still recording.
- R9. Only a genuinely empty library hides the summary sidebar and its shaded region, showing a full-width zero state with clear guidance to create the first note.
- R10. Preserve keyboard browsing and opening: arrow keys move selection and Enter opens the selected note, while rename and action-menu interactions retain their own behavior.

**Interface zoom**

- R11. Support standard application-focused zoom shortcuts on macOS and Windows: Command/Ctrl plus `+` or `=` to enlarge, `-` to reduce, and `0` to reset to 100%, without a visible Settings control.
- R12. Remember zoom on this machine across launches and apply the chosen scale consistently across app windows, including newly opened windows.

**Login startup**

- R13. Add a Start at login toggle that is off on a machine with no existing preference and reflects whether login startup is actually enabled.
- R14. Enabling Start at login starts Longhand quietly in the menu bar or tray after login, without opening its window or beginning a recording; disabling it stops future login launches.

**Logo and hierarchy**

- R15. Use a transparent SVG L with the pen dot for in-app branding, with a foreground color appropriate to the current light or dark appearance and no icon background rectangle.
- R16. Preserve the supplied generator's mark geometry as closely as practical, using the SVG as the shared source for the in-app logo, app icon, and tray icon.
- R17. Regenerate platform app icons from that source whenever the SVG mark differs from the existing icon, retaining the app icon's navy background and cream mark and the tray icon's template treatment.
- R18. Increase the home header's Longhand text and logo so that the branding is visually larger and more prominent than the Notes heading.

### Acceptance Examples

- AE1. **Covers R1–R2.** With laptop output muted, opening Settings or recording a note shows no audibility warning; failure alerts still operate and Play Sound remains available.
- AE2. **Covers R3–R4.** Click note A, then move the pointer over note B and outside the list: A remains selected and its sidebar remains visible; double-click B opens B.
- AE3. **Covers R5–R7.** Launch with several notes: the newest is selected. Select an older note, open it, return, and refresh the list: it remains selected even if a newer note arrives.
- AE4. **Covers R5–R9.** Delete the selected note: the newest remaining note becomes selected. Delete the final note: the shaded sidebar disappears and the zero state appears. Create the first note: on the library page, that note is selected and its sidebar shows the available status.
- AE5. **Covers R5, R8.** A library containing only a recording or processing note has a selection and a status sidebar, without pretending that a summary already exists.
- AE6. **Covers R10.** Arrow through notes and press Enter to open one; using Rename or Delete does not accidentally open the note.
- AE7. **Covers R11–R12.** Enlarge the interface with the platform shortcut, open Settings, and relaunch: the chosen scale is retained. The reset shortcut restores 100%.
- AE8. **Covers R13–R14.** On a fresh machine the toggle is off. Enable it and log in again: Longhand runs in the menu bar or tray without a window or recording. Disable it and confirm a later login does not launch it.
- AE9. **Covers R15–R18.** In light and dark appearances, the in-app L has no colored tile behind it, matches the mark in generated icons, and pairs with a Longhand title visually larger than Notes.

### Scope Boundaries

- Applies to the existing macOS and Windows desktop app.
- Existing recording navigation and real recording-readiness checks remain; R1 concerns alert audibility, not microphone or model readiness.
- No visible zoom preference, account-level preference sync, or requirement to restore the selected note across fresh launches.
- This is a polish pass, not a replacement library layout or a new logo concept.

### Outstanding Questions

- **Resolve Before Planning:** None.
- **Deferred to Planning:** Choose zoom increments and bounds, shortcut handling, and layout behavior at supported zoom levels while meeting R11–R12.
- **Deferred to Planning:** Choose login registration and preference storage, including accurate UI handling of OS changes or registration failures, while meeting R13–R14.
- **Deferred to Planning:** Choose SVG geometry precision, raster export tooling, and exact header sizing while meeting R15–R18; pixel-identical raster antialiasing is not required.

### Sources / Research

- User-confirmed scope in this conversation, including quiet startup, persistent machine-local zoom, selection for recording-only libraries, and the shared icon source.
- `src/home/Home.tsx`, `src/home/SessionRow.tsx`, and `src/home/PreviewPanel.tsx`: existing hover preview, single-click opening, selection, and empty-state behavior.
- `src/home/group.ts`: existing newest-first ordering by recording start time.
- `src/App.tsx`: Home remains mounted during note navigation, preserving its state.
- `src/session/RecordingBar.tsx` and `src/settings/Status.tsx`: muted-output warning and alert Settings copy.
- `src/settings/icons.tsx` and `scripts/icons/make_icons.py`: raster in-app icon and the same generator supplied by the user. The mark is two rounded rectangles plus a circular pen dot; the app background is RGB 28, 36, 52 and its mark is RGB 242, 236, 222.
- [Tauri webview API](https://v2.tauri.app/reference/javascript/api/namespacewebviewwindow/) and [autostart plugin](https://v2.tauri.app/plugin/autostart/): framework support for webview zoom and login startup; exact integration belongs to implementation planning.
