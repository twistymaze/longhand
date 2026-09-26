// Prevents an additional console window on Windows in release. Do not remove.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The app starts itself with this flag to read the system audio
    // permission in a new process. That process exits here.
    if let Some(code) = audio_system::permission::child_check(std::env::args()) {
        std::process::exit(code);
    }
    longhand_lib::run()
}
