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

- **Objective:** A user whose automatic update cannot install, or who knows that it will not, for example because security software blocks the installer, can get the newest installer in one click from the update window and update by hand.
- **Product authority:** The decisions in this plan come from the user in the brainstorm dialogue of 2026-11-21.
- **Means:** A new update-window choice that the update flow handles as one action: it opens the installer download in the browser and closes the window (KTD1).
- **Open blockers:** None.
- **Execution profile:** Lightweight. One PR on one branch, two units. Work in a worktree from `origin/main`.
- **Stop conditions:** Stop and ask if the update flow cannot treat the new choice as Close or Remind Me Later without a change to the delays or the install failure record.
- **Who finishes:** `ce-work` implements, opens one PR with a `feat` title, and runs `ce-babysit-pr` until the PR is ready to merge.
- **Product Contract preservation:** Restructured, no scope change: R5 names `site/index.html` in place of the domain longhand.md; the deferred question on the link's look and position is resolved by KTD3 and removed.

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

- R5. The download is the installer of the newest release, from the stable "latest release" download address that `README.md` and the website (`site/index.html`) also use. It is not the installer of the version that the window shows.
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

### Sources / Research

- `src/update/UpdateWindow.tsx`: `viewOf` defines the text and buttons of each update state.
- `src-tauri/src/managers/updates.rs` and `src-tauri/src/managers/updates/flow.rs`: the update flow, the `Failed` state, and the install attempt record that shows the failure at the next launch on Windows.
- Broadcom KB 419210, "Publisher blocks due to PARTIAL_CHAIN errors": https://knowledge.broadcom.com/external/article/419210/publisher-blocks-due-to-partial_chain-er.html

---

## Planning Contract

### Key Technical Decisions

- KTD1. **The click is a new `Choice` that the update flow handles.** `Choice::DownloadInstaller` in `src-tauri/src/managers/updates/flow.rs` gives the same state change as `Close` in `Failed` and as `RemindLater` in `Offered`, and it adds a new `Effect::OpenInstallerDownload`. The flow then decides the dismissal and the open together, and its unit tests prove both. The rejected alternative was an `openUrl` call in the window followed by a second `chooseUpdate` call: two calls can get out of step, and the window would need the download addresses in its opener allow list. Governs R1, R4.
- KTD2. **The app opens the address from Rust, with one address for each OS.** `src-tauri/src/managers/updates.rs` keeps the two download addresses beside `MACOS_MANIFEST_URL` and `WINDOWS_MANIFEST_URL`, chooses one with the same `windows: bool` pattern as `manifest_url`, and opens it with `tauri_plugin_opener::OpenerExt`, as `src-tauri/src/window.rs` does. The update window capability (`src-tauri/capabilities/update.json`) does not change, so the test that compares it with the changelog capability stays true. Governs R5, R6.
- KTD3. **The third action sits at the left of the button row.** In the failure window, "Download Installer" is a normal button at the left edge, apart from Close and Try Again, as a macOS alert places a third action (apple-design, Familiarity). In the offer window, "Download installer" is a text link in the same left position, styled as `WebLink` in `src/settings/About.tsx` (a `button` with the external-link icon). Return on the focused link chooses the download, as Return on a link in the list of changes opens that link. Otherwise the existing key rules of the window stay: Return chooses the default button, and Space activates the control that has the focus. If the offer row does not fit the 480px window on one line, the link moves to its own line at the left, below the list of changes, and the two buttons keep their row. Governs R2, R3.

### Assumptions

- The download addresses are `https://github.com/twistymaze/longhand/releases/latest/download/Longhand_x64-setup.exe` and `https://github.com/twistymaze/longhand/releases/latest/download/Longhand_aarch64.dmg`, as in `README.md` and `site/index.html`.
- An open error is logged and has no other effect. The window already closed, and the user can use the same choice again from the next notice.

### Sequencing

U1 adds the choice and regenerates `src/bindings.ts`. U2 uses the new choice in the window.

---

## Implementation Units

### U1. Update flow: the Download Installer choice

**Goal:** A `DownloadInstaller` choice that closes the failure window or the offer and opens the installer download of the running OS.

**Requirements:** R1, R4, R5, R6; KTD1, KTD2.

**Dependencies:** None.

**Files:**
- `src-tauri/src/managers/updates/flow.rs` (the `Choice` and `Effect` variants, the `choice` arms, unit tests)
- `src-tauri/src/managers/updates.rs` (address constants, the address function, the effect arm, unit test)
- `src/bindings.ts` (regenerated by `cargo test -p longhand export_bindings`)

**Approach:**
1. Add `Choice::DownloadInstaller`. In `Flow::choice`, it takes the same path as `RemindLater` from `Offered` and as `Close` from `Failed`, and it adds `Effect::OpenInstallerDownload`. In other states it does nothing, as an unknown choice does now.
2. Add the two address constants and a function that chooses one by `windows: bool`, next to `manifest_url`.
3. In the effect loop of `UpdateManager`, open the address with the opener plugin. Log an error with the `updates:` prefix and do nothing else.
4. Regenerate the bindings.

**Patterns to follow:** `manifest_url` and its test `each_os_reads_its_own_manifest_of_the_latest_release_of_the_source_repository`; the `Choice::RemindLater | Choice::Close` arm; `src-tauri/src/window.rs` for `app.opener().open_url`.

**Test scenarios:**
- Covers AE2. From `Offered`, `DownloadInstaller` gives `Idle`, sets `remind_until` as Remind Me Later does, and returns `SaveDelays` and `OpenInstallerDownload`.
- Covers AE1. From `Failed`, `DownloadInstaller` gives the same state as `Close` from `Failed` and returns `OpenInstallerDownload`.
- From `Ready`, `Downloading` and `Idle`, `DownloadInstaller` changes nothing and returns no effect.
- Covers AE4. The address function gives the `-setup.exe` address for Windows and the `.dmg` address for macOS, both under `releases/latest/download/` of `twistymaze/longhand`.

**Verification:** The new flow tests pass, and `src/bindings.ts` has `"download_installer"` in `Choice`.

### U2. Update window: the button and the link

**Goal:** The failure window shows a "Download Installer" button and the offer window shows a "Download installer" link, both of which send `download_installer`.

**Requirements:** R2, R3, R4; KTD3.

**Dependencies:** U1.

**Files:**
- `src/update/UpdateWindow.tsx`
- `src/update/UpdateWindow.test.tsx`

**Approach:**
1. Give `View` an optional left-side action, separate from `buttons`, so the default-button and focus rules do not change.
2. In `viewOf`, the `failed` view gets a button kind and the `offered` view gets a link kind, each with the `download_installer` choice.
3. Render the button row with the left action first and a flexible gap before the existing buttons. Render the link kind as `WebLink` renders, with the external-link icon and the screen-reader text.
4. In the `onKeyDown` handler, exempt the link from the Return-chooses-default rule, as the handler already exempts the list of changes.
5. Measure the offer row early. If it does not fit on one line, use the fallback in KTD3.

**Patterns to follow:** `WebLink` in `src/settings/About.tsx`; the existing button tests in `src/update/UpdateWindow.test.tsx`.

**Test scenarios:**
- The failure view shows "Download Installer", Close and Try Again, and Try Again stays the default button.
- A click on "Download Installer" sends `download_installer`.
- The offer view shows a "Download installer" link and the two buttons, and Install and Relaunch stays the default button.
- A click on the link sends `download_installer`.
- In a window that the user asked for, Return on the focused link sends `download_installer`, not `install`.
- In a failure window that the user asked for, Return still sends `try_again` when "Download Installer" has the focus.
- Up to date, check failed, ready and downloading views show no download action.

**Verification:** The window tests pass. In a release-flag test build, both windows show the action at the left, and no button is cut off at the right edge of the offer window.

---

## Verification Contract

| Check | When |
|---|---|
| `cargo test -p longhand updates` and `cargo test -p longhand export_bindings` | U1 |
| `npx vitest run src/update/UpdateWindow.test.tsx` | U2 |
| `npm run typecheck` and `npm test` | Before the push |
| `cargo fmt --all --check`, `cargo clippy` for the workspace, `scripts/check-windows.sh`, `node scripts/check-timing.ts` | Before the push |
| Manual test in a signed macOS test build with `LONGHAND_RELEASE=1` (`docs/solutions/workflow-issues/manual-test-of-the-updater-needs-a-release-flag-build.md`): Check for Updates… shows the offer with the link; a click opens the browser download of the `.dmg` and closes the window | Before the PR |

---

## Definition of Done

- R1 to R6 are true, and AE1 to AE4 have a test or a manual step.
- All checks in the Verification Contract pass.
- The PR title is a `feat` title; the user chooses patch or minor before the PR opens.
- No experimental code from abandoned attempts is left in the diff.
