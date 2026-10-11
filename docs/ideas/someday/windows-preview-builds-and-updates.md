---
title: Windows preview builds and in-app updates
description: "A signed Windows installer from each push to main, offered as an in-app update, as the macOS preview build is"
area: distribution
added: 2026-10-09
source: "a conversation on 2026-10-09 about the first Windows test build (archived PR 48)"
related: [preview-builds-and-updates]
---

# Windows preview builds and in-app updates

## Idea
Each push to main that passes CI publishes a signed Windows installer and update files to the builds repository, beside the macOS preview build. The Windows app offers each newer build in the update dialog, with the same rules as on macOS: never while a note records or processes.

## Why
Today a Windows build comes only from a manual CI run (archived PR 48). The installer is not signed, so SmartScreen shows a warning when a browser downloads it, and the artifact is deleted after 7 days. The installed app gets no updates, so each test needs a new manual install.

## Notes / Current state
- Archived PR 48 adds the manual `windows_installer` input to `ci.yml`. It builds a release NSIS installer and uploads it as a workflow artifact.
- SmartScreen checks only files with the Mark of the Web, which browsers add. A file copied with `scp` has no mark, so the first test on 2026-10-09 showed no warning. Test signing with a browser download.
- Needs a Windows code-signing certificate or a signing service (for example Azure Trusted Signing), and a secret for the `preview` environment.
- The `preview` job runs on macOS only. A Windows job, or a second job, must build, sign and publish the installer. The publish script (`scripts/publish-preview.ts`) and the update manifest must include the Windows platform.
- The Tauri updater needs the Windows update files (the NSIS installer and its signature) in the manifest.
- `done/preview-builds-and-updates.md` puts Windows updates in a later stage.
- `docs/plans/2026-10-11-1323-ci-releases-in-source-repository-plan.md` moves builds to GitHub Releases in `twistymaze/longhand`. Each release attaches the unsigned Windows installer, built on a standard GitHub-hosted runner. The macOS build alone gates a release, and the Windows installer is attached later (R3, R26). This keeps the slow Windows build (45 minutes or more) from delaying macOS updates.
- When Windows gets signing and in-app updates, decide again:
  - **Publish order.** `latest.json` must name the Windows update files when the release publishes, or Windows apps see a release with no Windows update. Either the release waits for both builds again, or the manifest gets the Windows entry when the installer is attached.
  - **Runner.** A standard runner is free but slow. The larger `windows-8core` runner builds about twice as fast, with billed minutes. A Rust cache from main runs also shortens the build.
  - **Immutable releases.** These are off because the installer is attached after publication. If the release waits for both builds again, immutable releases become possible.
