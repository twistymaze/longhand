//! The full licence list (R9, KTD6). The generator writes one text file for
//! each platform into `licences/generated/`. The binary contains the file for
//! the platform that it is built for, so the window needs no file access.

/// The full licence list for the platform of this build.
#[cfg(target_os = "windows")]
const FULL_LIST: &str = include_str!("../../../licences/generated/windows.txt");
#[cfg(not(target_os = "windows"))]
const FULL_LIST: &str = include_str!("../../../licences/generated/macos.txt");

/// Return the full licence list for the platform of this build.
#[tauri::command]
#[specta::specta]
pub fn licence_text() -> String {
    FULL_LIST.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_names_the_main_components() {
        let text = licence_text();
        assert!(!text.trim().is_empty());
        for name in ["ggml", "ONNX Runtime", "tauri"] {
            assert!(text.contains(name), "the licence list does not name {name}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_list_has_no_windows_only_entry() {
        let text = licence_text();
        for name in ["vcruntime140", "Visual C++", "Vulkan"] {
            assert!(
                !text.contains(name),
                "the macOS licence list names the Windows-only entry {name}"
            );
        }
    }
}
