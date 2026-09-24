//! The compiled-in weight catalog (KTD19, R28).
//!
//! Shape follows Handy's `src/catalog/mod.rs`: a JSON file compiled into the
//! binary, each file pinned to an immutable upstream revision with its size
//! and SHA-256, so downloaded bytes provably match the catalog. Handy's
//! mirror list is not taken: its mirror is Handy's own host, and R28 has the
//! weights come from their upstream source.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Asr,
    Segmentation,
    Embedding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Licence {
    pub name: String,
    pub url: String,
    pub attribution: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogModel {
    pub id: String,
    pub kind: ModelKind,
    pub name: String,
    pub description: String,
    /// Recognition models only: whether a live transcript is possible (R10).
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub timestamps: Option<String>,
    pub repository: String,
    pub revision: String,
    /// Name on disk inside the models directory.
    pub filename: String,
    pub url: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub licence: Licence,
}

#[derive(Deserialize)]
struct CatalogRoot {
    models: Vec<CatalogModel>,
}

pub static CATALOG: LazyLock<Vec<CatalogModel>> = LazyLock::new(|| {
    serde_json::from_str::<CatalogRoot>(include_str!("../catalog.json"))
        .expect("bundled catalog.json is valid")
        .models
});

pub fn model(id: &str) -> Option<&'static CatalogModel> {
    CATALOG.iter().find(|m| m.id == id)
}

/// The recognition model a fresh install selects (KTD6).
pub const DEFAULT_ASR_MODEL: &str = "whisper-large-v3-turbo-q5km";

/// The models processing cannot run without: the selected recognition model
/// and both diarization weights.
pub fn required_for(asr_model: &str) -> Vec<&'static CatalogModel> {
    CATALOG
        .iter()
        .filter(|m| match m.kind {
            ModelKind::Asr => m.id == asr_model,
            ModelKind::Segmentation | ModelKind::Embedding => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_parses_with_one_model_of_each_kind() {
        for kind in [
            ModelKind::Asr,
            ModelKind::Segmentation,
            ModelKind::Embedding,
        ] {
            assert!(CATALOG.iter().any(|m| m.kind == kind), "{kind:?}");
        }
    }

    #[test]
    fn every_entry_is_pinned_and_verifiable() {
        for m in CATALOG.iter() {
            assert_eq!(m.revision.len(), 40, "{}: revision is not a commit", m.id);
            assert!(
                m.url.contains(&m.revision),
                "{}: url is not at the pinned revision",
                m.id
            );
            assert!(m.url.starts_with("https://"), "{}", m.id);
            assert_eq!(m.sha256.len(), 64, "{}", m.id);
            assert!(m.size_bytes > 0, "{}", m.id);
            assert!(!m.licence.attribution.is_empty(), "{}", m.id);
        }
    }

    #[test]
    fn filenames_are_unique_and_plain() {
        let mut names: Vec<_> = CATALOG.iter().map(|m| m.filename.as_str()).collect();
        assert!(names.iter().all(|n| !n.contains('/') && !n.contains("..")));
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
    }

    #[test]
    fn default_model_needs_three_files() {
        assert!(model(DEFAULT_ASR_MODEL).is_some());
        assert_eq!(required_for(DEFAULT_ASR_MODEL).len(), 3);
    }
}
