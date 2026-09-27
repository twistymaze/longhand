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
- Source paths: `crates/owhisper-client/src/local_soniqo_live.rs` (`suppress_echo_dominant_mic`, `best_echo_score`, `echo_score_at_lag` and the `ECHO_GATE_*` constants)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the gate tests the last 120 ms of the microphone every 20 ms against a history of the system channel, on a 16 kHz copy, and searches delays up to 400 ms late and 60 ms early in place of a search inside one chunk. The mix around the gate (the 16 kHz copy, the balance gain of each channel and the mono mix) is new (KTD11 of the system audio plan).
