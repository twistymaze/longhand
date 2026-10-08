---
title: A separate identifier for test builds
description: "Manual test builds use their own bundle identifier, so macOS never opens a test copy in place of the installed app"
area: distribution
added: 2026-10-05
source: "a conversation on 2026-10-05: a startup crash on a work computer after a macOS update"
related: [old-build-stops-with-a-message, preview-builds-and-updates]
---

# A separate identifier for test builds

## Idea
A build for a manual test uses a different bundle identifier, for example `net.ewal.longhand.test`. The installed build in `/Applications` keeps `net.ewal.longhand`.

## Why
All builds use the identifier `net.ewal.longhand`. The login item (`SMAppService.mainAppService`) records the identifier, not a path. At login, macOS finds an app with that identifier. On 2026-10-05, after a macOS update, macOS opened an old test copy in `~/Downloads` instead of the installed build. The old build read data from a newer build and stopped with `abort()` at startup. The user removed the test copy, and the installed build started correctly. Spotlight and Launchpad can make the same choice.

The manual test steps tell you to remove the test copy after the test. A missed removal must not break the installed app.

## Notes / Current state
- A different identifier also gives the test build its own microphone permission, Keychain items, app data folder, log folder and settings. This can be an advantage: a test cannot change the settings or the database of the installed build. It can also be a disadvantage: each test build asks for the microphone again, and it has no API keys until you enter them.
- The storage folder is a user setting. Decide if a test build uses the same library or an empty one.
- The instance lock and the update channel must not let a test build and the installed build interfere.
- One possible method: a Tauri config overlay (`--config`) that the manual test build command applies.
