---
title: Unified Release and Direct Downloads - Plan
type: feat
date: 2026-10-29
topic: unified-release-and-direct-downloads
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Unified Release and Direct Downloads - Plan

## Goal Capsule

- **Objective:** A visitor to longhand.md gets the newest Longhand installer for their computer with one click, on macOS and on Windows. Each release has the installers for both platforms from the moment it is published.
- **Product authority:** This Product Contract. The user chose the release shape, the file names and the button behaviour in the brainstorm on 2026-10-29.
- **Open blockers:** None.

---

## Product Contract

### Summary

Each release builds the macOS and Windows installers in parallel, at the same time as the checks on `main`. The release publishes once, with both platforms, or it does not publish. The installers get fixed file names, so a link to the latest release always gives a file. The landing page shows one direct download button for the visitor's operating system and a link to the other installer.

### Problem Frame

The landing page sends visitors to the GitHub releases page. That page lists nine files for each release, and a person who is not technical must find the correct one. The Windows button shows "Coming soon", although signed Windows releases exist since v1.2.14.

A direct link is not possible today, for three reasons:

- The file names contain the version (`Longhand_1.2.14_aarch64.dmg`, `Longhand_1.2.14_x64-setup.exe`).
- The Windows installer comes in a separate workflow after the release is published. For v1.2.14, the macOS files were published at 14:37 and the Windows installer at 14:53.
- Some releases never get a Windows installer. GitHub replaces a waiting Windows run, or the Windows build or signing fails, and the release keeps only its macOS files (`docs/releases.md`, "The Windows installer is missing").

The cause is that Windows is a second-class platform in the release process. The user wants macOS and Windows to be almost equally important.

Measured times today, from the push to `main`: the checks take about 11–13 min. The macOS release build and publication take about 4 min, so macOS is available after about 17 min. The Windows build takes 9–18 min and starts only after the CI run ends, so Windows is available after about 27–35 min.

### Key Decisions

- **One release for both platforms.** A failure of the Windows build fails the release, the same as a failure of the macOS build. A failed Windows build is a defect in CI, and the fix goes in a `fix:` PR. Governs R1, R3, R4, R5. (session-settled: user-directed — chosen over a separate Windows workflow that attaches its installer after publication: Windows and macOS must be almost equally important, and a release with no Windows installer is a defect to fix.)
- **Release builds start alongside the checks.** The builds do not wait for the checks. Publication waits for the checks and both builds. Code on `main` already passed the checks on its PR, and a failed check still publishes nothing; the only cost is a signing and notarization run that is not used. Governs R1, R2. (session-settled: user-approved — chosen over release builds that start after the checks pass: both platforms come out about 15–20 min after the push, compared with 25–30 min.)
- **Fixed installer names.** When both platforms publish together, `releases/latest/download/<name>` always gives a file, so the site needs no step that finds file addresses. Governs R6, R9. (session-settled: user-approved — chosen over versioned names with a site build that finds the newest release that has each file: no blocker remains after the release is unified.)
- **One button for the detected operating system, and a link to the other installer.** Governs R9–R12. (session-settled: user-directed — chosen over two equal buttons: most visitors see one clear action, and the other platform stays one click away.)

### Requirements

**Release**

- R1. When a push to `main` makes a release due, the macOS and Windows release builds start in parallel with the checks of that CI run.
- R2. The release publishes only after every check and both release builds pass. All files go up to a draft first, and the draft becomes the latest release only after the last file is uploaded.
- R3. When a check, the macOS build or the Windows build fails, nothing publishes and no tag is made. The next run that makes the release due publishes it with both platforms.
- R4. When GitHub replaces a waiting CI run on `main`, the commits of that run go out in the next release, with both platforms.
- R5. The separate Windows release workflow (`.github/workflows/release-windows.yml`) and its rebuild of the installer for one tag are removed.

**Release files**

- R6. The installers have fixed names that contain the platform and the architecture but no version: `Longhand_aarch64.dmg` and `Longhand_x64-setup.exe`.
- R7. Each release continues to publish the update manifests that installed apps read: `latest.json` for macOS and `latest-windows.json` for Windows. Installed apps on both platforms continue to find updates with no change to the app.
- R8. Releases that were published before this change keep their files and names.

**Landing page**

- R9. The page detects the operating system of the visitor. On macOS or Windows, it shows one main button for that system ("Download for Mac" or "Download for Windows"), which downloads the installer directly from `https://github.com/twistymaze/longhand/releases/latest/download/<name>`.
- R10. Below the main button, a link downloads the installer for the other platform.
- R11. When the page cannot find macOS or Windows (a phone, Linux, an unknown system, or no JavaScript), it shows both buttons as equals.
- R12. Both download places on the page use R9–R11: the top of the page and the "Try Longhand in your next meeting" section. The "Coming soon" label goes away.
- R13. The platform line gives the system requirements for both platforms, as the release builds support them. For example: macOS 15 or later on Apple silicon; the Windows versions that the installer supports.

**Documentation**

- R14. `README.md` and `docs/releases.md` describe the single release for both platforms, the fixed file names and the failure rules of R3.

### Acceptance Examples

- AE1. **Covers R9, R10.** **Given** a visitor on a Mac, **when** the page loads, **then** the main button says "Download for Mac" and downloads `Longhand_aarch64.dmg` of the latest release, and a link below it downloads `Longhand_x64-setup.exe`.
- AE2. **Covers R9, R10.** **Given** a visitor on Windows, **when** the page loads, **then** the main button says "Download for Windows" and downloads `Longhand_x64-setup.exe`, and a link below it downloads the DMG.
- AE3. **Covers R11.** **Given** a visitor on an iPhone, on Linux, or with JavaScript off, **when** the page loads, **then** "Download for Mac" and "Download for Windows" show as two equal buttons.
- AE4. **Covers R2, R3.** **Given** a due release whose macOS build passes and whose Windows build fails, **when** the run ends, **then** no release and no tag exist for that version, and the latest release and its download links stay on the previous version.
- AE5. **Covers R2.** **Given** a release that is publishing, **when** a visitor clicks a download button before the last file is uploaded, **then** the visitor gets the installer of the previous release, not a missing file.

### Success Criteria

- For a due release, both installers are available about 15–20 min after the push to `main`. macOS is not available much later than today (about 17 min).
- Every published release has both installers.

### Scope Boundaries

- One `latest.json` with both platforms in place of `latest-windows.json` is deferred. Installed Windows apps read `latest-windows.json`, so a change needs a migration.
- An Intel macOS build, an ARM64 Windows build and a Linux build are not in scope. R6's names leave space for them.
- Visitors on an Intel Mac also see "Download for Mac". A browser cannot reliably tell Apple silicon from Intel. R13's platform line tells them the requirement.
- Versioned copies of the installers next to the fixed names are not made.
- A change to the release title, the release notes or the changelog page is not in scope.

### Dependencies / Assumptions

- The CI workflow on `main` runs one run at a time (`concurrency: ci-${{ github.ref }}` in `.github/workflows/ci.yml`). With release builds that start alongside the checks, the length of a run on `main` changes little, so the queue of runs on `main` does not get much longer. The separate Windows workflow existed to prevent a long queue (KTD2 of `docs/plans/2026-10-28-2304-feat-windows-signed-releases-and-updates-plan.md`).
- The Windows release build continues to run in the `release` environment, which allows deployments only from `main`. The Azure federated credential trusts only this environment.
- The website deploys only with a release or a change to the site (`changelog-pages` in `.github/workflows/ci.yml`). With fixed names, the buttons are static links, so the deploy needs no information about the release files.

### Outstanding Questions

**Deferred to Planning**

- Which Windows versions the installer supports, for R13.
- How the run finds that a release is due before the checks end, so that runs with no due release start no release build.
- Whether the release jobs stay in `.github/workflows/ci.yml` or move to their own workflow, with the trust and concurrency rules of `docs/releases.md`.
- How the macOS updater archive and the Windows `.sig` names change, if they change, with R7.
- Which method the page uses to detect the operating system.

### Sources / Research

- `site/index.html` — the two download places and the "Coming soon" label.
- `.github/workflows/ci.yml` — `release-build` (needs `checks` and `deny`), `release-publish`, `changelog-pages`.
- `.github/workflows/release-windows.yml` — the separate Windows build, its concurrency group and its `attach` job.
- `scripts/publish-release.ts` — `publish` and `attach`; its tests use the versioned names.
- `src-tauri/src/managers/updates.rs` — the apps read `releases/latest/download/latest.json` and `latest-windows.json`.
- `docs/releases.md` — the release flow and the rules for a missing Windows installer.
- Release v1.2.14 and the CI runs of 2026-10-28 and 2026-10-29 — the measured times in the Problem Frame.
