# Provenance

Handy-derived modules in the application crate, one record per file (R27).

## `build.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/build.rs` (`stage_transcribe_runtime_libs`, `split_versioned_so`, `stage_vc_runtime_dlls`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — Apple Intelligence bridge, tray translations, ONNX Runtime staging and Linux rpath removed; environment variable renamed `LONGHAND_VC_REDIST_DIRS`

## `src/window.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/lib.rs` (`show_main_window`, the `CloseRequested` handler)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — settings-driven tray visibility removed; the window always hides to the tray; the Settings window and the Dock-icon rule for it are Longhand's own

## `src/tray.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/lib.rs` (tray builder, Windows click convention)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — menu reduced to open, Settings and quit; the recording-time title is Longhand's own

## `src/managers/settings.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/settings.rs` (`get_settings`, `salvage_settings`, `apply_settings_migrations`, `write_settings`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — Longhand schema; portable mode omitted; writes flush immediately

## `src/lib.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/lib.rs` (tauri-specta builder, bindings export, `RunEvent::Reopen`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified

## `Entitlements.plist`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/Entitlements.plist`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: copied

## `Info.plist`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/Info.plist`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — usage string

## `capabilities/default.json`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/capabilities/default.json`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — duplicated permission lines, the `**` asset scope and the filesystem plugin removed


## `src/managers/destination.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/listener-core/src/actors/recorder/chunks.rs` (`check_storage`: free space from the deepest mount point containing the path; `DISK_RESERVE_BYTES`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — `available_space` only, returning the figure instead of an error; the reserve becomes the destination's reporting floor; the existence, directory and write-probe checks are new

## `src/shortcut.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/shortcut/tauri_impl.rs` (registration through `tauri-plugin-global-shortcut`, handling `Pressed` only)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, LICENSES/Handy-MIT.txt
- Status: modified — one chord that toggles a session; arming checked at the press; presses serialized
