---
title: "Carbon Black App Control blocked the signed Windows update installer as having no certificate, because the updater runs a new file before the agent checks its new certificate"
date: 2026-11-22
category: integration-issues
module: "Windows update install (tauri-plugin-updater, src-tauri/src/managers/updates.rs, src/update/UpdateWindow.tsx)"
problem_type: integration_issue
component: background_job
severity: medium
symptoms:
  - "On a work PC, the in-app update to 1.2.29 did not install, and Longhand showed The update could not be installed"
  - "Carbon Black reported: longhand.exe tried to run ...\\longhand-1.2.29-updater-XXXX\\longhand-1.2.29-installer.exe. Publisher: No Publisher (Valid[No] Trusted[Ineligible:No Cert])"
  - "The same release installer has a valid, timestamped Authenticode signature when it is checked on another computer"
root_cause: async_timing
resolution_type: code_fix
framework_version: "tauri-plugin-updater 2.14.1 on Windows"
tags:
  - updater
  - windows
  - code-signing
  - artifact-signing
  - carbon-black
  - app-control
  - endpoint-security
  - installer
check: "none: first case, and the cause is a third-party endpoint agent on a user's PC; no repo check can observe it. The Download Installer action and its tests in src-tauri/src/managers/updates/flow.rs keep the way around it"
retire_when: "tauri-plugin-updater waits before it starts the downloaded installer, or Azure Artifact Signing issues long-lived certificates. Read the plugin's write_to_temp and install code for the version in Cargo.lock, and the certificate validity of a release installer."
---

# Carbon Black App Control blocked the signed Windows update installer as having no certificate

## Problem

Endpoint security on a managed Windows PC (Carbon Black App Control) blocked the installer that the in-app update started, and said the file had no certificate. The installer was signed correctly. The automatic update and Try Again failed the same way, and the user had to find the installer by hand.

## Symptoms

- The update window showed "The update could not be installed".
- The Carbon Black event said `Publisher: No Publisher (Valid[No] Trusted[Ineligible:No Cert])` for `%TEMP%\longhand-1.2.29-updater-XXXX\longhand-1.2.29-installer.exe`.

## What Didn't Work

- Searching for a signing defect. The release asset `Longhand_x64-setup.exe` of v1.2.29 has a valid Authenticode signature (signer `CN=Ervin Walter` from Microsoft ID Verified CS EOC CA 03, chain to Microsoft Identity Verification Root Certificate Authority 2020, RFC 3161 timestamp). The URL in `latest-windows.json` serves a file with the same SHA-256. The release job also refuses an unsigned or untimestamped installer (`scripts/windows-signing.ts`).
- Suspecting the updater changed the file. tauri-plugin-updater 2.14.1 downloads the whole file into memory, checks the minisign signature over those exact bytes, writes the same bytes with `write_all` into a new temp folder (`write_to_temp`), and starts it with `ShellExecuteW` at once. A changed byte fails the minisign check, so the file that runs is the signed release file.

## Solution

The cause is on the endpoint, so Longhand gives the user a way around it (PR #111). The "could not be installed" window has a Download Installer button, and the update offer has a Download installer link. Both open the newest release's installer (https://github.com/twistymaze/longhand/releases/latest/download/Longhand_x64-setup.exe, or the `.dmg` on macOS) in the browser. The user runs it later, and the NSIS installer updates the installed app in place.

What the company IT can do, from Broadcom's guidance:

- Let the agent download missing certificates when it builds the chain: agent config `cert_chain_flags=0x28000001` (Broadcom KB 419210).
- Approve the publisher by name ("Ervin Walter"), not by certificate thumbprint: Artifact Signing gives a new certificate every 3 days, so each release has a different one.

## Why This Works

The likely cause, which was not confirmed on the endpoint: when the App Control agent first sees a new certificate, it checks the chain only against the certificates that the PC has cached. A second check that can download missing certificates runs up to 60 seconds later. A new file that runs within those 60 seconds is blocked, and it is approved about a minute later. Broadcom says this makes software updates fail until the user tries again (KB 419210). Each Longhand release has a certificate that the PC has not seen, and the updater runs the installer seconds after it writes it, so each release can hit this. A Defender scan that delays the agent's check gives the same result (KB 403841). A PC that does not have Microsoft Identity Verification Root CA 2020 cannot check the chain at all (KB5022661).

A browser download that the user runs later gives the agent time to finish its check.

## Prevention

- When a Windows user reports a blocked update, first verify the release asset's signature and hash. Do not change the signing setup to fix an endpoint check that runs too early.
- Do not add a delay before the updater starts the installer to work around one security product: every user would wait.

## Related Issues

- [One release for both platforms with fixed installer names](../tooling-decisions/one-release-for-both-platforms-with-fixed-installer-names.md): the fixed asset names that the download link uses.
- Plan: `docs/plans/2026-11-21-2134-feat-manual-installer-download-plan.md`.
- Broadcom KB 419210: https://knowledge.broadcom.com/external/article/419210/publisher-blocks-due-to-partial_chain-er.html
- Broadcom KB 403841: https://knowledge.broadcom.com/external/article/403841/unanalyzed-blocks-during-chrome-updates.html
- Microsoft KB5022661: https://support.microsoft.com/en-us/topic/kb5022661-windows-support-for-the-azure-code-signing-program-4b505a31-fa1e-4ea6-85dd-6630229e8ef4
