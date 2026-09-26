# Provenance

Borrowed material in this crate, one record per file (R27). `src/lib.rs` (the public API, the status handle and the own-process exclusion rule) and `src/rate.rs` (the rate reader) are new (KTD4). `src/silence.rs` (the silent Computer audio flag, U5 step 6) is new.

## `src/macos.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-actual/src/speaker/macos.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — the tap excludes Longhand's own process; a dedicated thread owns the tap and the device and reads an `rtrb` ring in place of anarlog's async ring reader; the thread reads the rate that the device measures and resamples to fixed-size frames; a device that is no longer alive ends the stream

## `src/windows.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-actual/src/speaker/windows.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — every channel is downmixed to mono in place of the first channel only; the thread resamples to fixed-size frames and starts the resampler again on an endpoint switch; each follower poll reads the peak meter of each active output for the other-output rule; a read error opens the device that the rule picks now, and ends the stream when none opens

## `src/follow.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-actual/src/speaker/windows.rs` (`EndpointFollower`, `ENDPOINT_POLL_INTERVAL`, `ENDPOINT_SWITCH_CONFIRMATIONS` and their tests)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — moved out of the WASAPI file so that the rule is tested on every platform; `OtherOutputWatch` (the other-output rule, U4 step 6) is new

## `src/convert.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-actual/src/speaker/windows.rs` (`push_wasapi_bytes`, `push_pcm_bytes_first_channel_to_ringbuf`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — the mean of all channels in place of the first channel; the converter resamples to fixed-size frames and takes a new format on an endpoint switch

## `tests/hardware.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-actual/src/speaker/mod.rs` (`test_macos`, `test_windows`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — reads fixed-size frames through the public API; the macOS tone comes from another process (`afplay`), and a second test checks that a tone from the test process is not in the tap

## `src/permission.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/tcc/swift-lib/src/lib.swift` (`_audio_capture_permission_status`), `crates/tcc/src/lib.rs` (the result codes); anarlog took the Swift function from AudioCap (https://github.com/insidegui/AudioCap, `AudioCap/ProcessTap/AudioRecordingPermission.swift` at 93881a4)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — Rust in place of Swift: `dlopen` and `dlsym` load `TCCAccessPreflight` directly, so no Swift build step is needed (KTD12); the function returns a C `int`; a missing function or an unknown result gives "unknown"; the loader is a test seam; `request` (a short tap) and the Windows result are new
