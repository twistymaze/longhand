# Provenance

Borrowed material in this crate, one record per file (R27). `src/lib.rs` and `src/turns.rs` are written for Longhand.

## `src/wav_source.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/listener2-core/src/batch/diarize.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — `WavAudioSource` (open with its format check, and the seeking read) made public and implemented as `AudioSource` directly; errors map to `DiarizeError` and name the file; the multichannel `MixedAudioSource` is not taken
