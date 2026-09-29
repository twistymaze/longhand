//! The identity of this build (R7, R8, KTD12). CI sets
//! `LONGHAND_BUILD_COMMIT` and `LONGHAND_TRACK` at build time. A build
//! without them is a local build: it has no track and gets no updates.

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

/// The update track of a build. Each track has its own update manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Track {
    Preview,
}

/// The track that a `LONGHAND_TRACK` value names. An absent or unknown value
/// gives no track, so the build makes no automatic check.
pub fn parse_track(value: Option<&str>) -> Option<Track> {
    match value {
        Some("preview") => Some(Track::Preview),
        _ => None,
    }
}

/// The track of this build.
pub fn track() -> Option<Track> {
    parse_track(option_env!("LONGHAND_TRACK"))
}

/// The first 7 characters of a commit hash. An empty value gives none.
fn short_commit(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.chars().take(7).collect())
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct BuildInfo {
    /// The full version, for example `0.1.0-preview.20260928143012`.
    pub version: String,
    /// The short commit of the build. None for a local build.
    pub commit: Option<String>,
    /// None for a local build.
    pub track: Option<Track>,
}

/// Return the version, the short commit and the track of this build.
#[tauri::command]
#[specta::specta]
pub fn build_info(app: AppHandle) -> BuildInfo {
    BuildInfo {
        version: app.package_info().version.to_string(),
        commit: short_commit(option_env!("LONGHAND_BUILD_COMMIT")),
        track: track(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_or_unknown_track_is_no_track() {
        assert_eq!(parse_track(None), None);
        assert_eq!(parse_track(Some("")), None);
        assert_eq!(parse_track(Some("stable")), None);
    }

    #[test]
    fn preview_is_the_preview_track() {
        assert_eq!(parse_track(Some("preview")), Some(Track::Preview));
    }

    #[test]
    fn the_commit_is_shortened_and_an_empty_commit_is_none() {
        assert_eq!(
            short_commit(Some("a1b2c3d4e5f6a7b8c9d0")),
            Some("a1b2c3d".to_string())
        );
        assert_eq!(short_commit(Some("  ")), None);
        assert_eq!(short_commit(None), None);
    }
}
