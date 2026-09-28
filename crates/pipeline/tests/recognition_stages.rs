mod common;
use asr::{
    AsrError, AsrSegment, Finality, Recognizer, SampleRange, SpeakerKey, TimingPrecision,
    WindowText,
};
use models::recognition::{ExecutionSpec, RecognitionConfig, SpeakerSource, resolve};
use pipeline::{EndpointProblem, EngineProvider, SummaryEndpoint, Unattended, checkpoint};
use session_core::Stage;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

const MODEL: &str = "whisper-large-v3-turbo-q5km";

struct Fake {
    calls: Arc<AtomicUsize>,
    native_tokens: bool,
}
impl Recognizer for Fake {
    fn model_id(&self) -> &str {
        MODEL
    }
    fn native_token_timestamps(&self) -> bool {
        self.native_tokens
    }
    fn transcribe_tokens(&mut self, pcm: &[f32]) -> Result<Vec<asr::TimedToken>, AsrError> {
        assert!(
            self.native_tokens,
            "synthetic fallback must not be used as native tokens"
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        let end = pcm.len() as u64 / 16;
        assert!(end > 3000, "speaker cuts removed required ASR context");
        Ok(vec![
            asr::TimedToken {
                text: " First".into(),
                start_ms: 0,
                end_ms: end / 2,
            },
            asr::TimedToken {
                text: " Second".into(),
                start_ms: end / 2,
                end_ms: end,
            },
        ])
    }
    fn transcribe(&mut self, pcm: &[f32]) -> Result<WindowText, AsrError> {
        assert!(
            !self.native_tokens,
            "native token-capable route used segment-only inference"
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(WindowText {
            text: "Hello there".into(),
            segments: vec![AsrSegment {
                start_ms: 0,
                end_ms: pcm.len() as u64 / 16,
                text: "Hello there".into(),
            }],
        })
    }
}
struct Provider {
    calls: Arc<AtomicUsize>,
    loads: Arc<AtomicUsize>,
    fail_speaker: bool,
    native_tokens: bool,
}
impl EngineProvider for Provider {
    fn nemo_helper(&self) -> Result<PathBuf, String> {
        Ok("/synthetic/no-helper-must-run".into())
    }
    fn artifact_path(&self, _: &models::recognition::ArtifactSpec) -> Result<PathBuf, String> {
        Ok("/synthetic/model".into())
    }
    fn load_exact(&self, _: &ExecutionSpec) -> Result<Box<dyn Recognizer>, String> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Fake {
            calls: self.calls.clone(),
            native_tokens: self.native_tokens,
        }))
    }
    fn analyzer_exact(
        &self,
        _: &ExecutionSpec,
    ) -> Result<Box<dyn diarize::SpeakerAnalyzer>, diarize::DiarizeError> {
        if self.fail_speaker {
            Err(diarize::DiarizeError::Format(
                "speaker inference failed".into(),
            ))
        } else {
            panic!("separate analyzer forbidden")
        }
    }
    fn ready(&self, _: &str) -> bool {
        true
    }
    fn ready_exact(&self, _: &ExecutionSpec) -> bool {
        true
    }
    fn load(&self, _: &str) -> Result<Box<dyn Recognizer>, String> {
        panic!("legacy model loading forbidden")
    }
    fn default_model(&self) -> String {
        panic!("default model forbidden")
    }
    fn analyzer(&self) -> Result<Box<dyn diarize::SpeakerAnalyzer>, diarize::DiarizeError> {
        panic!("legacy analyzer forbidden")
    }
    fn summary_endpoint(&self) -> Result<SummaryEndpoint, EndpointProblem> {
        Err(EndpointProblem::Unconfigured("not used".into()))
    }
    fn summary_prompt(&self) -> pipeline::summarize::Prompt {
        summarize::prompts::built_in(summarize::prompts::TOPIC_SUMMARY_ID).unwrap()
    }
    fn learn_limit(&self, _: &str, _: &str, _: u32) {}
    fn destination(&self) -> Option<PathBuf> {
        None
    }
}
fn spec(source: SpeakerSource) -> ExecutionSpec {
    resolve(&RecognitionConfig {
        asr_model: MODEL.into(),
        speaker_source: source,
        ..Default::default()
    })
    .unwrap()
}
fn setup(
    source: SpeakerSource,
) -> (
    tempfile::TempDir,
    pipeline::SharedStore,
    ExecutionSpec,
    Vec<session_core::Segment>,
) {
    setup_audio(source, common::speech())
}
fn setup_audio(
    source: SpeakerSource,
    audio: Vec<f32>,
) -> (
    tempfile::TempDir,
    pipeline::SharedStore,
    ExecutionSpec,
    Vec<session_core::Segment>,
) {
    let temp = tempfile::tempdir().unwrap();
    let store = common::store();
    let spec = spec(source);
    common::queue_session(
        &store,
        temp.path(),
        "route",
        100,
        Some(&spec.config.asr_model),
        &[(16000, audio)],
    );
    store
        .lock()
        .unwrap()
        .set_recognition_snapshot("route", &serde_json::to_value(&spec).unwrap())
        .unwrap();
    let segments = store.lock().unwrap().segments("route").unwrap();
    pipeline::assemble::run(
        &store,
        "route",
        &temp.path().join("route"),
        &segments,
        None,
        &Unattended,
    )
    .unwrap();
    pipeline::recognition::durable::prepare(
        &store,
        "route",
        &temp.path().join("route"),
        &segments,
        &spec,
    )
    .unwrap();
    pipeline::checkpoint::put(
        &store,
        "route",
        Stage::Transcribe,
        "recognition-generation-v1",
        &"speaker-analysis",
    )
    .unwrap();
    (temp, store, spec, segments)
}
/// Save one completed speaker turn for each segment. The tests then need no
/// speaker model.
fn seed_speakers(
    store: &pipeline::SharedStore,
    segments: &[session_core::Segment],
    epoch: u64,
) -> Vec<asr::SpeakerTurn> {
    let turns = segments
        .iter()
        .map(|segment| asr::SpeakerTurn {
            range: SampleRange {
                start: segment.offset_ms * 16,
                end: (segment.offset_ms + segment.duration_ms) * 16,
            },
            timing: TimingPrecision::NativeSegment,
            speaker: SpeakerKey {
                scope: asr::SpeakerScope {
                    generation: "speaker-analysis".into(),
                    epoch,
                },
                input_start_sample: 0,
                local_id: 1,
            },
            finality: Finality::Final,
        })
        .collect::<Vec<_>>();
    pipeline::recognition::durable::persist_speakers(store, "route", &turns).unwrap();
    turns
}
#[test]
fn off_and_separate_speakers_execute_without_speaker_inference() {
    for source in [SpeakerSource::Off, SpeakerSource::Nemotron] {
        let (temp, store, spec, segments) = setup(source);
        if source == SpeakerSource::Nemotron {
            seed_speakers(&store, &segments, 0);
        }
        let provider = Provider {
            calls: Arc::new(AtomicUsize::new(0)),
            loads: Arc::new(AtomicUsize::new(0)),
            fail_speaker: false,
            native_tokens: false,
        };
        for stage in [Stage::Transcribe, Stage::Join, Stage::Verify] {
            pipeline::recognition::execute(
                &store,
                "route",
                &temp.path().join("route"),
                &segments,
                &spec,
                &provider,
                stage,
                &Unattended,
                &mut |_| {},
            )
            .unwrap();
        }
        assert!(provider.calls.load(Ordering::SeqCst) > 0);
        let path = temp
            .path()
            .join("route")
            .join(pipeline::proc_file(&segments[0].audio_file));
        let audio = hound::WavReader::open(path)
            .unwrap()
            .into_samples::<f32>()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let mut voicing = asr::Voicing::new();
        voicing.push(&audio);
        let expected: u64 = voicing
            .frames()
            .split(|voiced| !*voiced)
            .filter(|run| run.len() >= 4)
            .map(|run| run.len() as u64 * 16)
            .sum();
        let coverage: pipeline::verify::Coverage =
            checkpoint::get(&store, "route", Stage::Verify, &checkpoint::segment_key(0))
                .unwrap()
                .unwrap();
        assert!(expected > 0);
        assert_eq!(
            coverage.voiced_ms, expected,
            "Verify must exclude request padding"
        );
        if source == SpeakerSource::Off {
            let text = std::fs::read_to_string(temp.path().join("route/transcript.md")).unwrap();
            assert!(text.contains("Speaker recognition was off"));
            assert!(!text.contains("failed"));
        }
        let result = pipeline::recognition::saved_result(&store, "route")
            .unwrap()
            .unwrap();
        assert_eq!(
            result.rows.iter().any(|r| r.speaker.is_some()),
            source == SpeakerSource::Nemotron
        );
    }
}
#[test]
fn off_reuses_successful_live_text_without_asr() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Off);
    let mut reader =
        pipeline::live::archive::ArchiveReader::new(pipeline::live::archive::Archive {
            dir: temp.path().join("route").join(pipeline::RECOVERY_DIR),
            segment: 0,
            offset_ms: segments[0].offset_ms,
            rate: 16000,
        });
    while reader.next_frame().unwrap().is_some() {}
    let mut config = pipeline::live::LiveConfig::new("route", 0, &spec.config.asr_model, 16000);
    config.snapshot = Some(spec.clone());
    pipeline::live::archive::save_proof(&store, &config, &reader).unwrap();
    pipeline::recognition::durable::stamp_live_identity(
        &store,
        "route",
        &temp.path().join("route"),
        &segments,
        &spec,
    )
    .unwrap();
    let end = segments[0].duration_ms;
    pipeline::live::save_part(
        &store,
        "route",
        0,
        0,
        &asr::LivePart {
            start_ms: 0,
            end_ms: end,
            words: vec![asr::LiveWord {
                start_ms: 0,
                end_ms: end,
                text: "Already transcribed".into(),
            }],
        },
    )
    .unwrap();
    let provider = Provider {
        calls: Arc::new(AtomicUsize::new(0)),
        loads: Arc::new(AtomicUsize::new(0)),
        fail_speaker: false,
        native_tokens: false,
    };
    pipeline::recognition::execute(
        &store,
        "route",
        &temp.path().join("route"),
        &segments,
        &spec,
        &provider,
        Stage::Transcribe,
        &Unattended,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        provider.loads.load(Ordering::SeqCst),
        0,
        "completed live text must not reload the model"
    );
}
#[tokio::test]
async fn explicit_speaker_failure_stops_before_summary_and_save() {
    let (temp, store, _, _) = setup(SpeakerSource::Pyannote);
    let runner = pipeline::Runner::new(pipeline::Environment {
        store: store.clone(),
        working_root: temp.path().into(),
        engines: Box::new(Provider {
            calls: Arc::new(AtomicUsize::new(0)),
            loads: Arc::new(AtomicUsize::new(0)),
            fail_speaker: true,
            native_tokens: false,
        }),
        suspended: Box::new(|| false),
        events: Box::new(|_| {}),
    });
    let (_, outcome) = runner.run_next().await.unwrap();
    assert!(matches!(
        outcome,
        pipeline::Outcome::Failed {
            stage: Stage::Diarize,
            ..
        }
    ));
    assert!(!temp.path().join("route/summary.md").exists());
    assert!(
        store
            .lock()
            .unwrap()
            .checkpoints("route", Stage::Save)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn separate_timeline_cuts_batch_asr_at_alternating_speakers() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Nemotron);
    let end = segments[0].duration_ms * 16;
    let mid = end / 2;
    let scope = asr::SpeakerScope {
        generation: "speaker-analysis".into(),
        epoch: 0,
    };
    let turns = vec![(0, mid, 1), (mid, end, 2)]
        .into_iter()
        .map(|(start, end, id)| asr::SpeakerTurn {
            range: SampleRange { start, end },
            timing: TimingPrecision::NativeSegment,
            speaker: SpeakerKey {
                scope: scope.clone(),
                input_start_sample: 0,
                local_id: id,
            },
            finality: Finality::Final,
        })
        .collect::<Vec<_>>();
    pipeline::recognition::durable::persist_speakers(&store, "route", &turns).unwrap();
    let provider = Provider {
        calls: Arc::new(AtomicUsize::new(0)),
        loads: Arc::new(AtomicUsize::new(0)),
        fail_speaker: false,
        native_tokens: false,
    };
    pipeline::recognition::execute(
        &store,
        "route",
        &temp.path().join("route"),
        &segments,
        &spec,
        &provider,
        Stage::Transcribe,
        &Unattended,
        &mut |_| {},
    )
    .unwrap();
    let result = pipeline::recognition::saved_result(&store, "route")
        .unwrap()
        .unwrap();
    assert!(provider.calls.load(Ordering::SeqCst) >= 2);
    assert!(
        result
            .rows
            .iter()
            .any(|r| r.speaker.as_ref().is_some_and(|s| s.local_id == 1))
    );
    assert!(
        result
            .rows
            .iter()
            .any(|r| r.speaker.as_ref().is_some_and(|s| s.local_id == 2))
    );
}

#[test]
fn native_token_route_keeps_context_and_joins_alternating_speakers() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Nemotron);
    let end = segments[0].duration_ms * 16;
    let scope = asr::SpeakerScope {
        generation: "speaker-analysis".into(),
        epoch: 0,
    };
    let turns = vec![(0, end / 2, 1), (end / 2, end, 2)]
        .into_iter()
        .map(|(start, end, id)| asr::SpeakerTurn {
            range: SampleRange { start, end },
            timing: TimingPrecision::NativeSegment,
            speaker: SpeakerKey {
                scope: scope.clone(),
                input_start_sample: 0,
                local_id: id,
            },
            finality: Finality::Final,
        })
        .collect::<Vec<_>>();
    pipeline::recognition::durable::persist_speakers(&store, "route", &turns).unwrap();
    let provider = Provider {
        calls: Arc::new(AtomicUsize::new(0)),
        loads: Arc::new(AtomicUsize::new(0)),
        fail_speaker: false,
        native_tokens: true,
    };
    pipeline::recognition::execute(
        &store,
        "route",
        &temp.path().join("route"),
        &segments,
        &spec,
        &provider,
        Stage::Transcribe,
        &Unattended,
        &mut |_| {},
    )
    .unwrap();
    let result = pipeline::recognition::saved_result(&store, "route")
        .unwrap()
        .unwrap();
    let baseline =
        pipeline::transcribe::plan_segment(&temp.path().join("route"), &segments[0], &[]).unwrap();
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        baseline.windows.len()
    );
    assert_eq!(result.rows.len(), 2 * baseline.windows.len());
    assert!(result.rows.iter().any(|row| {
        row.speaker
            .as_ref()
            .is_some_and(|speaker| speaker.local_id == 1)
    }));
    assert!(result.rows.iter().any(|row| {
        row.speaker
            .as_ref()
            .is_some_and(|speaker| speaker.local_id == 2)
    }));
}

#[test]
fn changed_preprocessing_invalidates_completed_transcription() {
    let (temp, store, mut spec, segments) = setup(SpeakerSource::Off);
    let provider = Provider {
        calls: Arc::new(AtomicUsize::new(0)),
        loads: Arc::new(AtomicUsize::new(0)),
        fail_speaker: false,
        native_tokens: false,
    };
    let dir = temp.path().join("route");
    let run = |spec: &ExecutionSpec| {
        pipeline::recognition::execute(
            &store,
            "route",
            &dir,
            &segments,
            spec,
            &provider,
            Stage::Transcribe,
            &Unattended,
            &mut |_| {},
        )
        .unwrap()
    };
    run(&spec);
    let calls = provider.calls.load(Ordering::SeqCst);
    let loads = provider.loads.load(Ordering::SeqCst);
    run(&spec);
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
    assert_eq!(
        provider.loads.load(Ordering::SeqCst),
        loads,
        "completed text must not reload the model"
    );
    spec.preprocessing_revision.push_str("-changed");
    run(&spec);
    assert!(provider.calls.load(Ordering::SeqCst) > calls);
}

#[test]
fn join_writes_portable_output_referenced_by_the_manifest() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Off);
    let provider = Provider {
        calls: Arc::new(AtomicUsize::new(0)),
        loads: Arc::new(AtomicUsize::new(0)),
        fail_speaker: false,
        native_tokens: false,
    };
    let dir = temp.path().join("route");
    for stage in [Stage::Transcribe, Stage::Join] {
        pipeline::recognition::execute(
            &store,
            "route",
            &dir,
            &segments,
            &spec,
            &provider,
            stage,
            &Unattended,
            &mut |_| {},
        )
        .unwrap();
    }
    let record = store.lock().unwrap().session("route").unwrap().unwrap();
    let doc = pipeline::save::session_doc(&record, &segments, &dir).unwrap();
    let value = serde_json::to_value(doc).unwrap();
    assert!(value["recognition_artifact"]["sha256"].as_str().is_some());
    assert!(dir.join("recognition.jsonl").is_file());
}

#[test]
fn unstamped_live_text_requires_an_explicit_retranscription_choice() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Off);
    pipeline::live::save_part(
        &store,
        "route",
        0,
        0,
        &asr::LivePart {
            start_ms: 0,
            end_ms: segments[0].duration_ms,
            words: vec![asr::LiveWord {
                start_ms: 0,
                end_ms: segments[0].duration_ms,
                text: "Already transcribed".into(),
            }],
        },
    )
    .unwrap();
    let provider = Provider {
        calls: Arc::new(AtomicUsize::new(0)),
        loads: Arc::new(AtomicUsize::new(0)),
        fail_speaker: false,
        native_tokens: false,
    };
    let error = pipeline::recognition::execute(
        &store,
        "route",
        &temp.path().join("route"),
        &segments,
        &spec,
        &provider,
        Stage::Transcribe,
        &Unattended,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(error.to_string().contains("Transcribe Again"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn audio_changes_and_external_transcript_edits_block_stale_reuse() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Off);
    let provider = Provider {
        calls: Arc::new(AtomicUsize::new(0)),
        loads: Arc::new(AtomicUsize::new(0)),
        fail_speaker: false,
        native_tokens: false,
    };
    let dir = temp.path().join("route");
    let run = |stage| {
        pipeline::recognition::execute(
            &store,
            "route",
            &dir,
            &segments,
            &spec,
            &provider,
            stage,
            &Unattended,
            &mut |_| {},
        )
    };
    run(Stage::Transcribe).unwrap();
    let calls = provider.calls.load(Ordering::SeqCst);
    let path = dir.join(pipeline::proc_file(&segments[0].audio_file));
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(run(Stage::Transcribe).is_err());
    pipeline::assemble::run(&store, "route", &dir, &segments, None, &Unattended).unwrap();
    run(Stage::Transcribe).unwrap();
    assert!(provider.calls.load(Ordering::SeqCst) > calls);
    run(Stage::Join).unwrap();
    std::fs::write(dir.join("transcript.md"), "External edit").unwrap();
    assert!(
        run(Stage::Verify)
            .unwrap_err()
            .to_string()
            .contains("Transcript changed")
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("transcript.md")).unwrap(),
        "External edit"
    );
}

#[test]
fn current_count_edit_retains_timed_asr_but_invalidates_speakers() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Nemotron);
    let dir = temp.path().join("route");
    let turns = vec![asr::SpeakerTurn {
        range: SampleRange {
            start: 0,
            end: segments[0].duration_ms * 16,
        },
        timing: TimingPrecision::NativeSegment,
        speaker: SpeakerKey {
            scope: asr::SpeakerScope {
                generation: "speaker-analysis".into(),
                epoch: 0,
            },
            input_start_sample: 0,
            local_id: 1,
        },
        finality: Finality::Final,
    }];
    let seed =
        || pipeline::recognition::durable::persist_speakers(&store, "route", &turns).unwrap();
    seed();
    let provider = Provider {
        calls: Arc::new(AtomicUsize::new(0)),
        loads: Arc::new(AtomicUsize::new(0)),
        fail_speaker: false,
        native_tokens: true,
    };
    let run = || {
        pipeline::recognition::execute(
            &store,
            "route",
            &dir,
            &segments,
            &spec,
            &provider,
            Stage::Transcribe,
            &Unattended,
            &mut |_| {},
        )
        .unwrap()
    };
    run();
    let calls = provider.calls.load(Ordering::SeqCst);
    store
        .lock()
        .unwrap()
        .set_speaker_count("route", Some(session_core::SpeakerCount::Exact(2)))
        .unwrap();
    pipeline::recognition::durable::prepare(&store, "route", &dir, &segments, &spec).unwrap();
    assert!(
        pipeline::checkpoint::get::<serde_json::Value>(
            &store,
            "route",
            Stage::Diarize,
            pipeline::recognition::SPEAKERS_KEY
        )
        .unwrap()
        .is_none()
    );
    seed();
    run();
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
}

#[test]
fn changed_archive_cannot_reuse_a_stale_processing_file() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Off);
    let dir = temp.path().join("route");
    pipeline::recognition::durable::prepare(&store, "route", &dir, &segments, &spec).unwrap();
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(&segments[0].audio_file))
        .unwrap()
        .write_all(b"external archive change")
        .unwrap();
    let error = pipeline::recognition::durable::prepare(&store, "route", &dir, &segments, &spec)
        .unwrap_err();
    assert!(error.to_string().contains("audio"));
}

#[test]
fn portable_reuse_requires_unchanged_transcript_and_suitable_timing() {
    use session_core::recognition::{ARTIFACT_FILE, Reuse, read_artifact, write_artifact};
    let (temp, store, spec, segments) = setup(SpeakerSource::Nemotron);
    seed_speakers(&store, &segments, 0);
    let dir = temp.path().join("route");
    let provider = Provider {
        calls: Arc::new(AtomicUsize::new(0)),
        loads: Arc::new(AtomicUsize::new(0)),
        fail_speaker: false,
        native_tokens: false,
    };
    for stage in [Stage::Transcribe, Stage::Join] {
        pipeline::recognition::execute(
            &store,
            "route",
            &dir,
            &segments,
            &spec,
            &provider,
            stage,
            &Unattended,
            &mut |_| {},
        )
        .unwrap();
    }
    let record = store.lock().unwrap().session("route").unwrap().unwrap();
    let reference = pipeline::save::session_doc(&record, &segments, &dir)
        .unwrap()
        .recognition_artifact
        .unwrap();
    let mut next =
        pipeline::recognition::durable::fingerprints(&store, "route", &dir, &segments, &spec)
            .unwrap();
    next.speakers = "new-speaker-model".into();
    next.execution = "new-execution".into();
    let mut count = 0;
    assert_eq!(
        pipeline::recognition::durable::reusable_artifact(&dir, Some(&reference), &next, |b| {
            count += b.rows.len();
            assert!(b.rows.iter().all(|r| r.speaker.is_none()));
            assert!(b.coverage.committed.is_none());
            Ok(())
        })
        .unwrap(),
        Reuse::TimedAsr
    );
    assert!(count > 0);
    let mut blocks = vec![];
    let header = read_artifact(&dir.join(ARTIFACT_FILE), &reference, |mut block| {
        for row in &mut block.rows {
            row.timing = TimingPrecision::SourceWindow;
        }
        blocks.push(block);
        Ok(())
    })
    .unwrap();
    let untimed = write_artifact(&dir.join(ARTIFACT_FILE), &header, blocks).unwrap();
    assert_eq!(
        pipeline::recognition::durable::reusable_artifact(&dir, Some(&untimed), &next, |_| panic!(
            "source-window text cannot be reused for speaker assignment"
        ))
        .unwrap(),
        Reuse::TranscribeAgain
    );
    std::fs::write(dir.join("transcript.md"), "Edited externally").unwrap();
    assert_eq!(
        pipeline::recognition::durable::reusable_artifact(&dir, Some(&untimed), &next, |_| panic!(
            "external text cannot be reused"
        ))
        .unwrap(),
        Reuse::TranscribeAgain
    );
    assert_eq!(
        pipeline::recognition::durable::reusable_artifact(&dir, None, &next, |_| panic!(
            "legacy note needs explicit retranscription"
        ))
        .unwrap(),
        Reuse::TranscribeAgain
    );
}

#[test]
fn live_identity_requires_recorded_source_proof_not_only_final_audio_hashes() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Off);
    assert!(
        pipeline::recognition::durable::stamp_live_identity(
            &store,
            "route",
            &temp.path().join("route"),
            &segments,
            &spec,
        )
        .is_err()
    );
}

#[test]
fn completed_live_prefix_survives_failure_in_the_next_epoch() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Off);
    let dir = temp.path().join("route");
    let mut reader =
        pipeline::live::archive::ArchiveReader::new(pipeline::live::archive::Archive {
            dir: dir.join(pipeline::RECOVERY_DIR),
            segment: 0,
            offset_ms: segments[0].offset_ms,
            rate: 16000,
        });
    while reader.position() < 16000 {
        assert!(reader.next_frame().unwrap().is_some());
    }
    let mut config = pipeline::live::LiveConfig::new("route", 0, &spec.config.asr_model, 16000);
    config.snapshot = Some(spec.clone());
    pipeline::live::archive::save_proof(&store, &config, &reader).unwrap();
    let key = format!("recognition-epoch-{:020}", segments[0].offset_ms * 16);
    pipeline::checkpoint::put(&store, "route", Stage::Diarize, &key, &serde_json::json!({"range":{"start":segments[0].offset_ms*16,"end":segments[0].offset_ms*16+16000}})).unwrap();
    pipeline::recognition::durable::prepare(&store, "route", &dir, &segments, &spec).unwrap();
    assert!(
        pipeline::checkpoint::get::<serde_json::Value>(&store, "route", Stage::Diarize, &key)
            .unwrap()
            .is_some(),
        "a verified completed epoch must survive a later stream failure"
    );
}

#[test]
fn changed_stereo_preprocessing_invalidates_live_text_and_labels_with_a_reason() {
    let temp = tempfile::tempdir().unwrap();
    let store = common::store();
    store
        .lock()
        .unwrap()
        .create_session("stereo", common::at(0), None)
        .unwrap();
    let spec = spec(SpeakerSource::Off);
    store
        .lock()
        .unwrap()
        .set_recognition_snapshot("stereo", &serde_json::to_value(&spec).unwrap())
        .unwrap();
    let audio = common::speech();
    let segment = common::write_stereo_segment(temp.path(), 0, 0, 16000, &audio, &audio, false);
    store
        .lock()
        .unwrap()
        .put_segment("stereo", &segment)
        .unwrap();
    let mut reader =
        pipeline::live::archive::ArchiveReader::new(pipeline::live::archive::Archive {
            dir: temp.path().join(pipeline::RECOVERY_DIR),
            segment: 0,
            offset_ms: 0,
            rate: 16000,
        });
    while reader.next_frame().unwrap().is_some() {}
    let mut config = pipeline::live::LiveConfig::new("stereo", 0, &spec.config.asr_model, 16000);
    config.snapshot = Some(spec.clone());
    pipeline::live::archive::save_proof(&store, &config, &reader).unwrap();
    pipeline::live::save_part(
        &store,
        "stereo",
        0,
        0,
        &asr::LivePart {
            start_ms: 0,
            end_ms: 1000,
            words: vec![],
        },
    )
    .unwrap();
    let key = "recognition-epoch-00000000000000000000";
    pipeline::checkpoint::put(
        &store,
        "stereo",
        Stage::Diarize,
        key,
        &serde_json::json!({"range":{"start":0,"end":16000}}),
    )
    .unwrap();
    pipeline::assemble::run(
        &store,
        "stereo",
        temp.path(),
        std::slice::from_ref(&segment),
        None,
        &Unattended,
    )
    .unwrap();
    pipeline::recognition::durable::prepare(
        &store,
        "stereo",
        temp.path(),
        std::slice::from_ref(&segment),
        &spec,
    )
    .unwrap();
    assert!(pipeline::live::rows(&store, "stereo").unwrap().is_empty());
    assert!(
        pipeline::checkpoint::get::<serde_json::Value>(&store, "stereo", Stage::Diarize, key)
            .unwrap()
            .is_none()
    );
    let reasons: Vec<String> = pipeline::checkpoint::get(
        &store,
        "stereo",
        Stage::Transcribe,
        "live-invalidation-reasons",
    )
    .unwrap()
    .unwrap();
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("preprocessing"))
    );
    checkpoint::put(
        &store,
        "stereo",
        Stage::Diarize,
        key,
        &serde_json::json!({"range":{"start":0,"end":16000}, "recomputed":true}),
    )
    .unwrap();
    pipeline::recognition::durable::prepare(&store, "stereo", temp.path(), &[segment], &spec)
        .unwrap();
    assert!(
        checkpoint::get::<serde_json::Value>(&store, "stereo", Stage::Diarize, key)
            .unwrap()
            .is_some(),
        "later prepare must retain recognition recomputed from cleaned audio"
    );
}

#[test]
fn prepare_preserves_partial_feature_windows_for_pcm_verification() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Off);
    store
        .lock()
        .unwrap()
        .delete_checkpoint(
            "route",
            Stage::Transcribe,
            pipeline::recognition::durable::IDENTITY_KEY,
        )
        .unwrap();
    let key = "live-feature-00000000-000000000000";
    checkpoint::put(
        &store,
        "route",
        Stage::Diarize,
        key,
        &serde_json::json!({"digest":"must-be-verified", "file":"window.diar", "step":31860}),
    )
    .unwrap();
    pipeline::recognition::durable::prepare(
        &store,
        "route",
        &temp.path().join("route"),
        &segments,
        &spec,
    )
    .unwrap();
    assert!(
        checkpoint::get::<serde_json::Value>(&store, "route", Stage::Diarize, key)
            .unwrap()
            .is_some(),
        "preserve individually hashed features for bounded verification at normal entry"
    );
}

#[test]
fn live_identity_accepts_resampler_rounded_final_sample() {
    let temp = tempfile::tempdir().unwrap();
    let store = common::store();
    let spec = spec(SpeakerSource::Off);
    common::queue_session(
        &store,
        temp.path(),
        "route",
        100,
        Some(&spec.config.asr_model),
        &[(48000, vec![0.1; 48002])],
    );
    let segments = store.lock().unwrap().segments("route").unwrap();
    let dir = temp.path().join("route");
    pipeline::assemble::run(&store, "route", &dir, &segments, None, &Unattended).unwrap();
    let mut reader =
        pipeline::live::archive::ArchiveReader::new(pipeline::live::archive::Archive {
            dir: dir.join(pipeline::RECOVERY_DIR),
            segment: 0,
            offset_ms: segments[0].offset_ms,
            rate: 48000,
        });
    while reader.next_frame().unwrap().is_some() {}
    assert_eq!(reader.proof().samples, 48002);
    let mut config = pipeline::live::LiveConfig::new("route", 0, &spec.config.asr_model, 48000);
    config.snapshot = Some(spec.clone());
    pipeline::live::archive::save_proof(&store, &config, &reader).unwrap();
    let start = segments[0].offset_ms * 16;
    checkpoint::put(
        &store,
        "route",
        Stage::Transcribe,
        &format!("recognition-epoch-{start:020}"),
        &serde_json::json!({"range":{"start":start,"end":start+16001}}),
    )
    .unwrap();
    pipeline::recognition::durable::stamp_live_identity(&store, "route", &dir, &segments, &spec)
        .unwrap();
    checkpoint::put(
        &store,
        "route",
        Stage::Transcribe,
        &format!("recognition-epoch-{start:020}"),
        &serde_json::json!({"range":{"start":start,"end":start+16002}}),
    )
    .unwrap();
    assert!(
        pipeline::recognition::durable::stamp_live_identity(
            &store, "route", &dir, &segments, &spec
        )
        .is_err()
    );
}

struct SavedResume {
    _temp: tempfile::TempDir,
    store: pipeline::SharedStore,
    spec: ExecutionSpec,
    segments: Vec<session_core::Segment>,
    dir: PathBuf,
    home: PathBuf,
    provider: Provider,
    original: pipeline::recognition::RouteResult,
}
impl SavedResume {
    /// The number of model calls that the appended audio needs.
    fn appended_windows(&self) -> usize {
        pipeline::transcribe::plan_segment(&self.dir, &self.segments[1], &[])
            .unwrap()
            .windows
            .len()
    }
    /// Save the completed speaker analysis of the resumed note. The appended
    /// audio gets its own speaker scope, as a new analysis gives.
    fn seed_appended_speakers(&self) {
        pipeline::recognition::durable::prepare(
            &self.store,
            "route",
            &self.dir,
            &self.segments,
            &self.spec,
        )
        .unwrap();
        let mut turns = self.original.speakers.clone();
        turns.extend(seed_speakers(&self.store, &self.segments[1..], 1));
        pipeline::recognition::durable::persist_speakers(&self.store, "route", &turns).unwrap();
    }
}
fn saved_resume_case(tail: &[f32]) -> SavedResume {
    saved_resume_audio(&common::speech(), tail)
}
fn saved_resume_audio(old: &[f32], tail: &[f32]) -> SavedResume {
    let (temp, store, spec, mut segments) = setup_audio(SpeakerSource::Nemotron, old.to_vec());
    let saved_turns = seed_speakers(&store, &segments, 0);
    let dir = temp.path().join("route");
    let home = temp.path().join("saved");
    let provider = Provider {
        calls: Arc::new(AtomicUsize::new(0)),
        loads: Arc::new(AtomicUsize::new(0)),
        fail_speaker: false,
        native_tokens: false,
    };
    for stage in [Stage::Transcribe, Stage::Join, Stage::Verify] {
        pipeline::recognition::execute(
            &store,
            "route",
            &dir,
            &segments,
            &spec,
            &provider,
            stage,
            &Unattended,
            &mut |_| {},
        )
        .unwrap();
    }
    let original = pipeline::recognition::saved_result(&store, "route")
        .unwrap()
        .unwrap();
    assert!(!original.rows.is_empty());
    assert!(original.rows.iter().all(|row| row.speaker.is_some()));
    let record = store.lock().unwrap().session("route").unwrap().unwrap();
    let doc = pipeline::save::session_doc(&record, &segments, &dir).unwrap();
    assert!(doc.recognition_artifact.is_some());
    assert_eq!(doc.recognition_audio.as_ref().unwrap().len(), 1);
    // Save retains the archive and neutral artifact, but removes processing PCM.
    std::fs::create_dir_all(home.join("audio")).unwrap();
    for filename in [
        "transcript.md",
        "recognition.jsonl",
        &segments[0].audio_file,
    ] {
        std::fs::copy(dir.join(filename), home.join(filename)).unwrap();
    }
    std::fs::write(home.join("session.md"), doc.render()).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    {
        let mut guard = store.lock().unwrap();
        guard
            .set_home_path("route", home.to_str().unwrap())
            .unwrap();
        guard
            .apply("route", &session_core::Event::Dequeued)
            .unwrap();
        guard.apply("route", &session_core::Event::Saved).unwrap();
        guard
            .apply(
                "route",
                &session_core::Event::UserResume {
                    space_ok: true,
                    audio_intact: true,
                },
            )
            .unwrap();
    }
    let appended = common::write_segment(&dir, 1, segments[0].end_ms() + 5_000, 16000, tail, false);
    store
        .lock()
        .unwrap()
        .put_segment("route", &appended)
        .unwrap();
    segments.push(appended.clone());
    common::stop_session(&store, "route", 200);
    pipeline::assemble::run(&store, "route", &dir, &segments, Some(&home), &Unattended).unwrap();
    assert_eq!(saved_turns, original.speakers);
    SavedResume {
        _temp: temp,
        store,
        spec,
        segments,
        dir,
        home,
        provider,
        original,
    }
}

#[test]
fn resume_reuses_verified_saved_prefix_and_recognizes_only_appended_audio() {
    let case = saved_resume_case(&common::speech());
    case.seed_appended_speakers();
    let SavedResume {
        store,
        spec,
        segments,
        dir,
        provider,
        original,
        ..
    } = &case;
    let appended = &segments[1];
    provider.calls.store(0, Ordering::SeqCst);
    pipeline::recognition::execute(
        store,
        "route",
        dir,
        segments,
        spec,
        provider,
        Stage::Transcribe,
        &Unattended,
        &mut |_| {},
    )
    .unwrap();
    assert!(
        case.appended_windows() > 0,
        "the new recording must be recognized"
    );
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        case.appended_windows(),
        "completed saved audio was recognized again",
    );
    let combined = pipeline::recognition::saved_result(store, "route")
        .unwrap()
        .unwrap();
    assert!(
        combined
            .rows
            .iter()
            .any(|row| row.range.start >= appended.offset_ms * 16 && row.text == "Hello there"),
        "spoken appended audio needs new text"
    );
    for stage in [Stage::Join, Stage::Verify] {
        pipeline::recognition::execute(
            store,
            "route",
            dir,
            segments,
            spec,
            provider,
            stage,
            &Unattended,
            &mut |_| {},
        )
        .unwrap();
    }
    let record = store.lock().unwrap().session("route").unwrap().unwrap();
    let doc = pipeline::save::session_doc(&record, segments, dir).unwrap();
    let mut appended_rows = 0;
    session_core::recognition::read_artifact(
        &dir.join("recognition.jsonl"),
        doc.recognition_artifact.as_ref().unwrap(),
        |block| {
            appended_rows += block
                .rows
                .iter()
                .filter(|row| {
                    row.range.start >= appended.offset_ms * 16 && row.text == "Hello there"
                })
                .count();
            Ok(())
        },
    )
    .unwrap();
    assert!(appended_rows > 0);
    let old_text = std::fs::read_to_string(case.home.join("transcript.md")).unwrap();
    let new_text = std::fs::read_to_string(dir.join("transcript.md")).unwrap();
    assert!(new_text.matches("Hello there").count() > old_text.matches("Hello there").count());
    assert!(
        combined
            .coverage_limitations
            .iter()
            .any(|note| note == pipeline::recognition::SCOPED_LABELS_NOTE),
        "merging independent speaker scopes must retain their limitation"
    );
    let retained: Vec<_> = combined
        .rows
        .iter()
        .filter(|row| row.range.start < appended.offset_ms * 16)
        .cloned()
        .collect();
    assert_eq!(
        retained, original.rows,
        "saved text, timing, and speaker scopes must stay unchanged"
    );
    assert_eq!(
        combined
            .speakers
            .iter()
            .filter(|turn| turn.range.start < appended.offset_ms * 16)
            .cloned()
            .collect::<Vec<_>>(),
        original.speakers
    );
    let mut expected = original.coverage.clone();
    let samples = hound::WavReader::open(dir.join(pipeline::proc_file(&appended.audio_file)))
        .unwrap()
        .duration() as u64;
    expected.push(SampleRange {
        start: appended.offset_ms * 16,
        end: appended.offset_ms * 16 + samples,
    });
    assert_eq!(
        combined.coverage, expected,
        "combined coverage must include old and new recordings"
    );
}

#[test]
fn resumed_silent_tail_preserves_saved_words_without_model_inference() {
    let silence = vec![0.0; 16000 * 5];
    let case = saved_resume_case(&silence);
    case.seed_appended_speakers();
    case.provider.calls.store(0, Ordering::SeqCst);
    for stage in [Stage::Transcribe, Stage::Join, Stage::Verify] {
        pipeline::recognition::execute(
            &case.store,
            "route",
            &case.dir,
            &case.segments,
            &case.spec,
            &case.provider,
            stage,
            &Unattended,
            &mut |_| {},
        )
        .unwrap();
    }
    assert_eq!(case.provider.calls.load(Ordering::SeqCst), 0);
    let result = pipeline::recognition::saved_result(&case.store, "route")
        .unwrap()
        .unwrap();
    assert_eq!(result.rows, case.original.rows);
    assert_eq!(result.coverage.len(), 2);
}

#[test]
fn combined_nemo_resume_preserves_saved_prefix_and_completed_new_live_epoch() {
    completed_nemo_resume(false);
}

#[test]
fn separate_nemo_resume_repeated_diarize_keeps_one_copy_of_saved_turns() {
    completed_nemo_resume(true);
}

fn completed_nemo_resume(separate: bool) {
    use session_core::recognition::{
        Coverage, OutputBlock, SpeakerTurn, read_artifact, write_artifact,
    };
    let mut case = saved_resume_case(&common::speech());
    case.spec = resolve(&RecognitionConfig {
        asr_model: if separate {
            MODEL
        } else {
            "nemotron-speech-streaming-en-0.6b-q80"
        }
        .into(),
        speaker_source: SpeakerSource::Nemotron,
        processing_mode: if separate {
            models::recognition::ProcessingMode::AfterStop
        } else {
            models::recognition::ProcessingMode::DuringRecording
        },
        speaker_processing_mode: models::recognition::ProcessingMode::DuringRecording,
        ..Default::default()
    })
    .unwrap();
    case.store
        .lock()
        .unwrap()
        .promote_result_snapshot("route", Some(&serde_json::to_value(&case.spec).unwrap()))
        .unwrap();
    // This synthetic native result uses the same neutral saved-artifact contract.
    let mut doc = session_core::generation::manifest(&case.home).unwrap();
    let mut blocks = vec![];
    if separate {
        case.original.speakers.clear();
    }
    let mut header = read_artifact(
        &case.home.join("recognition.jsonl"),
        doc.recognition_artifact.as_ref().unwrap(),
        |mut block| {
            if separate {
                block.speakers = block
                    .rows
                    .iter()
                    .map(|row| SpeakerTurn {
                        range: row.range,
                        timing: row.timing,
                        speaker: row.speaker.clone().unwrap(),
                        finality: Finality::Final,
                    })
                    .collect();
                case.original
                    .speakers
                    .extend(block.speakers.iter().cloned());
            }
            blocks.push(block);
            Ok(())
        },
    )
    .unwrap();
    header.snapshot = serde_json::to_value(&case.spec).unwrap();
    header.fingerprints = pipeline::recognition::durable::fingerprints(
        &case.store,
        "route",
        &case.dir,
        &case.segments[..1],
        &case.spec,
    )
    .unwrap();
    doc.recognition_artifact =
        Some(write_artifact(&case.home.join("recognition.jsonl"), &header, blocks).unwrap());
    doc.recognition_snapshot = Some(header.snapshot.clone());
    std::fs::write(case.home.join("session.md"), doc.render()).unwrap();
    let appended = &case.segments[1];
    let range = SampleRange {
        start: appended.offset_ms * 16,
        end: appended.offset_ms * 16
            + hound::WavReader::open(case.dir.join(pipeline::proc_file(&appended.audio_file)))
                .unwrap()
                .duration() as u64,
    };
    let scope = case
        .store
        .lock()
        .unwrap()
        .restart_recognition_epoch("route", &header.generation, range.start, range.end)
        .unwrap()
        .scope;
    let mut row = case.original.rows[0].clone();
    row.id.scope = scope.clone();
    row.id.input_start_sample = range.start;
    row.range = range;
    row.speaker = Some(SpeakerKey {
        scope: scope.clone(),
        input_start_sample: range.start,
        local_id: 1,
    });
    let new_turn = SpeakerTurn {
        range,
        timing: TimingPrecision::NativeSegment,
        speaker: row.speaker.clone().unwrap(),
        finality: Finality::Final,
    };
    case.store
        .lock()
        .unwrap()
        .commit_recognition(
            "route",
            &OutputBlock {
                key: format!("native-final/{:020}/row/00000000000000000000", scope.epoch),
                scope: scope.clone(),
                revision: row.revision,
                coverage: Coverage {
                    received: Some(range),
                    computed: Some(range),
                    provisional: None,
                    committed: Some(range),
                },
                rows: if separate { vec![] } else { vec![row.clone()] },
                speakers: if separate {
                    vec![new_turn.clone()]
                } else {
                    vec![]
                },
            },
        )
        .unwrap();
    let key = format!("recognition-epoch-{:020}", range.start);
    let epoch_stage = if separate {
        Stage::Diarize
    } else {
        Stage::Transcribe
    };
    checkpoint::put(&case.store, "route", epoch_stage, &key,
        &serde_json::json!({ "scope": scope, "range": range, "rows": usize::from(!separate), "speakers": usize::from(separate), "limitations": [], "output": { "text": !separate, "speakers": true } })).unwrap();
    let mut archive =
        pipeline::live::archive::ArchiveReader::new(pipeline::live::archive::Archive {
            dir: case.dir.join(pipeline::RECOVERY_DIR),
            segment: appended.index,
            offset_ms: appended.offset_ms,
            rate: 16000,
        });
    while archive.next_frame().unwrap().is_some() {}
    let mut config = pipeline::live::LiveConfig::new(
        "route",
        appended.index,
        &case.spec.config.asr_model,
        16000,
    );
    config.snapshot = Some(case.spec.clone());
    pipeline::live::archive::save_proof(&case.store, &config, &archive).unwrap();
    // The provider has no real helper. Recomputing old or completed new audio fails.
    for _ in 0..2 {
        if separate {
            pipeline::recognition::execute(
                &case.store,
                "route",
                &case.dir,
                &case.segments,
                &case.spec,
                &case.provider,
                Stage::Diarize,
                &Unattended,
                &mut |_| {},
            )
            .unwrap();
        }
        pipeline::recognition::execute(
            &case.store,
            "route",
            &case.dir,
            &case.segments,
            &case.spec,
            &case.provider,
            Stage::Transcribe,
            &Unattended,
            &mut |_| {},
        )
        .unwrap();
        let result = pipeline::recognition::saved_result(&case.store, "route")
            .unwrap()
            .unwrap();
        assert_eq!(result.rows[..case.original.rows.len()], case.original.rows);
        if separate {
            assert!(
                result
                    .rows
                    .iter()
                    .any(|row| row.range.start >= range.start && row.text == "Hello there"),
                "each separate speaker pass must keep the appended ASR text"
            );
            let mut expected = case.original.speakers.clone();
            expected.push(new_turn.clone());
            assert_eq!(
                result.speakers, expected,
                "Diarize retry duplicated or changed saved turns"
            );
        } else {
            assert_eq!(result.rows.last(), Some(&row));
            assert_eq!(result.rows.len(), case.original.rows.len() + 1);
        }
        assert_eq!(result.coverage.len(), 2);
    }
    let saved: serde_json::Value = checkpoint::get(&case.store, "route", epoch_stage, &key)
        .unwrap()
        .unwrap();
    assert_eq!(saved["scope"], serde_json::to_value(&scope).unwrap());
}

#[test]
fn resumed_recognition_retry_retains_prefix_after_cancellation() {
    struct Cancel;
    impl pipeline::Control for Cancel {
        fn proceed(&self) -> Result<(), pipeline::StageError> {
            Err(pipeline::StageError::Cancelled)
        }
    }
    let case = saved_resume_case(&common::speech());
    case.seed_appended_speakers();
    let error = pipeline::recognition::execute(
        &case.store,
        "route",
        &case.dir,
        &case.segments,
        &case.spec,
        &case.provider,
        Stage::Transcribe,
        &Cancel,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(error, pipeline::StageError::Cancelled));
    case.provider.calls.store(0, Ordering::SeqCst);
    pipeline::recognition::execute(
        &case.store,
        "route",
        &case.dir,
        &case.segments,
        &case.spec,
        &case.provider,
        Stage::Transcribe,
        &Unattended,
        &mut |_| {},
    )
    .unwrap();
    assert!(
        case.provider.calls.load(Ordering::SeqCst) > 0,
        "retry must infer new spoken audio"
    );
    for stage in [Stage::Join, Stage::Verify] {
        pipeline::recognition::execute(
            &case.store,
            "route",
            &case.dir,
            &case.segments,
            &case.spec,
            &case.provider,
            stage,
            &Unattended,
            &mut |_| {},
        )
        .unwrap();
    }
    assert_eq!(
        case.provider.calls.load(Ordering::SeqCst),
        case.appended_windows(),
        "retry recognized completed saved audio again"
    );
    let result = pipeline::recognition::saved_result(&case.store, "route")
        .unwrap()
        .unwrap();
    assert_eq!(result.rows[..case.original.rows.len()], case.original.rows);
    assert!(
        result
            .rows
            .iter()
            .any(|row| row.range.start >= case.segments[1].offset_ms * 16
                && row.text == "Hello there")
    );
}

#[test]
fn review_merged_independent_single_scope_results_explain_speaker_uncertainty() {
    let speech = common::speech();
    let clip = &speech[..16000 * 3];
    let case = saved_resume_audio(clip, clip);
    case.seed_appended_speakers();
    assert!(
        pipeline::recognition::found_speakers(&case.original).is_some(),
        "the original fixture must have one speaker namespace"
    );
    assert!(
        !case
            .original
            .coverage_limitations
            .iter()
            .any(|note| note == pipeline::recognition::SCOPED_LABELS_NOTE)
    );
    pipeline::recognition::execute(
        &case.store,
        "route",
        &case.dir,
        &case.segments,
        &case.spec,
        &case.provider,
        Stage::Transcribe,
        &Unattended,
        &mut |_| {},
    )
    .unwrap();
    let result = pipeline::recognition::saved_result(&case.store, "route")
        .unwrap()
        .unwrap();
    assert!(pipeline::recognition::found_speakers(&result).is_none());
    assert!(
        result
            .coverage_limitations
            .iter()
            .any(|note| note == pipeline::recognition::SCOPED_LABELS_NOTE),
        "merging distinct single-scope outputs needs the speaker uncertainty note"
    );
}
