# Provenance

Borrowed material in this crate, one record per file (R27). `silero.rs` is not taken: it depends on a git fork of `vad-rs` (KTD8).

## `src/lib.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/audio_toolkit/vad/mod.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — `WHISPER_SAMPLE_RATE` replaced by a local `VAD_SAMPLE_RATE`; Silero export removed

## `src/earshot.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/audio_toolkit/vad/earshot.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: copied

## `src/smoothed.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 7b2a28a30aa24174292ddd7f4e92e12e6e3bd501
- Source paths: `src-tauri/src/audio_toolkit/vad/smoothed.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — the `BufferedFrame` comment states that `emitted` now affects the pre-roll, and Longhand adds the pre-roll tests

