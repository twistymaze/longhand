---
title: "The system audio permission check kept its old result after the user allowed the tap prompt"
date: 2026-09-26
category: integration-issues
module: "System audio permission (crates/audio-system/src/permission.rs)"
problem_type: integration_issue
component: tauri_backend
severity: high
symptoms:
  - "In the signed macOS app, Settings showed \"Waiting…\" on the Computer audio row and never changed to \"Allowed\""
  - "The user had clicked \"Allow…\", saw the macOS prompt and allowed it"
  - "The log showed that the app started 11 s before the grant"
root_cause: wrong_api
resolution_type: code_fix
framework_version: "tauri 2 on macOS 26"
related_components:
  - capture
tags: [macos, tcc, permission, system-audio, process-tap, tccaccesspreflight, signed-bundle]
---

# The system audio permission check kept its old result after the user allowed the tap prompt

## Problem
macOS has no public API for the "Screen & System Audio Recording" permission. Longhand calls the private `TCCAccessPreflight` function for `kTCCServiceAudioCapture`, as AudioCap does. After the user allowed the prompt that the first tap opens, the function gave the old result ("not asked") for the rest of the process life. So the permission row stayed on "Waiting…".

## Symptoms
- The Computer audio row in Settings stayed on "Waiting…" after the user allowed the macOS prompt.
- The log showed that the app started 11 s before the grant. The check had run in the same process before and after the grant.

## What Didn't Work
- **Polling the check again in the same process.** The request code already polled `preflight` after the tap, because the prompt does not wait for the tap. Each poll returned the same old result. More polls or a longer wait do not help.
- **Testing in `tauri dev`.** The dev binary's request goes to the terminal that started it, so it cannot show this behaviour. Test only in the signed bundle (see `CLAUDE.md`).

## Solution
When the in-process check does not say "allowed", run the check again in a new process. The new process is the app binary itself, started with a flag that is not for users. It runs the check, exits with a code for the state, and the app reads the code. anarlog does the same with a separate helper binary (`plugins/permissions/swift/check-permissions.swift` in the anarlog clone). Longhand uses its own binary, so there is no second binary to build and sign.

`crates/audio-system/src/permission.rs`:

```rust
pub const CHILD_FLAG: &str = "--system-audio-permission";

/// An in-process "allowed" is final. Any other in-process result can be old.
pub(crate) fn combine(
    in_process: Permission,
    child: impl FnOnce() -> Option<Permission>,
) -> Permission {
    if in_process == Permission::Allowed {
        return in_process;
    }
    child().unwrap_or(in_process)
}

pub fn preflight() -> Permission {
    #[cfg(target_os = "macos")]
    {
        combine(preflight_with(tcc::load), check_in_child)
    }
    // ...
}
```

`check_in_child` starts `std::env::current_exe()` with `CHILD_FLAG` and null stdio, and maps the exit code: 0 allowed, 1 denied, 2 not asked, anything else unknown. If the process does not start, the in-process result stays.

`src-tauri/src/main.rs` runs the child side before the app starts, so the child opens no window and no tray icon:

```rust
if let Some(code) = audio_system::permission::child_check(std::env::args()) {
    std::process::exit(code);
}
```

## Why This Works
The TCC result appears to be kept in the process after the first check. A new process reads the current state. The in-process result is still used first, because it is correct when it says "allowed" and it takes about a millisecond. Only a result other than "allowed" pays for the new process, some tens of milliseconds.

## Prevention
- **Any TCC check done through a private preflight function can be old.** When Longhand adds a check for another TCC service, use the same `combine` rule: trust an in-process "allowed", and ask a new process for anything else.
- **Call `child_check` first in `main`.** If code runs before it, the child can start the Tauri app, open a window or take the single-instance lock.
- **The test binary cannot act as the child.** The test harness does not know `CHILD_FLAG` and exits with 101. The hardware test in `crates/audio-system/tests/hardware.rs` therefore uses the in-process check, and `combine` is tested with a closure in place of the child.
- **Test the grant flow in the signed bundle, from a fresh start.** Reset the permission (the TCC service is `kTCCServiceAudioCapture`, so `tccutil reset AudioCapture net.ewal.longhand` should do it; this command was not tested in archived PR 12), open the app, click "Allow…", allow the prompt, and look at the row without a restart.

## Related Issues
- Archived PR 12 (system audio and microphone choice), which added the permission check and this fix.
- `crates/audio-system/PROVENANCE.md`, the `src/permission.rs` record.
- [WKWebView does not focus a clicked button](../ui-bugs/wkwebview-click-does-not-focus-button-breaks-shortcut-capture.md): another defect that only the signed bundle shows.
