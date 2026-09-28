# Provenance

Borrowed material in this crate, one record per file (R27). The weights themselves are not borrowed: `catalog.json` pins each to its publisher's repository and revision (R28), and the licences view lists their terms.

## `src/catalog.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/catalog/mod.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — the compiled-in JSON catalog and its revision pinning kept; mirrors, quant lists and ranking removed; source-linked ASR scores retained; one file per entry with kind, upstream URL, size, SHA-256, licence, language codes and a recommended flag

## `catalog.json`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/catalog/catalog.json` (five recognition entries: `parakeet-unified-en-0.6b`, `nemotron-speech-streaming-en-0.6b`, `nemotron-3.5-asr-streaming-0.6b`, `parakeet-tdt-0.6b-v3` and `whisper-large-v3-turbo`: repository, revision, file name, size, SHA-256, streaming, timestamp level, languages and licence id)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — the recognition entries are reshaped to this schema, one file for each entry. `scripts/refresh-catalog.py` (Longhand's own code) makes four of them from Handy's file: Handy's default quant, the plain file for each model, and a Hugging Face URL at the pinned revision instead of Handy's mirror. The Whisper entry keeps its Q5_K_M file, with Handy's language codes added. Names and descriptions are Longhand's. Handy's licence id "other" becomes the licence that each Hugging Face model card names: the NVIDIA Open Model License, or OpenMDW-1.1 for Nemotron 3.5 ASR Streaming. The four speaker-analysis artifacts are pinned to publisher revisions. Sortformer V2 and Nemotron 3 use the portable Q8 NeMo format and upstream Hugging Face LFS hashes. NeMo ASR artifacts use separate identities for Nemotron English, Nemotron 3.5 and Parakeet TDT 0.6B v3. Their upstream Hugging Face revisions and LFS hashes were read on 2026-09-27. Assessment scores preserve Handy attribution; speaker accuracy and benchmark context remain unknown

## `src/download.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/managers/model/download.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — detached from `ModelManager` and Tauri; typed errors instead of `anyhow`; connect and stall limits are parameters; `ensure_model` moves a verified partial into place

## `src/download/tests.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/managers/model/download/tests.rs`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — assertions on typed errors; stall-then-resume and `ensure_model` tests added
