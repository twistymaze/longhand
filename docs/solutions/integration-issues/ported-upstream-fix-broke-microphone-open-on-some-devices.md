---
title: "A ported upstream fix (fixed 500 ms WASAPI buffer) stopped some microphones from opening"
date: 2026-11-18
category: integration-issues
module: "Microphone capture (crates/audio-cpal)"
problem_type: integration_issue
component: service_layer
severity: high
symptoms:
  - "Windows: the microphone does not open, and the log shows \"Failed to initialize audio client: The parameter is incorrect. (os error -2147024809)\""
  - "This occurs only with the 500 ms buffer request, on the Windows 10 Remote Audio device and on some USB headsets"
root_cause: wrong_api
resolution_type: code_fix
retire_when: "cpal or handy-recorder opens these devices with a fixed buffer, or cjpais/Handy#2531 closes with an upstream fix; then compare that fix with with_buffer_fallback"
related_components:
  - capture
tags: [windows, wasapi, cpal, buffer-size, remote-desktop, upstream-port, handy-recorder, e-invalidarg]
---

# A ported upstream fix (fixed 500 ms WASAPI buffer) stopped some microphones from opening

## Problem

Longhand ported handy-recorder's fix for audio loss over Remote Desktop: on Windows, the capture stream asks WASAPI for a fixed 500 ms buffer (`BufferSize::Fixed(sample_rate / 2)`). A week after that fix was released, Handy users reported that the same request stops some microphones from opening. Without a fallback, these users could not record at all.

## Symptoms

- `Failed to build input stream: Failed to initialize audio client: The parameter is incorrect. (os error -2147024809)` (`E_INVALIDARG`).
- The failure occurs on the Windows 10 Remote Audio device (the device that the fix is for) and on some USB headsets. The same devices open with the default buffer (cjpais/Handy#2531).

## What Didn't Work

- A direct port of the upstream commit. The commit was small and approved, and the local tests passed. Unit tests cannot show what a WASAPI driver accepts.
- cpal 0.18.2 does not prevent the failure. Its WASAPI code says that `IAudioClient::Initialize` accepts any positive buffer duration in shared mode, and it does not check the range (the cpal crate's WASAPI `build_input_stream_raw_inner`). Some drivers do not follow that rule.

## Solution

Keep the fixed buffer, but build the stream again with the default buffer when the fixed build fails (PR #101):

```rust
// crates/audio-cpal/src/callback.rs
pub fn with_buffer_fallback<T, E: std::fmt::Display>(
    preferred: BufferSize,
    mut build: impl FnMut(BufferSize) -> Result<T, E>,
) -> Result<T, E> {
    match build(preferred) {
        Err(e) if preferred != BufferSize::Default => {
            log::warn!("...; using the default buffer");
            build(BufferSize::Default)
        }
        result => result,
    }
}
```

`build` in `crates/audio-cpal/src/lib.rs` calls it with `capture_buffer_size(cfg!(windows), rate)`. On macOS the preferred buffer is `Default`, so nothing changes. A device that rejects the fixed buffer opens as before the port; on that device, the Remote Desktop audio loss can still occur.

## Why This Works

The failure is in `Initialize`, which runs when the stream is built, before `play()`. So a second build with `BufferSize::Default` (duration 0, the value that cpal used before) restores the old behaviour for exactly the devices that reject the new request.

## Prevention

- **Before you port a recent upstream fix, search the issue trackers for reports against that fix.** Search the upstream project and its consumers (for handy-recorder, also cjpais/Handy): the PR number, the commit, and the error text that the change can cause. A regression report often comes days after a release. In this case, only the independent cross-model review found cjpais/Handy#2531.
- When a fix makes a new request of the platform that a driver can refuse (a buffer size, a sample format, an exclusive mode), keep the previous request as a fallback, and test the fallback order with a closure, as `a_rejected_fixed_buffer_is_tried_again_with_the_default` does.
