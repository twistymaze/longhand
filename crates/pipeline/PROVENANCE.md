# Provenance

Borrowed material in this crate, one record per file (R27). The runner, the checkpoint layout, and the assemble and transcribe stages are written for Longhand.

## `src/resample.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/audio_toolkit/audio/resampler.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — fixed-size output frames removed; the filter's leading delay is discarded and the output length is held to exactly input × out / in, so files written after stop stay on the session timeline
