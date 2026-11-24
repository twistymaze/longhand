---
title: An update download with no network after a wake failed at once and showed an error
date: 2026-11-24
category: logic-errors
module: Update manager download (src-tauri/src/managers/updates.rs)
problem_type: logic_error
component: tauri_backend
symptoms:
  - "The update could not be installed: Error sending request for url (https://github.com/twistymaze/longhand/releases/download/v1.2.35/Longhand.app.tar.gz)"
  - "Try Again failed in the same second each time; the same download worked 4 minutes later"
root_cause: missing_validation
resolution_type: code_fix
severity: medium
tags: [updater, download, network, wake, sleep, retry, is_connect, reqwest, timeout]
related_components: [updater]
---

# An update download with no network after a wake failed at once and showed an error

## Problem

The user opened the laptop lid and clicked Install and Relaunch before Wi-Fi connected. The update download made one request. The request got no connection, and the window showed "The update could not be installed". The problem corrected itself some seconds later, so the app showed an error for a normal state.

## Symptoms

- The app log (UTC) on the laptop showed, in the same second: `the check failed: error sending request for url (.../latest.json)`, then `the download failed: error sending request for url (.../Longhand.app.tar.gz)`.
- Each Try Again failed in the same second. At 17:56, the same download worked and installed.
- The log gave only reqwest's top line, so it did not show why the request failed.

## What Didn't Work

- **"The download has the 5-second limit of the quick check."** The Install click runs a check with a 5-second limit (`QUICK_CHECK_TIMEOUT`), and `Update::download` applies `self.timeout` to its request. But tauri-plugin-updater builds each `Update` with `timeout: None` (`update_from_release` in `tauri-plugin-updater-2.14.1/src/updater.rs`, also in 2.12.0 and 2.14.0). A timeout set on `UpdaterBuilder` limits only the check. The download has no total limit.
- **"The Mac uses a proxy that Longhand ignores."** The failures came in the same second as the request, which fits a refused connection. But the laptop had no proxy. It had no network yet after the wake.
- **The first log came from the wrong Mac.** The log on the desktop Mac had no failure. Ask which Mac showed the error before you read a log.

## Solution

- `UpdateManager::download` runs each try through `retry_not_reached` with `DOWNLOAD_RETRY_WAITS` (2, 5, 10, 20, 30 and 60 seconds). It tries again only when `no_connection` is true: `tauri_plugin_updater::Error::Reqwest(e)` with `e.is_connect()`. The window keeps "Downloading Longhand <version>…" during the waits. The error shows only after the last try fails.
- Each failed try and each failed check logs the error with all its causes (`with_causes`).

## Why This Works

- A request with no connection did not reach the server, so a new try is safe. A download is a GET, so the summary client's reason to retry only `is_connect()` (no double charge) does not apply here. The download keeps that rule all the same, so a bad signature or a 404 shows at once.
- On macOS, a tokio timer does not advance while the Mac sleeps. A wait that starts in a short wake continues after the next full wake.

## Prevention

- Each network request whose failure shows a message must try again after a connection failure, as the summary client does (`crates/summarize/src/client.rs`, [the summary lesson](summary-in-a-wake-with-no-network-kept-the-default-budget.md)). This is the second case of this mistake.
- Tests in `src-tauri/src/managers/updates.rs`: `a_refused_connection_counts_as_no_connection_and_its_log_names_the_cause` uses a closed local port, and three tests check the retry rule with tokio's paused time.
- For a "could not be installed" report, collect the log of the Mac that showed the error, and `pmset -g log | grep -E '(Sleep|Wake|DarkWake)'` for that day.
