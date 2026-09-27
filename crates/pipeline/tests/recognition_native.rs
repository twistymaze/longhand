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
    fn summary_prompt(&self) -> pipeline::summarize::Prompt {
        summarize::prompts::built_in(summarize::prompts::TOPIC_SUMMARY_ID).unwrap()
    }
    fn learn_limit(&self, _: &str, _: &str, _: u32) {}
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
        "integrated" => SpeakerSource::Integrated,
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
        pipeline::recognition::execute(
            &store,
            "route-test",
            &dir,
            &segments,
            &spec,
            &provider,
            stage,
            &pipeline::Unattended,
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
    if let Ok(path) = std::env::var("LONGHAND_ROUTE_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
}
