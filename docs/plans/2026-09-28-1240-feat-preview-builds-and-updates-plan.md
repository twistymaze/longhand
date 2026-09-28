---
title: Preview Builds and In-App Updates - Plan
type: feat
date: 2026-09-28
topic: preview-builds-and-updates
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Preview Builds and In-App Updates - Plan

## Goal Capsule

- **Objective:** A Longhand user on an Apple silicon Mac always runs a build close to main. Each change that gets into main reaches them as an update offer, and they accept it with one click at a time they choose.
- **Product authority:** This Product Contract. Only the preview track is in scope. The stable track, release tags and a track picker are later work, and this plan must not prevent them (R8).
- **Open blockers:** None. The version form must pass the spike (KD5) before CI or updater work continues.

---

## Product Contract

### Summary

Each push to main that is not docs-only and passes CI publishes a signed, notarized preview build to a public repository that holds only builds. The app finds newer preview builds and asks whether to install and relaunch, in a standard macOS dialog. It asks only while no note is recording or processing. A setting in General turns automatic checks off.

### Problem Frame

Today a new build gets to a Mac only by hand: a local signed build, notarization, a zip, and an `scp` copy from the build Mac. The user runs Longhand every day, and a few peers may start to use it. Each of them must know when main changed and repeat those steps. Builds fall behind, and a fix on main does not reach the people who found the defect.

`STRATEGY.md` defers distribution machinery "until someone asks". The user and those peers now ask for it.

The reference apps each miss part of the expected macOS behaviour. Handy shows only a small "Update available" text in its footer, which is easy to miss for a long time. Anarlog downloads, installs and relaunches without asking. The expected behaviour is the Sparkle pattern: a dialog that asks, and a "later" choice that asks again.

---

### Key Decisions

- KD1. **Builds go to a public repository that holds only builds.** The updater must download without a login, and a private repository's releases cannot serve that. (session-settled: user-directed — chosen over making the source repository public or a private cloud bucket: the source stays private, and GitHub stays the only host. macOS CI minutes stay billed.) Governs R3, R5.
- KD2. **A preview version is a semver pre-release of the next stable version, with a UTC timestamp.** Semver has no numeric position after patch, so the pre-release tag holds the build time. A later stable release then sorts correctly with no change to the scheme. (session-settled: user-directed — chosen over a plain date-time version, `0.1.<build number>`, and the track name "beta": it fits later semver releases. The user chose the name "preview".) Governs R6.
- KD3. **The track is a setting of one app, not a separate app for each track.** A user can move between tracks with no reinstall. (session-settled: user-directed — chosen over anarlog's separate app for each track: switching then means installing another app.) Governs R8.
- KD4. **The app asks before it downloads.** Nothing downloads without consent, and "Remind Me Later" waits 24 hours. (session-settled: user-directed — chosen over a background download followed by a question, and over a third "Install When I Quit" button. The 24-hour delay was chosen over "until next launch" and 4 hours.) Governs R13, R15, R16.
- KD5. **The first unit proves the version form on real macOS.** Tauri writes the version into both bundle version fields (`tauri-codegen-2.6.3/src/context.rs:324`). A spike must show that notarization, Gatekeeper, TCC and the updater accept a pre-release version before other work continues. Governs R6.
- KD6. **The update flow never interrupts a recording.** The recording surface must not draw attention back to the screen (U7's attention budget in `CLAUDE.md`). (session-settled: user-directed — chosen over a relaunch with no second question after the recording, or an install at the next quit with no second question.) Governs R14, R17.
- KD7. **Automatic checks can be turned off in General, and a manual check always works.** (session-settled: user-directed — chosen over automatic checks with no setting: the user asked for the setting in General.) Governs R10, R11.

---

### Requirements

**Preview builds in CI**

- R1. Each push to main that runs CI and passes all checks makes a preview build of the macOS app. The build is signed with the Developer ID certificate, notarized and stapled. Docs-only pushes already run no CI (`.github/workflows/ci.yml`, `push.paths`).
- R2. A preview build runs on Apple silicon only.
- R3. CI publishes each preview build to the public repository that holds only builds. The publication contains a DMG for the first install, the update archive with its signature, and an update manifest that identifies the newest preview build.
- R4. A failed build, signature or notarization step publishes nothing, and the manifest continues to identify the previous preview build. CI also publishes nothing when the build's commit is no longer the tip of main at publish time, or when its version is not higher than the version the manifest identifies. Thus a late or re-run build of an older commit cannot replace a newer build.
- R5. The source repository stays private. The public repository holds only built files.
- R19. Only the preview build job for pushes to main can read the signing certificate, the notarization key, the updater private key and the publish credential. Pull request runs get none of them.

**Version and track**

- R6. A preview build's version is `<next stable version>-preview.<UTC build time as YYYYMMDDHHMMSS>`, for example `0.2.0-preview.20260928143012`. A later build always has a higher version.
- R7. Settings → About shows the full version and the short commit of the installed build.
- R8. Each build records its track, which is `preview` for all builds in this plan. The updater offers only builds of the track that is set, and the stored track value is a setting of the same app. Thus a later plan can add a stable track and a track picker with no new app identity and no reinstall.

**Update check**

- R9. When automatic checks are on, the app checks for a newer build at launch and at regular intervals while it runs.
- R10. Settings → General has a setting to check for updates automatically. It is on by default. When it is off, the app makes no automatic check.
- R11. "Check for Updates…" in the app menu checks at once. It ignores R10 and any remind delay from R15. It shows its result at once, also while a note is recording or processing, because the user asked for it. If the user then clicks "Install and Relaunch", the download starts, and R17 holds the relaunch until recording and processing end. If no newer build exists, it says the app is up to date. If the check fails, it shows the error.
- R12. An automatic check that fails, for example with no network, shows nothing to the user.

**Update dialog and install**

- R13. When a newer build exists, the app shows a standard macOS dialog: "Longhand <new version> is available. You have <installed version>." It has two buttons: "Install and Relaunch" and "Remind Me Later".
- R14. While a note is recording or processing, the app does not show the dialog, start a download, install or relaunch. A download that started before the recording can finish. If an offer is due during that time, the app shows it when recording and processing both end.
- R15. After "Remind Me Later", no automatic dialog shows for 24 hours, even when newer builds are published. After 24 hours, the next check offers the newest build.
- R16. "Install and Relaunch" downloads the build with visible progress, checks its signature, installs it and relaunches Longhand. The build does not download before this click. A check reads only the update manifest.
- R17. If a recording starts before the download ends, the app does not relaunch. When recording and processing both end, it asks: "Longhand <new version> is ready. [Relaunch Now] [Later]". "Later" installs the build when the user next quits Longhand. Closing the window does not quit Longhand, so if it has not quit 24 hours after "Later", the next check asks the ready question again, or offers a newer build if one exists.
- R18. If a download or signature check fails, the app shows the error with a choice to try again. The installed app does not change.

---

### Key Flows

- F1. Offer and install
  - **Trigger:** An automatic check at launch or at an interval (R9) finds a newer preview build.
  - **Steps:** The app waits until no note is recording or processing (R14). It shows the dialog (R13). The user clicks "Install and Relaunch". The app downloads with progress, checks the signature and relaunches (R16).
  - **Outcome:** The newer build runs.
  - **Covers:** R9, R13, R14, R16
- F2. Remind later
  - **Trigger:** The user clicks "Remind Me Later" in the dialog.
  - **Steps:** Automatic checks continue but show no dialog for 24 hours (R15). After that time, the next check offers the newest build.
  - **Outcome:** The user sees one offer per day at most, unless they ask for a check (R11).
  - **Covers:** R11, R15
- F3. Recording starts during a download
  - **Trigger:** The user clicked "Install and Relaunch", then started a recording before the download ended.
  - **Steps:** The download finishes and the app does not relaunch. When recording and processing end, the app asks "Relaunch Now" or "Later" (R17).
  - **Outcome:** The recording is not interrupted. The update installs at a relaunch the user chose, or at the next quit.
  - **Covers:** R14, R17

---

### Acceptance Examples

- AE1. **Covers R14.** Given the launch check found a newer build, when the user starts a recording before the dialog shows, then no dialog shows until recording and processing end.
- AE2. **Covers R14.** Given the dialog is open, when the user starts a recording with the shortcut, then the dialog closes. The dialog shows again when recording and processing end.
- AE3. **Covers R15.** Given the user clicked "Remind Me Later" at 09:00 and three newer builds are published before 17:00, then no automatic dialog shows until 09:00 the next day. At that time the dialog offers the newest of the three builds.
- AE4. **Covers R11, R15.** Given the remind delay is active, when the user chooses "Check for Updates…", then the dialog shows at once.
- AE8. **Covers R11, R17.** Given a note is recording, when the user chooses "Check for Updates…" and clicks "Install and Relaunch", then the download starts, and the app does not relaunch until recording and processing end.
- AE5. **Covers R10, R11.** Given automatic checks are off, when Longhand starts, then it makes no network request for updates. When the user chooses "Check for Updates…", then the app checks.
- AE6. **Covers R4.** Given the notarization of a new build fails, then the manifest still identifies the previous build, and no user receives an offer for the failed build.
- AE7. **Covers R17.** Given the "ready" question shows after a recording and the user clicks "Later", when the user quits Longhand, then the new build is installed, and the next launch runs it.

---

### Scope Boundaries

- A stable track, release tags, manually set version numbers and a track picker are deferred. R8 keeps the path open for them.
- Windows builds and Windows updates are deferred.
- Intel Macs are out of scope (R2).
- The dialog shows no list of changes and no release notes. After an update, the app shows no "What's New" screen.
- The dialog has no "Skip This Version" choice. On a track that publishes at each push, a newer build supersedes the skipped one soon after.
- The update flow sends no email, push notification or macOS notification.

---

### Dependencies / Assumptions

- The Developer ID certificate and the App Store Connect API key that local notarized builds use (see `docs/development-workflow.md`) can be stored as CI secrets.
- The user creates the public repository that holds only builds. CI gets a credential that can write only to that repository's contents and releases (a fine-grained token or a GitHub App limited to that repository), stored as a CI secret.
- The updater's signing key pair is new. Its private key is a CI secret and must not be lost. Without it, installed builds cannot accept any later update. If the private key leaks, make a new key pair, publish a build with the new public key as a DMG, and tell each user to reinstall from it by hand. Installed builds cannot move to a new key through an update.
- Builds installed before this work have no updater. Each user installs the first preview build from its DMG by hand.
- Each preview build adds a signed and notarized macOS build to the billed CI minutes on each push to main.
- Anyone who has the public repository's link can download a build. A build contains no user data.

---

### Outstanding Questions

**Deferred to Planning**

- If the spike (KD5) shows that macOS or notarization rejects a pre-release version in the bundle's short version field, use the fallback: set the bundle build number to the timestamp and the short version to the plain `X.Y.Z`. The build must still embed the full R6 version, the updater must compare against that value (not the plain config version), and Settings → About (R7) shows it. Otherwise every newer preview build sorts below the installed `X.Y.Z` and no update is offered.
- Build the dialog as a small app window that looks and behaves like a standard macOS alert, with progress for R16. The dialog plugin's native alert blocks the main thread in a modal loop: code cannot close it and it cannot show progress, so it cannot meet AE2 or R16. What does the window look like in detail?
- How often does R9 check while the app runs?
- How many old preview builds does the public repository keep?
- Is the preview build a separate CI job after the checks, or a separate workflow that runs when CI passes? It must meet R1 and R4.
- How does the "Later" choice in R17 install at quit? Tauri's updater may need extra work for that.
- Where does the next stable version for R6 come from (for example, the version in `src-tauri/tauri.conf.json`), and who increases it after a stable release?

---

### Sources / Research

- `.github/workflows/ci.yml`: the triggers, the docs-only `paths` filter, and the ad-hoc signed debug build with no signing secrets.
- `src-tauri/tauri.conf.json`: version `0.1.0`, identifier `net.ewal.longhand`, the `dmg` target, `hardenedRuntime`, `minimumSystemVersion` 15.0 and the Developer ID `signingIdentity`. No updater plugin exists in `src-tauri/Cargo.toml`.
- `src/settings/SettingsWindow.tsx`: the General pane (`id: "status"`) and the About pane.
- `src-tauri/src/managers/capture.rs` (`is_capturing`, `RecordingStatus`) and `src-tauri/src/managers/processing.rs` (`ProcessingUpdate`): the recording and processing state that R14 depends on. These are two separate states, not one.
- Handy (`reference-source/Handy`): `tauri-plugin-updater` with a `latest.json` on GitHub Releases, `createUpdaterArtifacts`, and signing and notarization through `tauri-apps/tauri-action` in `.github/workflows/build.yml`. The only sign of an update is a small footer text (`src/components/update-checker/UpdateChecker.tsx`).
- Anarlog (`reference-source/anarlog`): checks at launch and every 30 minutes, holds updates while a meeting records (`plugins/updater2/src/lib.rs`), and uses the version form `X.Y.Z-nightly.N`. It installs and relaunches without asking, which this plan does not copy.
