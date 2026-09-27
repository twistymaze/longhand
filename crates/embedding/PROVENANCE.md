# Provenance

Borrowed material in this crate, one record per file (R27). `Cargo.toml` follows the upstream manifest; `knf-rs` comes from its crates.io release `=0.3.2`, whose sources are identical to the git revision (e23bd29) upstream pins.

## `src/lib.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/embedding/src/lib.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `src/error.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/embedding/src/error.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied

## `src/onnx/mod.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/embedding/src/onnx/mod.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the model loads from a path (`from_path`) instead of bytes compiled into the binary; tests that needed the bundled weights read them from `LONGHAND_WEIGHTS_DIR` or are ignored, and the snapshot tests are dropped

## `src/onnx/model.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/embedding/src/onnx/model.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the `include_bytes!` of the upstream weights is removed; only the constants remain
