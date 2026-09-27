# Provenance

Borrowed material in this crate, one record per file (R27). `src/windows.rs` and `src/stream.rs` are written for Longhand; `src/windows.rs` reuses the VAD constants of `crates/vad`, whose own record covers them.

## `src/engine.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/managers/transcription.rs` (`init_transcribe_backend`, the `Model::load_with` / `session.run` invocation, and the `session.stream` invocation with `StreamOptions::default()` in `run_stream_worker`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — backend start-up, the load/run sequence and the stream start kept; device selection, translation and custom-word prompts dropped; results keep their segments instead of collapsing to text; windows under 1 s are zero-padded and conditioning on previous tokens is off; a stream asks for token timestamps and returns its tokens, not only its text
