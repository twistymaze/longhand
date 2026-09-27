# Provenance

Handy-derived modules in the application crate, one record per file (R27).

## `build.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/build.rs` (`stage_transcribe_runtime_libs`, `split_versioned_so`, `stage_vc_runtime_dlls`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — Apple Intelligence bridge, tray translations, ONNX Runtime staging and Linux rpath removed; environment variable renamed `LONGHAND_VC_REDIST_DIRS`

## `src/window.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/lib.rs` (`show_main_window`, the `CloseRequested` handler)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — settings-driven tray visibility removed; the window always hides to the tray; the Settings window and the Dock-icon rule for it are Longhand's own

## `src/tray.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/lib.rs` (tray builder, Windows click convention)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — menu reduced to open, Settings and quit; the recording-time title is Longhand's own

## `src/managers/settings.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/settings.rs` (`get_settings`, `salvage_settings`, `apply_settings_migrations`, `write_settings`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — Longhand schema; portable mode omitted; writes flush immediately

## `src/lib.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/lib.rs` (tauri-specta builder, bindings export, `RunEvent::Reopen`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified

## `Entitlements.plist`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/Entitlements.plist`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: copied

## `Info.plist`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/Info.plist`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — usage string

## `capabilities/default.json`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/capabilities/default.json`
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — duplicated permission lines, the `**` asset scope and the filesystem plugin removed


## `src/managers/destination.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/listener-core/src/actors/recorder/chunks.rs` (`check_storage`: free space from the deepest mount point containing the path; `DISK_RESERVE_BYTES`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — `available_space` only, returning the figure instead of an error; the reserve becomes the destination's reporting floor; the existence, directory and write-probe checks are new

## `src/managers/library.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `plugins/notify/src/ext.rs` (`Notify::start` and `Notify::stop`: a `notify-debouncer-full` debouncer with a 900 ms delay, one recursive watch, the event-kind filter), `plugins/notify/src/path.rs` (`should_skip_path`, `to_relative_path`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — one watch for each retained Destination on a thread that owns the watches, a minute's full scan and the pending-delete timer; an event marks its top-level note folder for a targeted scan, and a rescan, an unpaired rename or an event with no usable path runs a full scan; the path filter also skips every hidden name and keeps audio files; no own-writes registry and no vault settings

## `src/shortcut.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/shortcut/tauri_impl.rs` (registration through `tauri-plugin-global-shortcut`, handling `Pressed` only; `validate_shortcut`), `src-tauri/src/shortcut/mod.rs` (`change_binding` and `restore_registration`: unregister, register, and register the old binding again on failure; `suspend_all_shortcuts` and `resume_all_shortcuts`)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — two named shortcuts instead of a binding map; the recording chord toggles a session, with arming checked at the press and presses serialized; a chord needs a modifier; the two chords cannot match; one lock serializes every register, change, suspend and resume; the change decision is a pure function
