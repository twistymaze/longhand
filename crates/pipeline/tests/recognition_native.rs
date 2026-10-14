//! Opt-in inference through the production route stages. Keep audio and output
//! outside the repository. This test prints counts, never transcript text.
mod common;
use models::recognition::{
    ArtifactSpec, AsrRuntime, ExecutionSpec, RecognitionConfig, SpeakerSource, resolve_for_runtime,
};
use pipeline::{EndpointProblem, EngineProvider, SummaryEndpoint};
use std::{collections::BTreeMap, path::PathBuf};

struct Provider {
    paths: BTreeMap<String, PathBuf>,
    helper: PathBuf,
    gpu: i32,
}
impl EngineProvider for Provider {
    fn artifact_path(&self, a: &ArtifactSpec) -> Result<PathBuf, String> {
        self.paths
            .get(&a.id)
            .cloned()
            .ok_or_else(|| format!("missing exact artifact {}", a.id))
    }
    fn load_exact(&self, s: &ExecutionSpec) -> Result<Box<dyn asr::Recognizer>, String> {
        let path = self.artifact_path(&s.artifacts[0])?;
        let mut engine =
            asr::Engine::load(&s.config.asr_model, &path).map_err(|e| e.to_string())?;
        engine.set_language(s.config.language.clone());
        Ok(Box::new(engine))
    }
    fn analyzer_exact(
        &self,
        s: &ExecutionSpec,
    ) -> Result<Box<dyn diarize::SpeakerAnalyzer>, diarize::DiarizeError> {
        let path = |id: &str| {
            s.artifacts
                .iter()
                .find(|a| a.id == id)
                .and_then(|a| self.paths.get(&a.id))
                .cloned()
                .ok_or_else(|| diarize::DiarizeError::MissingWeights(id.into()))
        };
        Ok(Box::new(diarize::OnnxAnalyzer::load(
            &path("pyannote-segmentation-3.0")?,
            &path("wespeaker-voxceleb-resnet34-lm")?,
        )?))
    }
    fn nemo_helper(&self) -> Result<PathBuf, String> {
        Ok(self.helper.clone())
    }
    fn nemo_gpu(&self) -> i32 {
        self.gpu
    }
    fn ready(&self, _: &str) -> bool {
        panic!("legacy readiness forbidden")
    }
    fn load(&self, _: &str) -> Result<Box<dyn asr::Recognizer>, String> {
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
    fn summary_prompt(&self, _: Option<&str>) -> pipeline::summarize::Prompt {
        summarize::prompts::built_in(summarize::prompts::TOPIC_SUMMARY_ID).unwrap()
    }
    fn background(&self) -> String {
        String::new()
    }
    fn destination(&self) -> Option<PathBuf> {
        None
    }
}
#[test]
#[ignore = "requires explicit local model paths and real recording"]
fn production_route_recording_excerpt() {
    let model = std::env::var("LONGHAND_ROUTE_MODEL").expect("LONGHAND_ROUTE_MODEL");
    let speaker = match std::env::var("LONGHAND_ROUTE_SPEAKER")
        .as_deref()
        .unwrap_or("off")
    {
        "off" => SpeakerSource::Off,
        "pyannote" => SpeakerSource::Pyannote,
        "sortformer" => SpeakerSource::Sortformer,
        "nemotron" => SpeakerSource::Nemotron,
        _ => panic!("unknown speaker source"),
    };
    let runtime = if std::env::var("LONGHAND_ROUTE_RUNTIME").as_deref() == Ok("nemo") {
        AsrRuntime::Nemo
    } else {
        AsrRuntime::Transcribe
    };
    let spec = resolve_for_runtime(
        &RecognitionConfig {
            asr_model: model,
            speaker_source: speaker,
            ..Default::default()
        },
        runtime,
    )
    .unwrap();
    let mapping: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("LONGHAND_ROUTE_PATHS").expect("LONGHAND_ROUTE_PATHS"))
            .unwrap(),
    )
    .unwrap();
    let paths = mapping
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["id"].as_str().unwrap().to_string(),
                PathBuf::from(a["path"].as_str().unwrap()),
            )
        })
        .collect();
    let provider = Provider {
        paths,
        helper: std::env::var("LONGHAND_NEMO_HELPER")
            .unwrap_or_default()
            .into(),
        gpu: std::env::var("LONGHAND_ROUTE_GPU")
            .unwrap_or("-1".into())
            .parse()
            .unwrap(),
    };
    let mut reader =
        hound::WavReader::open(std::env::var("LONGHAND_ROUTE_WAV").expect("LONGHAND_ROUTE_WAV"))
            .unwrap();
    assert_eq!(reader.spec().sample_rate, 16000);
    assert_eq!(reader.spec().channels, 1);
    let count = std::env::var("LONGHAND_ROUTE_SECONDS")
        .unwrap_or("30".into())
        .parse::<usize>()
        .unwrap()
        * 16000;
    let pcm: Vec<f32> = if reader.spec().sample_format == hound::SampleFormat::Float {
        reader
            .samples::<f32>()
            .take(count)
            .collect::<Result<_, _>>()
            .unwrap()
    } else {
        reader
            .samples::<i16>()
            .take(count)
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect()
    };
    let temp = tempfile::tempdir().unwrap();
    let store = common::store();
    common::queue_session(
        &store,
        temp.path(),
        "route-test",
        100,
        Some(&spec.config.asr_model),
        &[(16000, pcm)],
    );
    store
        .lock()
        .unwrap()
        .set_recognition_snapshot("route-test", &serde_json::to_value(&spec).unwrap())
        .unwrap();
    let segments = store.lock().unwrap().segments("route-test").unwrap();
    let dir = temp.path().join("route-test");
    pipeline::assemble::run(
        &store,
        "route-test",
        &dir,
        &segments,
        None,
        &pipeline::Unattended,
    )
    .unwrap();
    let started = std::time::Instant::now();
    // Each speaker report with the seconds since the start.
    let mut speaker_reports: Vec<(f64, pipeline::progress::SpeakerWork)> = Vec::new();
    let graph = pipeline::recognition::TaskGraph::resolve(&spec).unwrap();
    for stage in graph.stages.into_iter().filter(|s| {
        matches!(
            s,
            session_core::Stage::Diarize
                | session_core::Stage::Transcribe
                | session_core::Stage::Join
                | session_core::Stage::Verify
        )
    }) {
        pipeline::recognition::execute_with_speakers(
            &store,
            "route-test",
            &dir,
            &segments,
            &spec,
            &provider,
            stage,
            &pipeline::Unattended,
            &mut |work| speaker_reports.push((started.elapsed().as_secs_f64(), work)),
            &mut |_| {},
        )
        .unwrap_or_else(|e| {
            if let Ok(path) = std::env::var("LONGHAND_ROUTE_OUTPUT") {
                let mut checkpoints = serde_json::Map::new();
                for saved_stage in [
                    session_core::Stage::Assemble,
                    session_core::Stage::Diarize,
                    session_core::Stage::Transcribe,
                    session_core::Stage::Verify,
                ] {
                    let rows = store
                        .lock()
                        .unwrap()
                        .checkpoints("route-test", saved_stage)
                        .unwrap();
                    let values = rows
                        .into_iter()
                        .map(|(key, payload)| {
                            (
                                key,
                                serde_json::from_str::<serde_json::Value>(&payload)
                                    .unwrap_or(serde_json::Value::String(payload)),
                            )
                        })
                        .collect::<serde_json::Map<_, _>>();
                    checkpoints.insert(
                        format!("{saved_stage:?}"),
                        serde_json::Value::Object(values),
                    );
                }
                let diagnostic = serde_json::json!({
                    "failure": { "stage": format!("{stage:?}"), "message": e.to_string() },
                    "result": pipeline::recognition::saved_result(&store, "route-test").unwrap(),
                    "checkpoints": checkpoints,
                });
                std::fs::write(path, serde_json::to_vec_pretty(&diagnostic).unwrap()).unwrap();
            }
            panic!("{stage:?}: {e}")
        });
    }
    let result = pipeline::recognition::saved_result(&store, "route-test")
        .unwrap()
        .unwrap();
    assert!(!result.rows.is_empty());
    if speaker != SpeakerSource::Off {
        assert!(result.rows.iter().any(|r| r.speaker.is_some()));
    }
    let mut labels = Vec::new();
    for speaker in result.rows.iter().filter_map(|r| r.speaker.as_ref()) {
        if !labels.contains(&speaker) {
            labels.push(speaker);
        }
    }
    if let Ok(minimum) = std::env::var("LONGHAND_ROUTE_MIN_LABELS") {
        assert!(
            labels.len() >= minimum.parse::<usize>().unwrap(),
            "too few distinct labels in the joined text"
        );
    }
    println!(
        "route={:?} model={} rows={} attributed={} labels={} speaker_turns={} elapsed_s={:.3} limitations={:?}",
        result.route,
        spec.config.asr_model,
        result.rows.len(),
        result.rows.iter().filter(|r| r.speaker.is_some()).count(),
        labels.len(),
        result.speakers.len(),
        started.elapsed().as_secs_f64(),
        result.coverage_limitations
    );
    if !speaker_reports.is_empty() {
        let measured: Vec<(f64, u8)> = speaker_reports
            .iter()
            .filter(|(_, work)| work.measuring)
            .filter_map(|(at, work)| work.percent.map(|p| (*at, p)))
            .collect();
        let percents: Vec<u8> = speaker_reports
            .iter()
            .filter_map(|(_, work)| work.percent)
            .collect();
        // The longest time with no new measured value, to the last report.
        let mut longest = 0.0f64;
        let mut previous = speaker_reports[0].0;
        for (at, _) in &measured {
            longest = longest.max(at - previous);
            previous = *at;
        }
        let last = speaker_reports.last().unwrap();
        println!(
            "speaker_reports={} measured_reports={} first_measured_s={:.1} longest_unmeasured_s={:.1} final_unmeasured_s={:.1} monotone={} max_percent={:?} last={:?}",
            speaker_reports.len(),
            measured.len(),
            measured.first().map_or(-1.0, |(at, _)| *at),
            longest,
            last.0 - previous,
            percents.windows(2).all(|pair| pair[0] <= pair[1]),
            percents.iter().max(),
            last.1
        );
        assert!(percents.iter().all(|p| *p < 100), "{percents:?}");
    }
    if let Ok(path) = std::env::var("LONGHAND_ROUTE_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "requires explicit local artifacts, recording, and paced live input"]
fn paced_durable_capture_runs_native_recognition_before_stop() {
    use pipeline::live::{EngineAccess, LiveConfig, LiveEnd, LiveSink, Workers};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;
    struct Sink(Arc<AtomicUsize>);
    impl LiveSink for Sink {
        fn recognition(&mut self, block: &session_core::recognition::OutputBlock) {
            if !block.rows.is_empty() || !block.speakers.is_empty() {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
    let speaker = match std::env::var("LONGHAND_ROUTE_SPEAKER")
        .as_deref()
        .unwrap_or("nemotron")
    {
        "sortformer" => SpeakerSource::Sortformer,
        "nemotron" => SpeakerSource::Nemotron,
        _ => panic!("live test requires separate NeMo speakers"),
    };
    let runtime = if std::env::var("LONGHAND_ROUTE_RUNTIME").as_deref() == Ok("nemo") {
        AsrRuntime::Nemo
    } else {
        AsrRuntime::Transcribe
    };
    let live_text = runtime == AsrRuntime::Nemo
        && std::env::var("LONGHAND_LIVE_TEXT").as_deref() != Ok("false");
    let live_speakers = std::env::var("LONGHAND_LIVE_SPEAKERS").as_deref() != Ok("false");
    let timing = |enabled| {
        if enabled {
            models::recognition::ProcessingMode::DuringRecording
        } else {
            models::recognition::ProcessingMode::AfterStop
        }
    };
    let spec = resolve_for_runtime(
        &RecognitionConfig {
            asr_model: std::env::var("LONGHAND_ROUTE_MODEL").unwrap(),
            speaker_source: speaker,
            processing_mode: timing(live_text),
            speaker_processing_mode: timing(live_speakers),
            ..Default::default()
        },
        runtime,
    )
    .unwrap();
    let mapping: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("LONGHAND_ROUTE_PATHS").unwrap()).unwrap(),
    )
    .unwrap();
    let provider = Arc::new(Provider {
        paths: mapping
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (
                    a["id"].as_str().unwrap().to_string(),
                    PathBuf::from(a["path"].as_str().unwrap()),
                )
            })
            .collect(),
        helper: std::env::var("LONGHAND_NEMO_HELPER").unwrap().into(),
        gpu: std::env::var("LONGHAND_ROUTE_GPU")
            .unwrap_or("-1".into())
            .parse()
            .unwrap(),
    });
    let mut input = hound::WavReader::open(std::env::var("LONGHAND_ROUTE_WAV").unwrap()).unwrap();
    assert_eq!(input.spec().sample_rate, 16000);
    assert_eq!(input.spec().channels, 1);
    let seconds: usize = std::env::var("LONGHAND_ROUTE_SECONDS")
        .unwrap_or("300".into())
        .parse()
        .unwrap();
    let cancel_at = std::env::var("LONGHAND_LIVE_CANCEL_AT_SECONDS")
        .ok()
        .map(|s| s.parse::<usize>().unwrap());
    let pace: u64 = std::env::var("LONGHAND_LIVE_PACE_MS")
        .unwrap_or("1000".into())
        .parse()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let store = common::store();
    store
        .lock()
        .unwrap()
        .create_session("live", common::at(0), None)
        .unwrap();
    store
        .lock()
        .unwrap()
        .set_recognition_snapshot("live", &serde_json::to_value(&spec).unwrap())
        .unwrap();
    let recovery = temp.path().join(pipeline::RECOVERY_DIR);
    let mut writer = recorder::ChunkedWriter::create(recorder::SegmentArgs {
        dir: recovery.clone(),
        segment: 0,
        capture_started_at: 1,
        offset_ms: 0,
        sample_rate: 16000,
        channels: 1,
        floor_bytes: 0,
    })
    .unwrap();
    let provider_live = provider.clone();
    let workers = Workers::new(
        store.clone(),
        EngineAccess::new(
            Arc::new(Mutex::new(None)),
            Arc::new(|_| panic!("native live must not load transcribe.cpp")),
            CancellationToken::new(),
        ),
    )
    .with_recognition(Arc::new(move |store, config, cancel, end, sink| {
        pipeline::live::recognize(store, config, provider_live.as_ref(), cancel, end, sink)
    }));
    let mut config =
        LiveConfig::new("live", 0, &spec.config.asr_model, 16000).with_archive(recovery, 0);
    config.text_enabled = false;
    config.speakers_enabled = true;
    config.snapshot = Some(spec.clone());
    let output = Arc::new(AtomicUsize::new(0));
    let (tap, worker) = workers
        .start(
            config,
            asr::Parts::new(0, asr::PartPolicy::default()),
            Box::new(Sink(output.clone())),
        )
        .unwrap();
    let began = Instant::now();
    for second in 0..seconds {
        let pcm: Vec<f32> = if input.spec().sample_format == hound::SampleFormat::Float {
            input
                .samples::<f32>()
                .take(16000)
                .collect::<Result<_, _>>()
                .unwrap()
        } else {
            input
                .samples::<i16>()
                .take(16000)
                .map(|s| s.unwrap() as f32 / 32768.0)
                .collect()
        };
        assert_eq!(pcm.len(), 16000);
        writer.write(&pcm).unwrap();
        if cancel_at == Some(second + 1) {
            workers.shutdown();
        }
        // Drop all notifications. Disk is the only native input source.
        let due = Duration::from_millis((second as u64 + 1) * pace);
        if let Some(wait) = due.checked_sub(began.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    let before_stop = output.load(Ordering::SeqCst);
    writer.finish().unwrap();
    drop(tap);
    let stopped = Instant::now();
    worker.end(LiveEnd::Keep);
    let finalize_timeout = std::env::var("LONGHAND_LIVE_FINALIZE_TIMEOUT_SECONDS")
        .unwrap_or("600".into())
        .parse::<u64>()
        .unwrap();
    while workers.busy() && stopped.elapsed() < Duration::from_secs(finalize_timeout) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let completed = !workers.busy();
    workers.shutdown();
    assert!(completed, "live worker did not finalize");
    let failure: Option<String> = pipeline::checkpoint::get(
        &store,
        "live",
        session_core::Stage::Transcribe,
        "live-failure-00000000",
    )
    .unwrap();
    assert!(
        failure.is_none()
            || (cancel_at.is_some()
                && failure
                    .as_ref()
                    .is_some_and(|s| s.to_lowercase().contains("cancel"))),
        "native live failure: {failure:?}"
    );
    assert!(before_stop > 0, "no native output arrived before Stop");
    let stage = if live_text {
        session_core::Stage::Transcribe
    } else {
        session_core::Stage::Diarize
    };
    let epochs = store
        .lock()
        .unwrap()
        .checkpoints("live", stage)
        .unwrap()
        .into_iter()
        .filter(|(key, _)| key.starts_with("recognition-epoch-"))
        .count();
    if cancel_at.is_none() {
        assert_eq!(epochs, seconds.div_ceil(1200));
    }
    eprintln!(
        "live_finalize_seconds={:.3}",
        stopped.elapsed().as_secs_f64()
    );
    let completed_live: Vec<_> = store
        .lock()
        .unwrap()
        .checkpoints("live", stage)
        .unwrap()
        .into_iter()
        .filter(|(key, _)| key.starts_with("recognition-epoch-"))
        .collect();
    let segment = session_core::Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: seconds as u64 * 1000,
        wall_start: common::at(0),
        wall_end: common::at(seconds as i64),
        audio_file: "audio/segment-01.opus".into(),
        channels: 1,
        ends_in_pause: false,
    };
    store.lock().unwrap().put_segment("live", &segment).unwrap();
    pipeline::assemble::run(
        &store,
        "live",
        temp.path(),
        std::slice::from_ref(&segment),
        None,
        &pipeline::Unattended,
    )
    .unwrap();
    eprintln!(
        "stop_through_assemble_seconds={:.3}",
        stopped.elapsed().as_secs_f64()
    );
    for task in pipeline::recognition::TaskGraph::resolve(&spec)
        .unwrap()
        .stages
        .into_iter()
        .filter(|s| {
            matches!(
                s,
                session_core::Stage::Diarize
                    | session_core::Stage::Transcribe
                    | session_core::Stage::Join
                    | session_core::Stage::Verify
            )
        })
    {
        let task_started = Instant::now();
        pipeline::recognition::execute(
            &store,
            "live",
            temp.path(),
            std::slice::from_ref(&segment),
            &spec,
            provider.as_ref(),
            task,
            &pipeline::Unattended,
            &mut |_| {},
        )
        .unwrap();
        eprintln!(
            "final_stage={task:?} seconds={:.3}",
            task_started.elapsed().as_secs_f64()
        );
    }
    for (key, value) in completed_live {
        assert_eq!(
            store
                .lock()
                .unwrap()
                .checkpoint_value("live", stage, &key)
                .unwrap()
                .as_deref(),
            Some(value.as_str()),
            "successful live epoch was replayed after Stop"
        );
    }
    let result = pipeline::recognition::saved_result(&store, "live")
        .unwrap()
        .unwrap();
    assert!(!result.rows.is_empty());
    assert!(result.rows.iter().any(|row| row.speaker.is_some()));
    assert!(!result.coverage.is_empty());
    let mut covered = 0;
    for range in &result.coverage {
        assert_eq!(
            range.start, covered,
            "live epochs must have no gaps or overlap"
        );
        assert!(
            range.end > range.start,
            "live epoch coverage must be nonempty"
        );
        covered = range.end;
    }
    assert_eq!(covered, seconds as u64 * 16000);
    eprintln!(
        "live_seconds={seconds} pace_ms={pace} events_before_stop={before_stop} epochs={epochs} rows={} stop_through_verify_seconds={:.3}",
        result.rows.len(),
        stopped.elapsed().as_secs_f64()
    );
}

#[test]
#[ignore = "requires local pyannote artifacts and a real recording excerpt"]
fn live_pyannote_features_are_reused_by_final_clustering() {
    use diarize::SpeakerAnalyzer;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;
    struct Counted {
        inner: diarize::OnnxAnalyzer,
        calls: Arc<AtomicUsize>,
    }
    impl SpeakerAnalyzer for Counted {
        fn analyze(
            &mut self,
            audio: &mut dyn diarize::AudioSource,
            request: &diarize::DiarizeRequest<'_>,
        ) -> Result<diarize::Analysis, diarize::DiarizeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.analyze(audio, request)
        }
    }
    let spec = models::recognition::resolve(&RecognitionConfig {
        asr_model: "whisper-large-v3-turbo-q5km".into(),
        speaker_source: SpeakerSource::Pyannote,
        ..Default::default()
    })
    .unwrap();
    let mapping: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("LONGHAND_ROUTE_PATHS").unwrap()).unwrap(),
    )
    .unwrap();
    let path = |id: &str| {
        PathBuf::from(
            mapping
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["id"] == id)
                .unwrap()["path"]
                .as_str()
                .unwrap(),
        )
    };
    let analyzer = diarize::OnnxAnalyzer::load(
        &path("pyannote-segmentation-3.0"),
        &path("wespeaker-voxceleb-resnet34-lm"),
    )
    .unwrap();
    eprintln!(
        "feature_backend={:?} fallback={:?}",
        analyzer.embedding_backend(),
        analyzer.embedding_fallback_reason()
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let mut counted = Counted {
        inner: analyzer,
        calls: calls.clone(),
    };
    let temp = tempfile::tempdir().unwrap();
    let store = common::store();
    store
        .lock()
        .unwrap()
        .create_session("features", common::at(0), None)
        .unwrap();
    store
        .lock()
        .unwrap()
        .set_recognition_snapshot("features", &serde_json::to_value(&spec).unwrap())
        .unwrap();
    let recovery = temp.path().join(pipeline::RECOVERY_DIR);
    let mut writer = recorder::ChunkedWriter::create(recorder::SegmentArgs {
        dir: recovery.clone(),
        segment: 0,
        capture_started_at: 1,
        offset_ms: 0,
        sample_rate: 16000,
        channels: 1,
        floor_bytes: 0,
    })
    .unwrap();
    // Evaluate the production extraction path before advertising this capability.
    let mut config = pipeline::live::LiveConfig::new("features", 0, &spec.config.asr_model, 16000)
        .with_archive(recovery, 0);
    config.snapshot = Some(spec.clone());
    let end = Arc::new(Mutex::new(None));
    let worker_store = store.clone();
    let worker_end = end.clone();
    let worker = std::thread::spawn(move || {
        pipeline::live::features::extract(
            &worker_store,
            &config,
            &mut counted,
            &CancellationToken::new(),
            &worker_end,
        )
        .unwrap();
        counted
    });
    let mut input = hound::WavReader::open(std::env::var("LONGHAND_ROUTE_WAV").unwrap()).unwrap();
    assert_eq!(input.spec().sample_rate, 16000);
    assert_eq!(input.spec().channels, 1);
    let seconds: usize = std::env::var("LONGHAND_ROUTE_SECONDS")
        .unwrap_or("300".into())
        .parse()
        .unwrap();
    let pace: u64 = std::env::var("LONGHAND_LIVE_PACE_MS")
        .unwrap_or("1000".into())
        .parse()
        .unwrap();
    let began = Instant::now();
    for second in 0..seconds {
        let pcm: Vec<f32> = if input.spec().sample_format == hound::SampleFormat::Float {
            input
                .samples::<f32>()
                .take(16000)
                .collect::<Result<_, _>>()
                .unwrap()
        } else {
            input
                .samples::<i16>()
                .take(16000)
                .map(|s| s.unwrap() as f32 / 32768.0)
                .collect()
        };
        assert_eq!(pcm.len(), 16000);
        writer.write(&pcm).unwrap();
        if let Some(wait) =
            Duration::from_millis((second as u64 + 1) * pace).checked_sub(began.elapsed())
        {
            std::thread::sleep(wait);
        }
    }
    let before_stop = calls.load(Ordering::SeqCst);
    assert!(before_stop > 0, "no features were extracted before Stop");
    writer.finish().unwrap();
    *end.lock().unwrap() = Some(pipeline::live::LiveEnd::Keep);
    let stopped = Instant::now();
    let mut counted = worker.join().unwrap();
    let extracted = calls.load(Ordering::SeqCst);
    let segment = session_core::Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: seconds as u64 * 1000,
        wall_start: common::at(0),
        wall_end: common::at(seconds as i64),
        audio_file: "audio/segment-01.opus".into(),
        channels: 1,
        ends_in_pause: false,
    };
    store
        .lock()
        .unwrap()
        .put_segment("features", &segment)
        .unwrap();
    pipeline::assemble::run(
        &store,
        "features",
        temp.path(),
        std::slice::from_ref(&segment),
        None,
        &pipeline::Unattended,
    )
    .unwrap();
    pipeline::recognition::durable::prepare(
        &store,
        "features",
        temp.path(),
        std::slice::from_ref(&segment),
        &spec,
    )
    .unwrap();
    pipeline::diarize::run(
        &store,
        "features",
        temp.path(),
        std::slice::from_ref(&segment),
        None,
        &mut counted,
        &pipeline::Unattended,
        &mut pipeline::progress::SpeakerMeter::new(&mut |_| {}),
    )
    .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        extracted,
        "final clustering repeated live inference"
    );
    let pipeline::diarize::Speakers::Turns(turns) =
        pipeline::diarize::speakers(&store, "features").unwrap()
    else {
        panic!("speaker clustering failed");
    };
    assert!(!turns.is_empty());
    assert!(turns.iter().all(|t| t.end_ms <= seconds as u64 * 1000));
    eprintln!(
        "feature_seconds={seconds} pace_ms={pace} windows_before_stop={before_stop} windows={extracted} turns={} stop_through_clustering_seconds={:.3}",
        turns.len(),
        stopped.elapsed().as_secs_f64()
    );
}
