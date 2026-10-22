# Provenance

Borrowed material in this crate, one record per file (R27).

## `src/lib.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/audio/src/lib.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied


## `src/joiner.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/audio-actual/src/capture/joiner.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the microphone is the clock: the joiner pairs each microphone frame with a sample queue of the speaker, pads the speaker with silence at once in place of a four-frame wait, and drops only the oldest speaker samples above a 60 ms bound. It never pads or drops the microphone (KTD5 of the system audio plan).


## `src/resampler.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/audio_toolkit/audio/resampler.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: copied — moved here from `crates/audio-cpal` so that the microphone and the system audio backends both use it


## `src/live_mix.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/owhisper-client/src/local_soniqo_live.rs` (`echo_score_at_lag` and the `ECHO_GATE_MIN_SAMPLES` and `ECHO_GATE_LAG_STEP_SAMPLES` constants)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — `score` keeps only the correlation of `echo_score_at_lag`; the microphone copy check (`src/mic_copy.rs`) uses it. The plain mix in the same file is new (KTD9 of the app audio plan).
