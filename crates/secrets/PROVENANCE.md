# Provenance

Borrowed material in this crate, one record per file (R27). The Tauri plugin surface, scoped JSON store, renderer/native caller split and legacy-location read-through are not taken: Longhand calls the store only from Rust and has no earlier storage location to migrate from.

## `src/error.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `plugins/store2/src/commands.rs` (`secure_store_error`, `keychain_error_code`, the macOS and Linux messages); since anarlog 9efc3d98 these are in `crates/desktop-identity/src/secure_store.rs` (`secure_store_error` is now `error_message`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — mapped onto a typed `SecretError` taxonomy (R20b) with `-34018` and `-25308` distinguished

## `src/lib.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `plugins/store2/src/commands.rs` (`secure_store_service`, `secret_entry`, `repair_macos_keychain_access`, read/write/delete flow); since anarlog 9efc3d98 all except `repair_macos_keychain_access` are in `crates/desktop-identity/src/secure_store.rs` and `crates/desktop-identity/src/chunked.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — Tauri-free, behind a `Backend` trait, with endpoint-host keying (KTD15, R30a)

