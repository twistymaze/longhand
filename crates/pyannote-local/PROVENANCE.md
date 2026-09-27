# Provenance

Borrowed material in this crate, one record per file (R27). `Cargo.toml` follows the upstream manifest without the voiceprint dependency. The weights are not bundled: they are the model catalog's downloads (KTD19).

## `src/lib.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/pyannote-local/src/lib.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — exports the analysis, clustering and reconstruction steps; the voiceprint exports are removed

## `src/pipeline.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/pyannote-local/src/pipeline.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the pipeline is split into `Diarizer::analyze` (segmentation and embeddings, returned as an `Analysis`) and public `cluster` and `reconstruct`, so analysis checkpoints per segment and clustering spans a session; the voiceprint path is removed; `max_windows` defaults to 3612 (U1)

## `src/segmentation.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/pyannote-local/src/segmentation.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — `Segmenter::from_path` loads the weights from disk instead of bytes compiled into the binary

## `src/clustering.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/pyannote-local/src/clustering.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `src/metrics.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/pyannote-local/src/metrics.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `src/error.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/pyannote-local/src/error.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `src/bin/diarization_eval.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/pyannote-local/src/bin/diarization_eval.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the weights are read from `LONGHAND_WEIGHTS_DIR`
