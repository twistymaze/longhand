//! Resolve a recognition choice to pinned artifacts and runtime identities.
use crate::{DEFAULT_ASR_MODEL, ModelKind, model};
use serde::{Deserialize, Serialize};
use specta::Type;

pub const TRANSCRIBE_REVISION: &str = "4807edaf210d0d7e8a6f7fb2a44b65966a2797f0";
pub const NEMO_REVISION: &str = "97a15afa5caa9bce5baaa86c1184103877af4101";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerSource {
    Off,
    #[default]
    Pyannote,
    Sortformer,
    Nemotron,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingMode {
    DuringRecording,
    #[default]
    AfterStop,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpeakerCount {
    Exact { count: u32 },
    Range { min: u32, max: u32 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RecognitionConfig {
    pub asr_model: String,
    pub speaker_source: SpeakerSource,
    pub processing_mode: ProcessingMode,
    #[serde(default)]
    pub speaker_processing_mode: ProcessingMode,
    pub speaker_count: Option<SpeakerCount>,
    pub language: Option<String>,
}
impl Default for RecognitionConfig {
    fn default() -> Self {
        Self {
            asr_model: DEFAULT_ASR_MODEL.into(),
            speaker_source: SpeakerSource::Pyannote,
            processing_mode: ProcessingMode::AfterStop,
            speaker_processing_mode: ProcessingMode::AfterStop,
            speaker_count: None,
            language: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ArtifactSpec {
    pub id: String,
    pub filename: String,
    pub sha256: String,
    pub revision: String,
    pub url: String,
    pub size_bytes: u64,
    pub format: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RuntimeSpec {
    pub name: String,
    pub revision: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Capabilities {
    pub live_transcription: bool,
    pub live_speaker_analysis: bool,
    pub live_attributed_text: bool,
    pub maximum_speakers: Option<u32>,
    /// Catalog support is distinct from an installed-package acceptance test.
    pub locally_verified: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ExecutionSpec {
    pub schema_version: u32,
    pub catalog_revision: String,
    pub config: RecognitionConfig,
    pub artifacts: Vec<ArtifactSpec>,
    pub runtimes: Vec<RuntimeSpec>,
    pub capabilities: Capabilities,
    pub enabled_live_transcription: bool,
    pub enabled_live_speaker_analysis: bool,
    pub preprocessing_revision: String,
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("Unknown transcription model: {model}")]
    UnknownModel { model: String },
    #[error("{model} cannot transcribe during recording")]
    NoLiveTranscription { model: String },
    #[error("Speaker count must be positive and ordered")]
    InvalidCount,
    #[error("The selected speaker route does not support live speaker analysis")]
    NoLiveSpeakerAnalysis,
    #[error("{model} has no compatible artifact for the selected runtime")]
    RuntimeUnavailable { model: String },
    #[error(
        "The selected speaker model supports at most {maximum} speakers; requested {requested}"
    )]
    SpeakerLimit { maximum: u32, requested: u32 },
    #[error("{model} does not support language {language}")]
    Language { model: String, language: String },
}
impl ExecutionSpec {
    /// Count edits do not change the model or runtime identity.
    pub fn validate_count(&self, count: Option<SpeakerCount>) -> Result<(), ResolveError> {
        validate_count(count, self.capabilities.maximum_speakers)
    }
}
fn validate_count(count: Option<SpeakerCount>, limit: Option<u32>) -> Result<(), ResolveError> {
    let requested = match count {
        None => return Ok(()),
        Some(SpeakerCount::Exact { count }) if count > 0 => count,
        Some(SpeakerCount::Range { min, max }) if min > 0 && min <= max => max,
        _ => return Err(ResolveError::InvalidCount),
    };
    if let Some(maximum) = limit
        && requested > maximum
    {
        return Err(ResolveError::SpeakerLimit { maximum, requested });
    }
    Ok(())
}
/// Select the tested runtime without changing the user's model family or timing.
pub fn resolve(config: &RecognitionConfig) -> Result<ExecutionSpec, ResolveError> {
    let combined_live = config.processing_mode == ProcessingMode::DuringRecording
        && matches!(
            config.speaker_source,
            SpeakerSource::Sortformer | SpeakerSource::Nemotron
        )
        && matches!(
            config.asr_model.as_str(),
            "nemotron-speech-streaming-en-0.6b-q80" | "nemotron-3.5-asr-streaming-0.6b-q80"
        );
    resolve_for_runtime(
        config,
        if combined_live {
            AsrRuntime::Nemo
        } else {
            AsrRuntime::Transcribe
        },
    )
}

fn resolve_transcribe(config: &RecognitionConfig) -> Result<ExecutionSpec, ResolveError> {
    let asr = crate::CATALOG
        .iter()
        .find(|m| m.id == config.asr_model)
        .filter(|m| m.kind == ModelKind::Asr)
        .ok_or_else(|| ResolveError::UnknownModel {
            model: config.asr_model.clone(),
        })?;
    if let Some(language) = &config.language
        && !asr.languages.contains(language)
    {
        return Err(ResolveError::Language {
            model: asr.id.clone(),
            language: language.clone(),
        });
    }
    if config.processing_mode == ProcessingMode::DuringRecording && !asr.streaming {
        return Err(ResolveError::NoLiveTranscription {
            model: asr.id.clone(),
        });
    }
    let live_speaker_analysis = matches!(
        config.speaker_source,
        SpeakerSource::Sortformer | SpeakerSource::Nemotron
    );
    if config.speaker_processing_mode == ProcessingMode::DuringRecording && !live_speaker_analysis {
        return Err(ResolveError::NoLiveSpeakerAnalysis);
    }
    let maximum_speakers = match config.speaker_source {
        SpeakerSource::Sortformer => Some(4),
        SpeakerSource::Nemotron => Some(8),
        _ => None,
    };
    validate_count(config.speaker_count, maximum_speakers)?;
    let mut selected = vec![asr];
    let mut runtimes = vec![RuntimeSpec {
        name: "transcribe.cpp".into(),
        revision: TRANSCRIBE_REVISION.into(),
    }];
    match config.speaker_source {
        SpeakerSource::Pyannote => {
            selected.push(model("pyannote-segmentation-3.0").expect("catalog segmentation"));
            selected.push(model("wespeaker-voxceleb-resnet34-lm").expect("catalog embedding"));
            runtimes.push(RuntimeSpec {
                name: "onnx-runtime".into(),
                revision: "1.22.0".into(),
            });
        }
        SpeakerSource::Sortformer | SpeakerSource::Nemotron => {
            let id = if config.speaker_source == SpeakerSource::Sortformer {
                "sortformer-v2-q80"
            } else {
                "nemotron-3-diarization-q80"
            };
            selected.push(model(id).expect("catalog speaker model"));
            runtimes.push(RuntimeSpec {
                name: "NeMo-Speech.cpp".into(),
                revision: NEMO_REVISION.into(),
            });
        }
        SpeakerSource::Off => {}
    }
    let artifacts = selected
        .into_iter()
        .map(|m| ArtifactSpec {
            id: m.id.clone(),
            filename: m.filename.clone(),
            sha256: m.sha256.clone(),
            revision: m.revision.clone(),
            url: m.url.clone(),
            size_bytes: m.size_bytes,
            format: if m.id == "sortformer-v2-q80" || m.id == "nemotron-3-diarization-q80" {
                "nemo-gguf"
            } else if m.kind == ModelKind::Asr {
                "transcribe-gguf"
            } else {
                "onnx"
            }
            .into(),
        })
        .collect();
    use sha2::{Digest, Sha256};
    let catalog_revision = format!("{:x}", Sha256::digest(include_bytes!("../catalog.json")));
    Ok(ExecutionSpec {
        schema_version: 1,
        catalog_revision,
        config: config.clone(),
        artifacts,
        runtimes,
        capabilities: Capabilities {
            live_transcription: asr.streaming,
            live_speaker_analysis,
            live_attributed_text: false,
            maximum_speakers,
            locally_verified: false,
        },
        enabled_live_transcription: config.processing_mode == ProcessingMode::DuringRecording,
        enabled_live_speaker_analysis: config.speaker_processing_mode
            == ProcessingMode::DuringRecording,
        preprocessing_revision: "mono-16khz-v1".into(),
    })
}

/// Internal route selection. This is not a user preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsrRuntime {
    Transcribe,
    Nemo,
}

/// Resolve the same model family for a tested internal route.
pub fn resolve_for_runtime(
    config: &RecognitionConfig,
    runtime: AsrRuntime,
) -> Result<ExecutionSpec, ResolveError> {
    let mut spec = resolve_transcribe(config)?;
    if runtime == AsrRuntime::Transcribe {
        return Ok(spec);
    }
    let id = match config.asr_model.as_str() {
        "nemotron-speech-streaming-en-0.6b-q80" => "nemo-nemotron-speech-streaming-en-0.6b-q80",
        "nemotron-3.5-asr-streaming-0.6b-q80" => "nemo-nemotron-3.5-asr-streaming-0.6b-q80",
        "parakeet-tdt-0.6b-v3-q80" => "nemo-parakeet-tdt-0.6b-v3-q80",
        _ => {
            return Err(ResolveError::RuntimeUnavailable {
                model: config.asr_model.clone(),
            });
        }
    };
    let m = model(id).expect("pinned NeMo artifact");
    spec.artifacts[0] = ArtifactSpec {
        id: m.id.clone(),
        filename: m.filename.clone(),
        sha256: m.sha256.clone(),
        revision: m.revision.clone(),
        url: m.url.clone(),
        size_bytes: m.size_bytes,
        format: "nemo-gguf".into(),
    };
    spec.runtimes
        .retain(|r| r.name != "transcribe.cpp" && r.name != "NeMo-Speech.cpp");
    spec.runtimes.insert(
        0,
        RuntimeSpec {
            name: "NeMo-Speech.cpp".into(),
            revision: NEMO_REVISION.into(),
        },
    );
    spec.capabilities.live_attributed_text =
        spec.capabilities.live_transcription && spec.capabilities.live_speaker_analysis;
    Ok(spec)
}

/// The model bundle of `asr_model`: the artifacts of every valid choice of
/// speaker source, processing mode, and speaker processing mode, without
/// duplicates. The transcription model comes first. A model that is not a
/// transcription model gives an empty bundle.
pub fn bundle(asr_model: &str) -> Vec<ArtifactSpec> {
    let sources = [
        SpeakerSource::Off,
        SpeakerSource::Pyannote,
        SpeakerSource::Sortformer,
        SpeakerSource::Nemotron,
    ];
    let modes = [ProcessingMode::AfterStop, ProcessingMode::DuringRecording];
    let mut artifacts: Vec<ArtifactSpec> = Vec::new();
    for speaker_source in sources {
        for processing_mode in modes {
            for speaker_processing_mode in modes {
                let config = RecognitionConfig {
                    asr_model: asr_model.into(),
                    speaker_source,
                    processing_mode,
                    speaker_processing_mode,
                    speaker_count: None,
                    language: None,
                };
                let Ok(spec) = resolve(&config) else {
                    continue;
                };
                for artifact in spec.artifacts {
                    if !artifacts.iter().any(|a| a.id == artifact.id) {
                        artifacts.push(artifact);
                    }
                }
            }
        }
    }
    if let Some(index) = artifacts.iter().position(|a| a.id == asr_model) {
        let asr = artifacts.remove(index);
        artifacts.insert(0, asr);
    }
    artifacts
}
