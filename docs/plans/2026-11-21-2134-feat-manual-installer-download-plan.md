---
title: Manual Installer Download - Plan
type: feat
date: 2026-11-21
topic: manual-installer-download
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Manual Installer Download - Plan

## Goal Capsule

- **Objective:** A user whose automatic update cannot install, for example because security software blocks the installer, can get the newest installer in one click from the update window and update by hand.
- **Product authority:** The decisions in this plan come from the user in the brainstorm dialogue of 2026-11-21.
- **Open blockers:** None.

---

## Product Contract

### Summary

The update window gets a way to download the newest installer in the default web browser. The "The update could not be installed" window gets a "Download Installer" button. The "A new version is available" window gets a small "Download installer" link. The user runs the downloaded installer by hand, and it updates the installed app in place.

### Problem Frame

On 2026-11-18, a Windows user at a company reported that Carbon Black App Control blocked the installer that the in-app update started. The installer was signed and valid. The most likely cause is a known agent behaviour: a new file that runs within about 60 seconds of its creation, with a certificate that the PC has not cached, is blocked (Broadcom KB 419210). Artifact Signing gives each release a new 3-day certificate, and the updater runs the installer immediately after it writes the file, so this can occur at each release.

Longhand then shows "The update could not be installed" with Close and Try Again. Try Again fails in the same way. The user must find the release page or longhand.md by hand to get the installer. A user who knows that automatic updates fail on their PC has no direct path either.

### Key Decisions

- **The browser downloads the installer, not Longhand.** The link opens the default browser at the direct download address. This adds no download code to Longhand, and the security software treats the file as an ordinary browser download that the user runs later. (session-settled: user-approved — chosen over Longhand saving the file to Downloads and showing it in Explorer or Finder: no new download and error handling, and the file does not come from the process that the security software already watches.) Governs R1, R5.
- **The link always goes to the newest release.** This matches Install, which checks again at the click and installs the newest release. (session-settled: user-directed — chosen over the version shown in the window: if a newer release came out while the window was open, the user must get the newest one.) Governs R5.
- **Both windows show it.** The failure window shows it when the automatic update failed. The offer window shows it for a user who already knows that the automatic update fails on their PC. (session-settled: user-directed — chosen over the failure window only and the offer window only.) Governs R2, R3.
- **Both platforms show it.** macOS updates can also fail, for example on a managed Mac. One behaviour on both platforms keeps the window the same. (session-settled: user-approved — chosen over Windows only.) Governs R6.

### Requirements

**Where it shows**

- R1. A click on the button or the link opens the default web browser at the installer download. Longhand itself does not download the file.
- R2. The "The update could not be installed" window has a "Download Installer" button beside Close and Try Again. Try Again stays the default button.
- R3. The "A new version is available" window has a small "Download installer" text link. It is not a third button. "Install and Relaunch" stays the default button and "Remind Me Later" does not change.
- R4. A click on the button or the link also closes the window. In the failure window this is the same as Close. In the offer window this is the same as Remind Me Later.

**What it downloads**

- R5. The download is the installer of the newest release, from the stable "latest release" download address that the README and longhand.md also use. It is not the installer of the version that the window shows.
- R6. On Windows the download is the `-setup.exe` installer. On macOS it is the `.dmg`.

### Acceptance Examples

- AE1. **Covers R2, R4, R5, R6.** **Given** a Windows PC where security software blocked the update installer, **when** the failure window shows and the user clicks "Download Installer", **then** the browser downloads the newest `Longhand_x64-setup.exe`, the window closes, and running that installer updates the installed app with notes and settings kept.
- AE2. **Covers R3, R4.** **Given** the offer window for 1.2.30, **when** the user clicks "Download installer", **then** the browser opens the download, the window closes, and the offer comes back later as after Remind Me Later.
- AE3. **Covers R5.** **Given** the offer window for 1.2.30 stayed open and 1.2.31 was released, **when** the user clicks the link, **then** the browser downloads 1.2.31. The window text still says 1.2.30, and that is accepted.
- AE4. **Covers R6.** **Given** a Mac, **when** the user clicks the link in either window, **then** the browser downloads `Longhand_aarch64.dmg`.

### Scope Boundaries

- No install steps in the window. The browser download and the installer give their own instructions.
- No change to the window text when the download can be newer than the version shown (AE3).
- No link in the "Longhand could not check for updates" window. If the check fails, the download site is probably not available either.
- No change to Try Again or to how the automatic update runs the installer, such as a delay before it runs.

### Dependencies / Assumptions

- Each release keeps the asset names `Longhand_x64-setup.exe` and `Longhand_aarch64.dmg` at `https://github.com/twistymaze/longhand/releases/latest/download/`, as `README.md` and `site/index.html` already use them.
- The Windows NSIS installer, run by hand, installs over an existing install in place. The automatic update runs this same file with `/UPDATE`.

### Outstanding Questions

**Deferred to Planning**

- The exact look and position of the link in the offer window, with the `apple-design` skill.

### Sources / Research

- `src/update/UpdateWindow.tsx`: `viewOf` defines the text and buttons of each update state.
- `src-tauri/src/managers/updates.rs` and `src-tauri/src/managers/updates/flow.rs`: the update flow, the `Failed` state, and the install attempt record that shows the failure at the next launch on Windows.
- Broadcom KB 419210, "Publisher blocks due to PARTIAL_CHAIN errors": https://knowledge.broadcom.com/external/article/419210/publisher-blocks-due-to-partial_chain-er.html
