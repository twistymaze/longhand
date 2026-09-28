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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Assessment {
    pub source: String,
    pub source_revision: String,
    pub speed_score: Option<u32>,
    pub asr_accuracy_score: Option<u32>,
    pub speaker_accuracy_score: Option<u32>,
    pub benchmark_context: Option<String>,
}

/// Ordered speaker choices for automatic mode selection. The resolver still
/// checks runtime support, language, timing, and speaker count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SpeakerPreferences {
    pub after_stop: Vec<crate::recognition::SpeakerSource>,
    pub during_recording: Vec<crate::recognition::SpeakerSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogModel {
    #[serde(default)]
    pub speaker_preferences: Option<SpeakerPreferences>,
    pub id: String,
    pub kind: ModelKind,
    pub name: String,
    pub description: String,
    /// Recognition models only: whether a live transcript is possible (R10).
    #[serde(default)]
    pub streaming: bool,
    /// Recognition models only: the timestamp level, "token", "word" or
    /// "segment". The speaker join needs timestamps (R2).
    #[serde(default)]
    pub timestamps: Option<String>,
    /// Recognition models only: the language codes the model recognizes.
    #[serde(default)]
    pub languages: Vec<String>,
    /// The model the picker marks as recommended.
    #[serde(default)]
    pub recommended: bool,
    pub repository: String,
    pub revision: String,
    /// Name on disk inside the models directory.
    pub filename: String,
    pub url: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub licence: Licence,
    #[serde(default)]
    pub assessment: Option<Assessment>,
}

#[derive(Deserialize)]
struct CatalogRoot {
    models: Vec<CatalogModel>,
    #[serde(default)]
    runtime_artifacts: Vec<CatalogModel>,
}

pub static CATALOG: LazyLock<Vec<CatalogModel>> = LazyLock::new(|| {
    serde_json::from_str::<CatalogRoot>(include_str!("../catalog.json"))
        .expect("bundled catalog.json is valid")
        .models
});

pub static RUNTIME_ARTIFACTS: LazyLock<Vec<CatalogModel>> = LazyLock::new(|| {
    serde_json::from_str::<CatalogRoot>(include_str!("../catalog.json"))
        .expect("bundled catalog.json is valid")
        .runtime_artifacts
});

pub fn model(id: &str) -> Option<&'static CatalogModel> {
    CATALOG
        .iter()
        .chain(RUNTIME_ARTIFACTS.iter())
        .find(|m| m.id == id)
}

/// The recognition model a fresh install selects (KD8, R3). An install that
/// already has a stored model keeps it.
pub const DEFAULT_ASR_MODEL: &str = "parakeet-unified-en-0.6b-q80";

/// The models processing cannot run without: the selected recognition model
/// and both diarization weights.
pub fn required_for(asr_model: &str) -> Vec<&'static CatalogModel> {
    CATALOG
        .iter()
        .filter(|m| match m.kind {
            ModelKind::Asr => m.id == asr_model,
            ModelKind::Segmentation | ModelKind::Embedding => matches!(
                m.id.as_str(),
                "pyannote-segmentation-3.0" | "wespeaker-voxceleb-resnet34-lm"
            ),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_speaker_choices_are_valid_and_keep_speakers_enabled() {
        use crate::recognition::{
            ProcessingMode, RecognitionConfig, SpeakerCount, SpeakerSource, resolve,
        };
        for model in CATALOG.iter().filter(|m| m.kind == ModelKind::Asr) {
            let preferences = model
                .speaker_preferences
                .as_ref()
                .expect("ASR speaker policy");
            for (mode, sources) in [
                (ProcessingMode::AfterStop, &preferences.after_stop),
                (
                    ProcessingMode::DuringRecording,
                    &preferences.during_recording,
                ),
            ] {
                assert!(!sources.is_empty(), "{}", model.id);
                for source in sources {
                    assert_ne!(*source, SpeakerSource::Off);
                    resolve(&RecognitionConfig {
                        asr_model: model.id.clone(),
                        speaker_source: *source,
                        speaker_processing_mode: mode,
                        ..Default::default()
                    })
                    .unwrap_or_else(|e| panic!("{}: {e}", model.id));
                }
            }
            assert!(
                preferences
                    .after_stop
                    .iter()
                    .any(|source| resolve(&RecognitionConfig {
                        asr_model: model.id.clone(),
                        speaker_source: *source,
                        speaker_count: Some(SpeakerCount::Exact { count: 9 }),
                        ..Default::default()
                    })
                    .is_ok()),
                "{} must retain an after-Stop choice above the live limit",
                model.id
            );
        }
    }

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
        for m in CATALOG.iter().chain(RUNTIME_ARTIFACTS.iter()) {
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
        let mut names: Vec<_> = CATALOG
            .iter()
            .chain(RUNTIME_ARTIFACTS.iter())
            .map(|m| m.filename.as_str())
            .collect();
        assert!(names.iter().all(|n| !n.contains('/') && !n.contains("..")));
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
    }

    /// R1: id, whether it streams, and the language count of each model.
    const R1: [(&str, bool, usize); 9] = [
        ("parakeet-unified-en-0.6b-q80", true, 1),
        ("nemotron-speech-streaming-en-0.6b-q80", true, 1),
        ("nemotron-3.5-asr-streaming-0.6b-q80", true, 28),
        ("multitalker-parakeet-streaming-0.6b-v1-q80", true, 1),
        ("parakeet-tdt-0.6b-v2-q80", false, 1),
        ("parakeet-tdt-0.6b-v3-q80", false, 25),
        ("parakeet-tdt-1.1b-q5km", false, 1),
        ("whisper-large-v3-turbo-q5km", false, 100),
        ("moss-transcribe-diarize-q80", false, 2),
    ];

    fn asr_models() -> Vec<&'static CatalogModel> {
        CATALOG
            .iter()
            .filter(|m| m.kind == ModelKind::Asr)
            .collect()
    }

    #[test]
    fn asr_entries_preserve_the_nine_models_and_add_granite() {
        let mut ids: Vec<_> = asr_models().iter().map(|m| m.id.as_str()).collect();
        let mut expected: Vec<_> = R1.iter().map(|(id, ..)| *id).collect();
        expected.push("granite-speech-4.1-2b-plus-q5km");
        ids.sort_unstable();
        expected.sort_unstable();
        assert_eq!(ids, expected);
    }

    #[test]
    fn streaming_and_languages_match_r1() {
        for (id, streaming, languages) in R1 {
            let m = model(id).unwrap_or_else(|| panic!("{id} is not in the catalog"));
            assert_eq!(m.streaming, streaming, "{id}: streaming");
            assert_eq!(m.languages.len(), languages, "{id}: language count");
            assert!(
                m.languages
                    .iter()
                    .all(|l| !l.is_empty() && !l.contains(' ')),
                "{id}: language codes"
            );
        }
    }

    #[test]
    fn every_asr_entry_has_timestamps() {
        for m in asr_models() {
            let level = m.timestamps.as_deref();
            assert!(
                matches!(level, Some("token" | "word" | "segment")),
                "{}: timestamp level {level:?}",
                m.id
            );
        }
    }

    #[test]
    fn every_entry_names_its_licence() {
        for m in CATALOG.iter().chain(RUNTIME_ARTIFACTS.iter()) {
            assert!(!m.licence.name.is_empty(), "{}", m.id);
            assert!(m.licence.url.starts_with("https://"), "{}", m.id);
        }
    }

    #[test]
    fn recommended_models_cover_live_and_integrated_afterwards() {
        let recommended: Vec<_> = CATALOG.iter().filter(|m| m.recommended).collect();
        assert_eq!(
            recommended
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            [
                "parakeet-unified-en-0.6b-q80",
                "nemotron-speech-streaming-en-0.6b-q80",
                "granite-speech-4.1-2b-plus-q5km"
            ]
        );
        assert!(recommended[0].streaming);
        assert!(
            recommended[2]
                .speaker_preferences
                .as_ref()
                .unwrap()
                .after_stop
                .contains(&crate::recognition::SpeakerSource::Integrated)
        );
    }

    #[test]
    fn default_model_needs_three_files() {
        assert_eq!(DEFAULT_ASR_MODEL, "parakeet-unified-en-0.6b-q80");
        let mut ids: Vec<_> = required_for(DEFAULT_ASR_MODEL)
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            [
                "parakeet-unified-en-0.6b-q80",
                "pyannote-segmentation-3.0",
                "wespeaker-voxceleb-resnet34-lm",
            ]
        );
    }

    #[test]
    fn unknown_model_needs_only_the_diarization_weights() {
        let kinds: Vec<_> = required_for("no-such-model")
            .iter()
            .map(|m| m.kind)
            .collect();
        assert_eq!(kinds.len(), 2);
        assert!(!kinds.contains(&ModelKind::Asr));
    }
}
