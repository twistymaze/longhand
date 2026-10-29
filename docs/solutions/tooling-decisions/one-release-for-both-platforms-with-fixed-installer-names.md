---
title: "One release for both platforms with fixed installer names, so the site can link to the latest installer"
date: 2026-10-29
category: tooling-decisions
module: "Release flow (.github/workflows/ci.yml release-version, release-build, release-windows, release-publish, changelog-pages; scripts/publish-release.ts; site/public/platform.js)"
problem_type: tooling_decision
component: development_workflow
severity: medium
applies_when:
  - "A change touches the release jobs in ci.yml or scripts/publish-release.ts"
  - "A change renames a release file or adds a platform"
  - "A change edits the condition of the changelog-pages (website) job"
  - "A change adds a link to a file under releases/latest/download/"
related_components:
  - ci
tags: [release, github-actions, tauri-updater, windows, macos, download-links, website]
---

# One release for both platforms with fixed installer names, so the site can link to the latest installer

## Context

Until PR #81, the macOS release published first, and a second workflow (`release-windows.yml`) attached the Windows installer later. The file names contained the version (`Longhand_1.2.15_aarch64.dmg`). So the website could not link to an installer directly: the name changed with each release, and for some time after each release the latest release had no Windows file. The site linked to the releases page, and visitors had to find the correct file.

PR #81 builds macOS and Windows in one CI run, publishes one release with both platforms, and gives the installers fixed names: `Longhand_aarch64.dmg` and `Longhand_x64-setup.exe`. The site and the README link to `https://github.com/twistymaze/longhand/releases/latest/download/<name>`.

## Guidance

**`releases/latest/download/<name>` works only when every published release has every file.** GitHub resolves `latest` to the newest published release that is not a draft or a prerelease. If a release publishes before one of its files is attached, the link returns 404 for that time. So:

- `release-publish` needs every check and both builds (`needs: [checks, deny, release-version, release-build, release-windows]`). A failed Windows build fails the release, the same as a failed macOS build.
- `scripts/publish-release.ts` uploads all files to a **draft** first, then reads `latest.json` and `latest-windows.json` back from the draft and compares them with what it uploaded, and only then publishes. A draft asset is read through the authenticated API (`GET /repos/{repo}/releases/assets/{id}` with `Accept: application/octet-stream`); the public download URL does not serve draft files. When the read-back fails, the release stays a draft and nothing publishes.
- The fixed names are exported constants (`DMG_NAME`, `INSTALLER_NAME`). `publish()` refuses any other name before the first API call, and `scripts/site.test.ts` builds the site links from the same constants.

**Renaming the files does not break updates.** The Tauri updater (tauri-plugin-updater 2.14) checks the minisign signature and the `version` field of the manifest; it does not use the file name. The manifests point at the new names, and installed apps update as before. Older releases keep their versioned names.

**A skipped need skips the website deploy.** `changelog-pages` deploys the site after a release, and also for a site-only change when no release is due. In the second case `release-publish` is skipped, and GitHub skips every job that needs a skipped job unless its condition starts with `!cancelled()`. The condition therefore starts with `!cancelled()` and then checks the needs explicitly: `checks`, `deny` and `release-version` must have the result `success`, and `due` must not be `'true'`. Without the explicit `release-version` success check, a failed version job gives an empty `due`, which reads as "no release due" and deploys the site without the release.

**The first PR with fixed names must make a release.** The site deploys only with a release or a site-only change. If the PR that adds the direct links had a non-release type (`docs`, `ci`, `chore`), the site would deploy with links to files that no release has yet. PR #81 used `build:` (a patch release), so the first fixed-name release and the new page deploy in the same run. The README has no such guard: GitHub shows it at merge, so its direct links return 404 until the merge run publishes (about 30 minutes). That window was accepted.

**Two link groups per download place keep the keyboard order.** The page shows one main button for the detected system and a link to the other installer. Reordering one group with CSS `order` would make the tab order differ from the visual order. Each download place has a Mac-first and a Windows-first group, and CSS hides the group that does not apply (`display: none` removes it from the tab order). A blocking head script (`site/public/platform.js`) sets `data-platform` before first paint, so the layout does not change after load. iPadOS Safari reports a Mac user agent, so the script detects a Mac only when `navigator.maxTouchPoints <= 1`.

## When this applies

Use this when you add a platform, a file, or a new link to a release file: the file must be built in the same CI run, uploaded to the draft, and listed in `publish()`, or `latest/download` links break. The older learnings that cite `release-windows.yml` (for example `docs/solutions/security-issues/dispatched-release-build-could-write-main-caches.md`) describe the workflow before PR #81; their rules still apply to any new dispatch or rebuild path.
