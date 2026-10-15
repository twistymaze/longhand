# Provenance

Borrowed material in this crate, one record per file (R27). The provider in `src/lib.rs` — the capture thread, explicit start, fault reporting and frame hand-off — is new (KTD3).

## `src/config.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/audio_toolkit/audio/recorder.rs` (`get_preferred_config`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — ported to the cpal 0.18 API; `device_name` reads `description().name()` with a fallback, so an unreadable description cannot panic; the Bluetooth-default replacement follows anarlog, recorded next

## `src/config.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/listener-core/src/actors/source/stream.rs` (`active_mic_device`, the reasoning for swapping a Bluetooth default)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — decided from the transport cpal reports instead of anarlog's device layer

## `src/startup.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 693500220d8e253b175e7a5b88fd831c48b4583b
- Source paths: `crates/audio-actual/src/mic.rs` (`StartupPermit`, the `init_rx.recv_timeout` wait of 5 s)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the permit takes its flag as a parameter so that tests use their own flag; the wait checks a cancel in short steps, so a Stop does not wait for the limit; the device lookup and the config query run inside the limit; the thread is not joined after a timeout or a cancel

## `src/callback.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/audio_toolkit/audio/recorder.rs` (`build_stream`, `write_input_to_ring`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — pause handshake and channel selection removed; the error callback records the cpal 0.18 error kind so a lost device ends capture; overruns are reported with their position so the worker can fill them with silence (U25)
