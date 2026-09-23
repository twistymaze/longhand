# Provenance

Borrowed material in this crate, one record per file (R27). `src/windows.rs` is written for Longhand; it reuses the VAD constants of `crates/vad`, whose own record covers them.

## `src/engine.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/managers/transcription.rs` (`init_transcribe_backend`, the `Model::load_with` / `session.run` invocation)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — backend start-up and the load/run sequence kept; device selection, streaming, translation and custom-word prompts dropped; results keep their segments instead of collapsing to text; windows under 1 s are zero-padded and conditioning on previous tokens is off
