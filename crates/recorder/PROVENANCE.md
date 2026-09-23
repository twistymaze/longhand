# Provenance

Borrowed material in this crate, one record per file (R27). `src/wav.rs` and the segment loop in `src/lib.rs` are new.

## `src/chunks.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/listener-core/src/actors/recorder/chunks.rs` (`ChunkedSink`, `list_recovery_chunks`, `recover_partial_chunks`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — native-rate 16-bit WAV instead of 16 kHz MP3; segment index in the name; no archive stream, acknowledgement or zero-retention path; lookback held as samples; storage and flush cadence counted in audio time; recovery repairs the WAV header instead of decoding

## `src/storage.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/listener-core/src/actors/recorder/chunks.rs` (`check_storage`, `storage_budget`, `DISK_RESERVE_BYTES`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — the 256 MB `audio-recovery/` cap removed (KTD4); the free-space lookup is injectable for tests
