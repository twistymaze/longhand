---
title: Windows code signing and in-app updates
description: "Sign the Windows installer of each release, and offer each newer release as an in-app update on Windows, as on macOS"
area: distribution
added: 2026-10-09
source: "a conversation on 2026-10-09 about the first Windows test build (archived PR 48)"
related: [preview-builds-and-updates]
plan: docs/plans/2026-10-28-2304-feat-windows-signed-releases-and-updates-plan.md
---

# Windows code signing and in-app updates

## Idea
Each release in `twistymaze/longhand` gets a signed Windows installer and the Windows update files. The Windows app offers each newer release in the update dialog, with the same rules as on macOS: never while a note records or processes.

## Why
Each release already has a Windows installer (`docs/releases.md`), but the installer is not signed, so SmartScreen shows a warning when a browser downloads it. The installed Windows app gets no updates, so each new release needs a manual install.

## Notes / Current state
- Shipped in [PR 79](https://github.com/twistymaze/longhand/pull/79) on 2026-10-29. v1.2.14 is the first release with the signed Windows installer and Windows updates: the v1.2.13 Windows build stopped at the signature gate, and [PR 80](https://github.com/twistymaze/longhand/pull/80) fixed the gate. The managed work PC test (R16) and the Windows download on longhand.md come after the first signed release.
- The plan builds this: `.github/workflows/release-windows.yml` signs the installer and every `.exe` and `.dll` it installs with Azure Artifact Signing (individual validation), checks the signatures on a silent install, and attaches the installer, its `.sig` and a separate Windows manifest `latest-windows.json`. The Windows build sets `LONGHAND_RELEASE`, so the Windows app gets updates with the macOS rules. `docs/releases.md` has the one-time setup.
- Decisions: the release still publishes after the macOS build, and the Windows workflow adds Windows later; the macOS `latest.json` never changes. The Windows app treats a missing Windows manifest or entry as "no update". x64 only. The release build stays on `windows-latest`.
- SmartScreen checks only files with the Mark of the Web, which browsers add. A file copied with `scp` has no mark. Test signing with a browser download.
- After the first signed release: test on a managed work PC (publisher name, install without admin rights over an older build, the next release as an in-app update). Only then turn on the Windows download on longhand.md.
- Immutable releases stay off, because the Windows files are attached after publication.
