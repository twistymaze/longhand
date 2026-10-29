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
- `docs/plans/2026-10-11-1323-ci-releases-in-source-repository-plan.md` moved builds to GitHub Releases in `twistymaze/longhand`. `.github/workflows/release-windows.yml` builds the unsigned NSIS installer on a standard GitHub-hosted runner after CI succeeds on `main`, and attaches it to the release. The macOS build alone gates a release, and the Windows installer is attached later (R3, R26). This keeps the slow Windows build (45 minutes or more) from delaying macOS updates. The Windows build does not set `LONGHAND_RELEASE`, so it gets no updates.
- SmartScreen checks only files with the Mark of the Web, which browsers add. A file copied with `scp` has no mark, so the first test on 2026-10-09 showed no warning. Test signing with a browser download.
- Needs a Windows code-signing certificate or a signing service (for example Azure Trusted Signing), and a secret in the `release` environment.
- The `build` job of `release-windows.yml` must sign the installer, and `scripts/publish-release.ts` and the update manifest must include the Windows platform.
- The Tauri updater needs the Windows update files (the NSIS installer and its signature) in the manifest.
- `done/preview-builds-and-updates.md` puts Windows updates in a later stage.
- When Windows gets signing and in-app updates, decide again:
  - **Publish order.** `latest.json` must name the Windows update files when the release publishes, or Windows apps see a release with no Windows update. Either the release waits for both builds, or the manifest gets the Windows entry when the installer is attached.
  - **Runner.** A standard runner is free but slow. The larger `windows-8core` runner builds about twice as fast, with billed minutes. A Rust cache from main runs also shortens the build.
  - **Immutable releases.** These are off because the installer is attached after publication. If the release waits for both builds, immutable releases become possible.
