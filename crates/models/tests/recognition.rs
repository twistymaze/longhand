use models::recognition::{
    ProcessingMode, RecognitionConfig, ResolveError, SpeakerCount, SpeakerSource, resolve,
};

#[test]
fn off_needs_no_speaker_artifacts_and_is_deterministic() {
    let config = RecognitionConfig {
        speaker_source: SpeakerSource::Off,
        ..Default::default()
    };
    let spec = resolve(&config).unwrap();
    assert_eq!(spec.artifacts.len(), 1);
    assert_eq!(spec, resolve(&config).unwrap());
}

#[test]
fn legacy_route_keeps_three_artifacts() {
    assert_eq!(
        resolve(&RecognitionConfig::default())
            .unwrap()
            .artifacts
            .len(),
        3
    );
}

#[test]
fn limits_reject_exact_and_range_above_capacity() {
    for count in [
        SpeakerCount::Exact { count: 5 },
        SpeakerCount::Range { min: 2, max: 5 },
    ] {
        let config = RecognitionConfig {
            speaker_source: SpeakerSource::Sortformer,
            speaker_count: Some(count),
            ..Default::default()
        };
        assert!(matches!(
            resolve(&config),
            Err(ResolveError::SpeakerLimit { maximum: 4, .. })
        ));
    }
}

#[test]
fn unknown_count_retains_visible_capacity_and_unverified_claim() {
    let config = RecognitionConfig {
        speaker_source: SpeakerSource::Sortformer,
        ..Default::default()
    };
    let spec = resolve(&config).unwrap();
    assert_eq!(spec.capabilities.maximum_speakers, Some(4));
    assert!(!spec.capabilities.locally_verified);
    assert_eq!(spec.artifacts.len(), 2);
}

#[test]
fn integrated_requires_a_speaker_capable_model() {
    let config = RecognitionConfig {
        speaker_source: SpeakerSource::Integrated,
        ..Default::default()
    };
    assert!(matches!(
        resolve(&config),
        Err(ResolveError::NoIntegratedSpeakers { .. })
    ));
}

#[test]
fn offline_model_cannot_claim_live_text() {
    let config = RecognitionConfig {
        asr_model: "whisper-large-v3-turbo-q5km".into(),
        processing_mode: ProcessingMode::DuringRecording,
        ..Default::default()
    };
    assert!(matches!(
        resolve(&config),
        Err(ResolveError::NoLiveTranscription { .. })
    ));
}

#[test]
fn integrated_bundles_have_no_pyannote_dependency() {
    for id in [
        "multitalker-parakeet-streaming-0.6b-v1-q80",
        "granite-speech-4.1-2b-plus-q5km",
        "moss-transcribe-diarize-q80",
    ] {
        let config = RecognitionConfig {
            asr_model: id.into(),
            speaker_source: SpeakerSource::Integrated,
            ..Default::default()
        };
        let spec = resolve(&config).unwrap();
        assert_eq!(spec.artifacts.len(), 1);
        if id.starts_with("multitalker") {
            assert!(spec.artifacts[0].url.contains("/bundle/"));
        }
    }
}

#[test]
fn offline_text_can_have_live_separate_speaker_analysis() {
    let config = RecognitionConfig {
        asr_model: "whisper-large-v3-turbo-q5km".into(),
        speaker_source: SpeakerSource::Sortformer,
        speaker_processing_mode: ProcessingMode::DuringRecording,
        ..Default::default()
    };
    let spec = resolve(&config).unwrap();
    assert!(!spec.capabilities.live_transcription);
    assert!(!spec.enabled_live_transcription);
    assert!(spec.enabled_live_speaker_analysis);
    assert!(spec.capabilities.live_speaker_analysis);
    assert!(!spec.capabilities.live_attributed_text);
}

#[test]
fn one_family_uses_distinct_artifacts_for_each_runtime() {
    use models::recognition::{AsrRuntime, resolve_for_runtime};
    let config = RecognitionConfig {
        asr_model: "nemotron-speech-streaming-en-0.6b-q80".into(),
        speaker_source: SpeakerSource::Nemotron,
        ..Default::default()
    };
    let separate = resolve(&config).unwrap();
    let combined = resolve_for_runtime(&config, AsrRuntime::Nemo).unwrap();
    assert_ne!(separate.artifacts[0].sha256, combined.artifacts[0].sha256);
    assert_eq!(separate.artifacts[0].format, "transcribe-gguf");
    assert_eq!(combined.artifacts[0].format, "nemo-gguf");
    assert_eq!(combined.runtimes.len(), 1);
}

#[test]
fn native_after_stop_accepts_live_preview_but_does_not_claim_live_labels() {
    let mut config = RecognitionConfig {
        asr_model: "multitalker-parakeet-streaming-0.6b-v1-q80".into(),
        speaker_source: SpeakerSource::Integrated,
        processing_mode: ProcessingMode::DuringRecording,
        ..Default::default()
    };
    assert!(resolve(&config).is_ok());
    config.speaker_processing_mode = ProcessingMode::DuringRecording;
    assert!(resolve(&config).is_err());
    config.speaker_source = SpeakerSource::Sortformer;
    assert!(resolve(&config).is_ok());
}

#[test]
fn live_nemotron_choices_select_the_combined_runtime_and_attributed_capability() {
    for id in [
        "nemotron-speech-streaming-en-0.6b-q80",
        "nemotron-3.5-asr-streaming-0.6b-q80",
    ] {
        for speaker_source in [SpeakerSource::Sortformer, SpeakerSource::Nemotron] {
            let config = RecognitionConfig {
                asr_model: id.into(),
                speaker_source,
                processing_mode: ProcessingMode::DuringRecording,
                speaker_processing_mode: ProcessingMode::DuringRecording,
                ..Default::default()
            };
            let spec = resolve(&config).unwrap();
            assert_eq!(spec.artifacts[0].format, "nemo-gguf");
            assert!(spec.capabilities.live_attributed_text);
            assert_eq!(spec.runtimes.len(), 1);
            assert_eq!(spec.config, config);
        }
    }
}

#[test]
fn explicit_transcribe_comparison_keeps_its_artifact_even_for_live_nemotron() {
    use models::recognition::{AsrRuntime, resolve_for_runtime};
    let config = RecognitionConfig {
        asr_model: "nemotron-speech-streaming-en-0.6b-q80".into(),
        speaker_source: SpeakerSource::Nemotron,
        processing_mode: ProcessingMode::DuringRecording,
        speaker_processing_mode: ProcessingMode::DuringRecording,
        ..Default::default()
    };
    let spec = resolve_for_runtime(&config, AsrRuntime::Transcribe).unwrap();
    assert_eq!(spec.artifacts[0].format, "transcribe-gguf");
    assert!(!spec.capabilities.live_attributed_text);
}

#[test]
fn live_nemotron_without_a_nemo_speaker_model_keeps_the_transcribe_route() {
    for speaker_source in [SpeakerSource::Off, SpeakerSource::Pyannote] {
        let config = RecognitionConfig {
            asr_model: "nemotron-speech-streaming-en-0.6b-q80".into(),
            speaker_source,
            processing_mode: ProcessingMode::DuringRecording,
            ..Default::default()
        };
        let spec = resolve(&config).unwrap();
        assert_eq!(spec.artifacts[0].format, "transcribe-gguf");
        assert!(!spec.capabilities.live_attributed_text);
    }
}
