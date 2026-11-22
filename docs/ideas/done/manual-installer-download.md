---
title: Download the installer from the update window
description: "The update window can open the newest installer in the browser, for a user whose automatic update cannot install"
area: distribution
added: 2026-11-21
source: "Carbon Black blocked the Windows update installer of 1.2.29 (conversation, 2026-11-18)"
related: [windows-preview-builds-and-updates]
plan: docs/plans/2026-11-21-2134-feat-manual-installer-download-plan.md
---

# Download the installer from the update window

## Idea
The "The update could not be installed" window gets a "Download Installer" button, and the "A new version is available" window gets a small "Download installer" link. Both open the newest release's installer in the default browser, on Windows and macOS.

## Why
Security software on a company PC (Carbon Black App Control) blocked the installer that the automatic update started, although the installer was signed. Try Again fails in the same way, and the user had to find the download by hand.

## Notes / Current state
Shipped in PR #111. The offer's link has its own line, because the link and both buttons did not fit in the 480px window. Lesson: `docs/solutions/integration-issues/carbon-black-blocked-the-signed-update-installer.md`.
