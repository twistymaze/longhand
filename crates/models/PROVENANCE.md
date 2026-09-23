# Provenance

Borrowed material in this crate, one record per file (R27). The weights themselves are not borrowed: `catalog.json` pins each to its publisher's repository and revision (R28), and the licences view lists their terms.

## `src/catalog.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/catalog/mod.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — the compiled-in JSON catalog and its revision pinning kept; mirrors, quant lists, languages, scores and ranking removed; one file per entry with kind, upstream URL, size, SHA-256 and licence

## `catalog.json`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/catalog/catalog.json` (the Whisper large-v3-turbo Q5_K_M entry: repository, revision, size, SHA-256)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — one recognition entry reshaped to this schema; the two diarization entries are new, pinned to their publishers' revisions

## `src/download.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/managers/model/download.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — detached from `ModelManager` and Tauri; typed errors instead of `anyhow`; connect and stall limits are parameters; `ensure_model` moves a verified partial into place

## `src/download/tests.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/managers/model/download/tests.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — assertions on typed errors; stall-then-resume and `ensure_model` tests added
