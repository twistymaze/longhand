# Provenance

Borrowed material in this crate, one record per file (R27).

## `src/lib.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/audio/src/lib.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: copied


## `src/joiner.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/audio-actual/src/capture/joiner.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — the microphone is the clock: the joiner pairs each microphone frame with a sample queue of the speaker, pads the speaker with silence at once in place of a four-frame wait, and drops only the oldest speaker samples above a 60 ms bound. It never pads or drops the microphone (KTD5 of the system audio plan).


## `src/resampler.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/audio_toolkit/audio/resampler.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: copied — moved here from `crates/audio-cpal` so that the microphone and the system audio backends both use it
