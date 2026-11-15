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
- Revision: aca8985190c3ad7f361d8a80cb30f179513ed690
- Source paths: `src-tauri/src/catalog/catalog.json`, a copy of the `@handy-computer/models` 0.2.3 package catalog, schema 2 (four recognition entries: `parakeet-unified-en-0.6b`, `nemotron-speech-streaming-en-0.6b`, `nemotron-3.5-asr-streaming-0.6b` and `parakeet-tdt-0.6b-v3`: repository, pinned revision, default quant file name, size and SHA-256, streaming, the timestamps flag, language tags and licence id)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — the recognition entries are reshaped to this schema, one file for each entry. `scripts/refresh-catalog.py` (Longhand's own code) makes four of them from Handy's file: the package's default quant, the plain file for each model, and a Hugging Face URL at the pinned revision instead of Handy's mirror. Region tags such as `en-US` become language codes. The timestamp level, the assessment scores of all five entries and the Whisper entry's artifact values come from Handy's schema 1 catalog at 8f9cf53cd1410cda26beea39ff802ac306e39585, through the checked-in catalog. The Whisper entry keeps its Q5_K_M file, with Handy's language codes in the order of that revision. Names and descriptions are Longhand's. Each licence is the one that the Hugging Face model card names: the NVIDIA Open Model License for Parakeet Unified EN (Handy says `cc-by-4.0`) and Nemotron Speech Streaming EN (Handy says "other"), and OpenMDW-1.1 for Nemotron 3.5 ASR Streaming. The four speaker-analysis artifacts are pinned to publisher revisions. Sortformer V2 and Nemotron 3 use the portable Q8 NeMo format and upstream Hugging Face LFS hashes. NeMo ASR artifacts use separate identities for Nemotron English, Nemotron 3.5 and Parakeet TDT 0.6B v3. Their upstream Hugging Face revisions and LFS hashes were read on 2026-09-27. Assessment scores preserve Handy attribution; speaker accuracy and benchmark context remain unknown

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
