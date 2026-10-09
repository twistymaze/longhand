---
title: Windows preview builds and in-app updates
description: "A signed Windows installer from each push to main, offered as an in-app update, as the macOS preview build is"
area: distribution
added: 2026-10-09
source: "a conversation on 2026-10-09 about the first Windows test build (PR #48)"
related: [preview-builds-and-updates]
---

# Windows preview builds and in-app updates

## Idea
Each push to main that passes CI publishes a signed Windows installer and update files to the builds repository, beside the macOS preview build. The Windows app offers each newer build in the update dialog, with the same rules as on macOS: never while a note records or processes.

## Why
Today a Windows build comes only from a manual CI run (PR #48). The installer is not signed, so SmartScreen shows a warning when a browser downloads it, and the artifact is deleted after 7 days. The installed app gets no updates, so each test needs a new manual install.

## Notes / Current state
- PR #48 adds the manual `windows_installer` input to `ci.yml`. It builds a release NSIS installer and uploads it as a workflow artifact.
- SmartScreen checks only files with the Mark of the Web, which browsers add. A file copied with `scp` has no mark, so the first test on 2026-10-09 showed no warning. Test signing with a browser download.
- Needs a Windows code-signing certificate or a signing service (for example Azure Trusted Signing), and a secret for the `preview` environment.
- The `preview` job runs on macOS only. A Windows job, or a second job, must build, sign and publish the installer. The publish script (`scripts/publish-preview.ts`) and the update manifest must include the Windows platform.
- The Tauri updater needs the Windows update files (the NSIS installer and its signature) in the manifest.
- `done/preview-builds-and-updates.md` puts Windows updates in a later stage.
