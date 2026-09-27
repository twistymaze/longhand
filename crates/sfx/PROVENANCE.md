# Provenance

Borrowed material in this crate, one record per file (R27). `src/cue.rs` is new: the cue is synthesized, and it plays through cpal rather than through anarlog's rodio player (see `src/lib.rs`).

## `src/output.rs`

- Repository: https://github.com/cjpais/Handy
- Revision: 8f9cf53cd1410cda26beea39ff802ac306e39585
- Source paths: `src-tauri/src/managers/audio.rs` (`get_mute`: the macOS `output muted of (get volume settings)` probe and the Windows `IAudioEndpointVolume::GetMute` probe)
- Copyright: Copyright (c) 2025 CJ Pais
- Licence: MIT, licences/sources/Handy-MIT.txt
- Status: modified — the macOS script also reads the output volume, and the Windows probe also reads `GetMasterVolumeLevelScalar`; an unknown answer passes the check instead of deciding a restore; no Linux backends
