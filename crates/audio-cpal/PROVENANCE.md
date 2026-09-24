# Provenance

Borrowed material in this crate, one record per file (R27). The provider in `src/lib.rs` — the capture thread, explicit start, fault reporting and frame hand-off — is new (KTD3).

## `src/resampler.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/audio_toolkit/audio/resampler.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: copied

## `src/config.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/audio_toolkit/audio/recorder.rs` (`get_preferred_config`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — ported to the cpal 0.18 API; the Bluetooth-default replacement follows anarlog, recorded next

## `src/config.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/listener-core/src/actors/source/stream.rs` (`active_mic_device`, the reasoning for swapping a Bluetooth default)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — decided from the transport cpal reports instead of anarlog's device layer

## `src/callback.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/audio_toolkit/audio/recorder.rs` (`build_stream`, `write_input_to_ring`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — pause handshake and channel selection removed; the error callback records the cpal 0.18 error kind so a lost device ends capture; overruns are reported with their position so the worker can fill them with silence (U25)
