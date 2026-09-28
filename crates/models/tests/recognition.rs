use models::recognition::{
    ProcessingMode, RecognitionConfig, ResolveError, SpeakerCount, SpeakerSource, bundle, resolve,
};

fn bundle_ids(asr_model: &str) -> Vec<String> {
    bundle(asr_model).into_iter().map(|a| a.id).collect()
}

#[test]
fn a_bundle_holds_every_speaker_model_that_a_choice_can_use() {
    assert_eq!(
        bundle_ids("parakeet-unified-en-0.6b-q80"),
        [
            "parakeet-unified-en-0.6b-q80",
            "pyannote-segmentation-3.0",
            "wespeaker-voxceleb-resnet34-lm",
            "sortformer-v2-q80",
            "nemotron-3-diarization-q80",
        ]
    );
}

#[test]
fn a_nemotron_streaming_bundle_holds_its_nemo_copy() {
    let ids = bundle_ids("nemotron-speech-streaming-en-0.6b-q80");
    assert_eq!(ids[0], "nemotron-speech-streaming-en-0.6b-q80");
    assert!(ids.contains(&"nemo-nemotron-speech-streaming-en-0.6b-q80".to_string()));
    assert!(
        !bundle_ids("parakeet-tdt-0.6b-v3-q80")
            .contains(&"nemo-parakeet-tdt-0.6b-v3-q80".to_string())
    );
}

#[test]
fn a_bundle_has_no_duplicates_and_an_unknown_model_has_none() {
    let ids = bundle_ids("moss-transcribe-diarize-q80");
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ids.len(), unique.len());
    assert!(bundle_ids("pyannote-segmentation-3.0").is_empty());
    assert!(bundle_ids("no-such-model").is_empty());
}

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
fn every_model_takes_speaker_labels_from_a_separate_speaker_model() {
    for model in models::CATALOG
        .iter()
        .filter(|m| m.kind == models::ModelKind::Asr)
    {
        for speaker_source in [
            SpeakerSource::Pyannote,
            SpeakerSource::Sortformer,
            SpeakerSource::Nemotron,
        ] {
            let spec = resolve(&RecognitionConfig {
                asr_model: model.id.clone(),
                speaker_source,
                ..Default::default()
            })
            .unwrap();
            assert!(spec.artifacts.len() > 1, "{}: {speaker_source:?}", model.id);
        }
        let off = resolve(&RecognitionConfig {
            asr_model: model.id.clone(),
            speaker_source: SpeakerSource::Off,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(off.artifacts.len(), 1, "{}", model.id);
    }
}

#[test]
fn stored_spec_with_model_speaker_labels_does_not_read() {
    let mut value = serde_json::to_value(resolve(&RecognitionConfig::default()).unwrap()).unwrap();
    value["config"]["speaker_source"] = "integrated".into();
    assert!(serde_json::from_value::<models::recognition::ExecutionSpec>(value).is_err());
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
fn after_stop_speakers_accept_live_preview_but_do_not_claim_live_labels() {
    let mut config = RecognitionConfig {
        asr_model: "parakeet-unified-en-0.6b-q80".into(),
        speaker_source: SpeakerSource::Pyannote,
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
