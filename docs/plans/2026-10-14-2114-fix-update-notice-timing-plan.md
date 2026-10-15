---
title: Update Notice Timing - Plan
type: fix
date: 2026-10-14
topic: update-notice-timing
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Update Notice Timing - Plan

## Goal Capsule

- **Objective:** A user sees an update notice only when they use Longhand, and one install takes them to the newest release.
- **Product authority:** This Product Contract. It changes R13, R16 and KTD9 of `docs/plans/2026-09-28-1240-feat-preview-builds-and-updates-plan.md` where they conflict; the other rules of that plan stay.
- **Open blockers:** None.

---

## Product Contract

### Summary

Automatic update notices wait until the user brings a Longhand window to the front. Until then, a tray menu item shows that an update is available and opens the notice. The notice is a normal window, not on top of other apps. Install always gets the newest release at the time of the click.

### Problem Frame

The update window opens as soon as a check finds a newer release. It floats over all apps, also when the user works in a different app and does not use Longhand. Other Mac apps (for example ChatGPT and Claude) show their update notice only when the user brings the app to the front.

When the window opens, it keeps the version that the check found. While the window is open, no other check runs. If a newer release comes out before the user clicks Install, Longhand downloads and installs the older release. After the relaunch, the next check finds the newer release, and the user gets a second update immediately. A saved download that installs at launch can cause the same problem.

### Key Decisions

- **Wait for a Longhand window, not for any app activation.** A window in front shows that the user uses Longhand. The tray menu and the recording widget do not show this. Governs R1, R2.
- **A tray menu item for users who keep no window open.** With "Hide from Dock" on, a user can go days with no Longhand window in front. The tray item shows the update without an interruption. (session-settled: user-directed — chosen over "wait for a window only" and over "show a normal window after a long wait (for example 3 days)": the tray item tells the user and does not interrupt.) Governs R4, R5.
- **The newest release, always.** Each step that starts a download or an install uses the newest release that a check found. This prevents two updates in a row. Governs R6 to R9.

### Requirements

**When the notice shows**

- R1. An automatic update notice shows only when a Longhand window (the main window, Settings or another app window) is the active window. If a notice is due while no Longhand window is active, it waits and shows when one becomes active.
- R2. The tray menu, the recording widget and the menu bar icon do not make a waiting notice show.
- R3. R1 applies to each automatic notice: the offer, the ready question ("Relaunch Now" or "Later") and the download error. The rules for busy (R14 and R17 of the earlier plan) still apply first. "Check for Updates…" shows its result immediately, as before.

**Tray menu item**

- R4. While a newer release is available and not installed, the tray menu shows an item that names the version, for example "Update Available: 1.0.5…". A click on the item opens the update notice and brings it to the front.
- R5. The tray item stays during the 24 hours after "Remind Me Later" or "Later". It shows nothing by itself, so it does not count as a reminder.

**Newest release**

- R6. The hourly checks continue while a notice waits or shows, and during the 24-hour delays after "Remind Me Later" and "Later". A check that finds a newer release changes the version in the notice and in the tray item. The delays still stop the notice from showing by itself.
- R7. A click on "Install and Relaunch" starts a new check and downloads the release that this check names, also when the notice shows an older version. If the check fails, the download uses the newest release that an earlier check found. The progress shows the version that downloads.
- R8. At launch, Longhand does not install a saved download when the update manifest names a newer release. It offers the newer release by the rules above.
- R9. "Relaunch Now" and the install at quit install a saved download only when no check has found a newer release. If a check has found a newer release, Longhand does not install the saved download and offers the newer release by the rules above.

**Window**

- R10. The update notice is a normal window. It does not stay on top of other apps. When the user changes to a different app, the notice goes behind that app.

### Acceptance Examples

- AE1. **Covers R1, R2.** Given Longhand runs with no window open and the user types in a different app, when an hourly check finds 1.0.5, then no window opens and the tray menu shows "Update Available: 1.0.5…". When the user opens the main window, the offer shows.
- AE2. **Covers R6, R7.** Given the offer for 1.0.4 waits for a Longhand window, when 1.0.5 comes out and the user then opens Longhand and clicks Install, then 1.0.5 downloads and installs, and after the relaunch no offer shows.
- AE3. **Covers R4, R5.** Given the user clicks "Remind Me Later", when they open the tray menu during the next 24 hours, then the tray item is there and a click on it opens the offer.
- AE4. **Covers R8.** Given 1.0.4 is downloaded and saved, and the user quits before an install, when Longhand launches and the manifest names 1.0.5, then Longhand does not install 1.0.4 and offers 1.0.5.
- AE5. **Covers R3.** Given no Longhand window is active, when the user chooses "Check for Updates…" in the tray menu, then the result shows immediately.
- AE6. **Covers R10.** Given the offer shows, when the user changes to a different app, then the offer goes behind that app's windows.
- AE7. **Covers R6, R9.** Given 1.0.4 is saved and the user clicks "Later", when 1.0.5 comes out during the 24-hour delay and the user quits or clicks "Relaunch Now" from the tray item, then 1.0.4 does not install and the offer names 1.0.5.

### Scope Boundaries

- Deferred: a badge or dot on the Dock icon or the tray icon.
- Unchanged: the busy rules, the 24-hour delays, the consent before a download, and the update settings.
- Unchanged: the Windows app, which does not check for updates.

### Outstanding Questions

**Deferred to Planning**

- How the window gets focus when the user clicks the tray item, given that the notice opens without focus today (`src-tauri/src/window.rs`).

### Sources / Research

- `src-tauri/src/managers/updates/flow.rs` — `timer` checks only from `Idle` and `WaitingForQuit`, so no check runs while an offer waits or shows.
- `src-tauri/src/managers/updates.rs` — `download` uses the `Update` of the last check; `install_at_launch` installs the saved version when the manifest names that version or a newer one.
- `src-tauri/src/window.rs` — `show_update_window` builds the window with `always_on_top(true)` and `focused(false)`.
- `src-tauri/src/tray.rs` — the tray menu, which already has "Check for Updates…".
