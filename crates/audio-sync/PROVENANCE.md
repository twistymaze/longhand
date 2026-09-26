# Provenance

Borrowed material in this crate, one record per file (R27). anarlog's `tests/e2e.rs` is not taken: it reads recorded audio. The tests in `tests/delay.rs` are Longhand's own and use synthetic signals only.

## `src/lib.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-sync/src/lib.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: copied

## `src/drift.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-sync/src/drift.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: copied

## `src/estimator.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-sync/src/estimator.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — the test helpers `delay_signal` and `advance_signal` copy with `copy_from_slice`, for the pinned clippy

## `src/level.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-sync/src/level.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: copied

## `src/probe.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-sync/src/probe.rs`; `crates/audio-actual/src/capture/stream.rs` (the window size rule in `AecReferenceAligner::new`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — `SyncProbeConfig::for_max_lag` added, from the window size rule of anarlog's `AecReferenceAligner::new` (the maximum lag plus 400 ms of overlap); the test helpers `delay_signal` and `advance_signal` copy with `copy_from_slice`, for the pinned clippy
