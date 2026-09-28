mod common;
use asr::{
    AsrError, AsrSegment, Finality, RecognitionId, RecognitionOutput, RecognitionRequest,
    RecognitionSegment, Recognizer, SampleRange, SpeakerKey, TimingPrecision, WindowText,
};
use models::recognition::{ExecutionSpec, RecognitionConfig, SpeakerSource, resolve};
use pipeline::{EndpointProblem, EngineProvider, SummaryEndpoint, Unattended};
use session_core::Stage;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Fake {
    calls: Arc<AtomicUsize>,
    native_tokens: bool,
}
impl Recognizer for Fake {
    fn model_id(&self) -> &str {
        "moss-transcribe-diarize-q80"
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
    fn transcribe_normalized(
        &mut self,
        pcm: &[f32],
        r: &RecognitionRequest,
    ) -> Result<RecognitionOutput, AsrError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let coverage = SampleRange {
            start: r.start_sample,
            end: r.start_sample + pcm.len() as u64,
        };
        Ok(RecognitionOutput {
            scope: r.scope.clone(),
            revision: 0,
            coverage,
            text: "Hello there".into(),
            segments: vec![RecognitionSegment {
                id: RecognitionId {
                    scope: r.scope.clone(),
                    input_start_sample: r.start_sample,
                    ordinal: 0,
                },
                range: coverage,
                timing: TimingPrecision::NativeSegment,
                text: "Hello there".into(),
                speaker: Some(SpeakerKey {
                    scope: r.scope.clone(),
                    input_start_sample: r.start_sample,
                    local_id: 1,
                }),
                revision: 0,
                text_finality: Finality::Final,
                speaker_finality: Finality::Final,
            }],
            speaker_turns: vec![],
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
        asr_model: "moss-transcribe-diarize-q80".into(),
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
    let temp = tempfile::tempdir().unwrap();
    let store = common::store();
    let spec = spec(source);
    common::queue_session(
        &store,
        temp.path(),
        "route",
        100,
        Some(&spec.config.asr_model),
        &[(16000, common::speech())],
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
#[test]
fn off_and_integrated_execute_without_separate_weights() {
    for source in [SpeakerSource::Off, SpeakerSource::Integrated] {
        let (temp, store, spec, segments) = setup(source);
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
            source == SpeakerSource::Integrated
        );
    }
}
#[test]
fn off_reuses_successful_live_text_without_asr() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Off);
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
#[test]
fn integrated_after_stop_runs_requested_native_labels_after_live_preview() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Integrated);
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
                text: "Preview only".into(),
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
    assert!(provider.calls.load(Ordering::SeqCst) > 0);
    assert!(
        pipeline::recognition::saved_result(&store, "route")
            .unwrap()
            .unwrap()
            .rows
            .iter()
            .all(|r| r.speaker.is_some())
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
fn changed_preprocessing_invalidates_completed_native_windows() {
    let (temp, store, mut spec, segments) = setup(SpeakerSource::Integrated);
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
    run(&spec);
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
    spec.preprocessing_revision.push_str("-changed");
    run(&spec);
    assert!(provider.calls.load(Ordering::SeqCst) > calls);
}

#[test]
fn join_writes_portable_output_referenced_by_the_manifest() {
    let (temp, store, spec, segments) = setup(SpeakerSource::Integrated);
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
    let (temp, store, spec, segments) = setup(SpeakerSource::Integrated);
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
    let (temp, store, spec, segments) = setup(SpeakerSource::Integrated);
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
    let (temp, store, spec, segments) = setup(SpeakerSource::Integrated);
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
