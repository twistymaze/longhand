//! Playback of the failed-capture sound.

/// Play the failed-capture cue once, so the user knows the sound (R18).
#[tauri::command]
#[specta::specta]
pub fn play_alert_sound() -> Result<(), String> {
    sfx::play_cue().map_err(|e| e.to_string())
}
