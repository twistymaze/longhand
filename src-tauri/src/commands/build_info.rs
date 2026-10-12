//! The identity of this build (R7, R8, KTD12; KTD5 of the releases plan).
//! CI sets `LONGHAND_BUILD_COMMIT` at build time, and also
//! `LONGHAND_RELEASE=1` for the macOS release build. Only a release build
//! gets updates.

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

/// True when a `LONGHAND_RELEASE` value is `1`. All other values, and no
/// value, give a build that is not a release build. That build makes no
/// automatic check.
pub fn parse_release(value: Option<&str>) -> bool {
    value == Some("1")
}

/// True when this build is a release build.
pub fn is_release_build() -> bool {
    parse_release(option_env!("LONGHAND_RELEASE"))
}

/// The first 7 characters of a commit hash. An empty value gives none.
pub(crate) fn short_commit(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.chars().take(7).collect())
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct BuildInfo {
    /// The full version, for example `1.0.4`.
    pub version: String,
    /// The short commit of the build. None for a local build.
    pub commit: Option<String>,
    /// True for a release build. Only a release build gets updates.
    pub release: bool,
}

/// Return the version, the short commit and the release flag of this build.
#[tauri::command]
#[specta::specta]
pub fn build_info(app: AppHandle) -> BuildInfo {
    BuildInfo {
        version: app.package_info().version.to_string(),
        commit: short_commit(option_env!("LONGHAND_BUILD_COMMIT")),
        release: is_release_build(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_empty_or_other_value_is_not_a_release_build() {
        assert!(!parse_release(None));
        assert!(!parse_release(Some("")));
        assert!(!parse_release(Some("0")));
        assert!(!parse_release(Some("true")));
    }

    #[test]
    fn one_is_a_release_build() {
        assert!(parse_release(Some("1")));
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
