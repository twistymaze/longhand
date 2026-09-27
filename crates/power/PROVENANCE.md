# Provenance

Borrowed material in this crate, one record per file (R27). `src/assertion.rs`, `src/sleep/mod.rs` and `src/sleep/windows.rs` are new.

## `src/sleep/macos.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/detect/src/sleep.rs` (`SleepObserver`, `SleepDetector`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — emits `WillSleep` and `DidWake` to a callback instead of `DetectEvent::SleepStateChanged`; started by a constructor and stopped on drop instead of the `Observer` trait; the observer thread is named and a spawn failure is logged
