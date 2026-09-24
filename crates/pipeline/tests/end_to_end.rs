//! A real recording through the real runner and model (U8 verification).
//!
//! ```text
//! LONGHAND_E2E_WAV=/path/to/mono-16bit.wav \
//! LONGHAND_ASR_MODEL=/path/to/model \
//! LONGHAND_WEIGHTS_DIR=/path/to/diarization/weights \
//! [LONGHAND_E2E_MUTE=start_s,length_s] [LONGHAND_E2E_BLANK_EVERY=n] \
//! cargo test -p pipeline --release --test end_to_end -- --ignored --nocapture
//! ```
//!
//! The WAV is fed through the chunk writer at its own rate, as capture
//! would, then assembled, diarized, transcribed and joined. Only counts and timings are
//! printed, never transcript text: the input may be a private recording.
//!
//! `LONGHAND_E2E_MUTE` silences a stretch of the input, which verification
//! must pass; `LONGHAND_E2E_BLANK_EVERY` makes every nth window decode
//! nothing, which verification must catch (U20).

mod common;

use std::sync::Arc;
use std::time::Instant;

use asr::{Engine, Recognizer};
use common::{at, store, write_segment};
use diarize::{DiarizeError, OnnxAnalyzer, SpeakerAnalyzer};
use pipeline::runner::Outcome;
use pipeline::transcribe::{self, WindowPlan};
use pipeline::{EngineProvider, Environment, Runner, checkpoint, proc_file};
use session_core::{Event, Stage};

struct RealModel {
    path: String,
    blank_every: Option<usize>,
    destination: Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
}

/// A recognizer that loses every nth window, as a failing decoder would.
struct Blanking {
    inner: Box<dyn Recognizer>,
    every: usize,
    calls: usize,
}

impl Recognizer for Blanking {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn transcribe(&mut self, pcm: &[f32]) -> Result<asr::WindowText, asr::AsrError> {
        self.calls += 1;
        if self.calls.is_multiple_of(self.every) {
            return Ok(asr::WindowText {
                text: String::new(),
                segments: vec![],
            });
        }
        self.inner.transcribe(pcm)
    }
}

impl EngineProvider for RealModel {
    fn summary_endpoint(&self) -> Result<pipeline::SummaryEndpoint, pipeline::EndpointProblem> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        match (
            var("LONGHAND_SUMMARY_BASE_URL"),
            var("LONGHAND_SUMMARY_MODEL"),
        ) {
            (Some(base_url), Some(model)) => Ok(pipeline::SummaryEndpoint {
                base_url,
                model,
                credential: var("LONGHAND_SUMMARY_KEY"),
                context_limit: var("LONGHAND_SUMMARY_CONTEXT").map(|v| v.parse().unwrap()),
                learned_limit: None,
            }),
            _ => Err(pipeline::EndpointProblem::Unconfigured(
                "no summary endpoint in the environment".into(),
            )),
        }
    }
    fn learn_limit(&self, _: &str, _: &str, limit: u32) {
        eprintln!("learned a context limit of {limit}");
    }
    fn destination(&self) -> Option<std::path::PathBuf> {
        self.destination.lock().unwrap().clone()
    }
    fn ready(&self, _: &str) -> bool {
        true
    }
    fn load(&self, model_id: &str) -> Result<Box<dyn Recognizer>, String> {
        let engine = Engine::load(model_id, std::path::Path::new(&self.path))
            .map(|e| Box::new(e) as Box<dyn Recognizer>)
            .map_err(|e| e.to_string())?;
        Ok(match self.blank_every {
            Some(every) => Box::new(Blanking {
                inner: engine,
                every,
                calls: 0,
            }),
            None => engine,
        })
    }
    fn default_model(&self) -> String {
        "e2e".into()
    }
    fn analyzer(&self) -> Result<Box<dyn SpeakerAnalyzer>, DiarizeError> {
        let dir = std::path::PathBuf::from(
            std::env::var("LONGHAND_WEIGHTS_DIR").expect("LONGHAND_WEIGHTS_DIR is set"),
        );
        Ok(Box::new(OnnxAnalyzer::load(
            &dir.join("segmentation.onnx"),
            &dir.join("embedding.onnx"),
        )?))
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a recording and a recognition model on disk"]
async fn a_real_recording_assembles_and_transcribes_end_to_end() {
    let wav = std::env::var("LONGHAND_E2E_WAV").expect("LONGHAND_E2E_WAV is set");
    let model = std::env::var("LONGHAND_ASR_MODEL").expect("LONGHAND_ASR_MODEL is set");
    let reader = hound::WavReader::open(&wav).unwrap();
    let spec = reader.spec();
    assert_eq!((spec.channels, spec.bits_per_sample), (1, 16));
    let mut audio: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32_768.0)
        .collect();
    let seconds = audio.len() as f64 / spec.sample_rate as f64;
    if let Ok(mute) = std::env::var("LONGHAND_E2E_MUTE") {
        let (start, length) = mute.split_once(',').expect("start_s,length_s");
        let rate = spec.sample_rate as usize;
        let start = start.parse::<usize>().unwrap() * rate;
        let end = (start + length.parse::<usize>().unwrap() * rate).min(audio.len());
        audio[start..end].fill(0.0);
        eprintln!("muted {} s", (end - start) / rate);
    }
    let blank_every = std::env::var("LONGHAND_E2E_BLANK_EVERY")
        .ok()
        .map(|n| n.parse::<usize>().unwrap());

    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("e2e");
    let segment = write_segment(&dir, 0, 0, spec.sample_rate, &audio, false);
    drop(audio);
    let store = store();
    {
        let mut s = store.lock().unwrap();
        s.create_session("e2e", at(0), None).unwrap();
        s.put_segment("e2e", &segment).unwrap();
        s.apply(
            "e2e",
            &Event::UserStop {
                at: at(seconds as i64),
                retained_audio: true,
            },
        )
        .unwrap();
    }

    // No destination at first: the session stops before landing, with its
    // working files in place to inspect, and lands once one is chosen.
    let destination = Arc::new(std::sync::Mutex::new(None));
    let stages = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&stages);
    let started = Instant::now();
    let runner = Runner::new(Environment {
        store: Arc::clone(&store),
        working_root: root.path().to_path_buf(),
        engines: Box::new(RealModel {
            path: model,
            blank_every,
            destination: Arc::clone(&destination),
        }),
        suspended: Box::new(|| false),
        events: Box::new(move |e| {
            if let pipeline::RunnerEvent::StageStarted { stage, .. } = e {
                sink.lock().unwrap().push((stage, started.elapsed()));
            }
        }),
    });
    let (_, outcome) = runner.run_next().await.unwrap();
    let total = started.elapsed();
    let coverage: Option<pipeline::verify::Coverage> =
        checkpoint::get(&store, "e2e", Stage::Verify, &checkpoint::segment_key(0)).unwrap();
    if let Some(c) = &coverage {
        eprintln!(
            "coverage: {:.1}% of {:.0} s voiced; worst untranscribed stretch {:.1} s; {} stretches",
            c.ratio() * 100.0,
            c.voiced_ms as f64 / 1000.0,
            c.longest_uncovered_ms as f64 / 1000.0,
            c.uncovered.len()
        );
    }
    if blank_every.is_some() {
        let Outcome::Failed { stage, message } = outcome else {
            panic!("blanked windows passed verification");
        };
        assert_eq!(stage, Stage::Verify);
        eprintln!(
            "verification caught it: {} characters of message",
            message.len()
        );
        return;
    }
    assert!(
        matches!(outcome, Outcome::AwaitingDestination { .. }),
        "{outcome:?}"
    );
    // Without an endpoint in the environment the summary is a stub.
    match std::fs::read_to_string(dir.join(pipeline::summarize::SUMMARY_FILE)) {
        Ok(text) => {
            let meta = session_core::folder::Summary::parse(&text).unwrap().meta;
            eprintln!(
                "summary: {} words; {meta:?}",
                text.split_whitespace().count()
            );
        }
        Err(_) => eprintln!("summary: none"),
    }

    let proc = hound::WavReader::open(dir.join(proc_file(&segment.audio_file))).unwrap();
    let proc_spec = proc.spec();
    assert_eq!(proc_spec.sample_rate, 16_000);
    assert_eq!(proc_spec.channels, 1);
    assert_eq!(proc_spec.sample_format, hound::SampleFormat::Float);
    let proc_seconds = proc.duration() as f64 / 16_000.0;
    assert!((proc_seconds - seconds).abs() < 0.05);

    let plan: WindowPlan =
        checkpoint::get(&store, "e2e", Stage::Transcribe, &checkpoint::plan_key(0))
            .unwrap()
            .unwrap();
    let results = transcribe::results(&store, "e2e").unwrap();
    assert_eq!(results.len(), plan.windows.len());
    let words: usize = results
        .iter()
        .map(|r| r.text.split_whitespace().count())
        .sum();
    let empty = results.iter().filter(|r| r.text.is_empty()).count();
    let longest = plan
        .windows
        .iter()
        .map(|w| w.duration_ms())
        .max()
        .unwrap_or(0);
    let archive = std::fs::metadata(dir.join(&segment.audio_file))
        .unwrap()
        .len();

    let stages = stages.lock().unwrap().clone();
    eprintln!("audio: {seconds:.0} s at {} Hz", spec.sample_rate);
    for (i, (stage, began)) in stages.iter().enumerate() {
        let ended = stages.get(i + 1).map_or(total, |(_, t)| *t);
        let took = (ended - *began).as_secs_f64();
        eprintln!("{stage:?}: {took:.1} s ({:.1}x real time)", seconds / took);
    }
    let speakers = match pipeline::diarize::speakers(&store, "e2e").unwrap() {
        pipeline::diarize::Speakers::Turns(turns) => {
            let labels: std::collections::BTreeSet<u32> = turns.iter().map(|t| t.speaker).collect();
            format!("{} turns, {} speakers", turns.len(), labels.len())
        }
        pipeline::diarize::Speakers::Unavailable => "unavailable".into(),
    };
    eprintln!("speakers: {speakers}");

    let transcript = std::fs::read_to_string(dir.join(pipeline::join::TRANSCRIPT_FILE)).unwrap();
    eprintln!("transcript: {} lines", transcript.lines().count());
    eprintln!(
        "windows: {} (longest {} ms, {} empty), voiced {:.0} s of {:.0} s",
        plan.windows.len(),
        longest,
        empty,
        plan.voiced_ms as f64 / 1000.0,
        plan.duration_ms as f64 / 1000.0
    );
    eprintln!("words: {words}; archive {:.1} MB", archive as f64 / 1e6);
    assert!(words > 0);

    let dest = tempfile::tempdir().unwrap();
    *destination.lock().unwrap() = Some(dest.path().to_path_buf());
    let working_bytes = tree_bytes(&dir);
    runner.retry_awaiting().unwrap();
    let save_started = Instant::now();
    let (_, outcome) = runner.run_next().await.unwrap();
    assert_eq!(outcome, Outcome::Finished);
    let record = store.lock().unwrap().session("e2e").unwrap().unwrap();
    let saved = std::path::PathBuf::from(record.home_path.unwrap());
    assert!(!saved.join(pipeline::PROC_DIR).exists());
    assert!(!saved.join(pipeline::RECOVERY_DIR).exists());
    eprintln!(
        "landed in {:.2} s: {:.1} MB (archive {:.1} MB) from a {:.1} MB working copy",
        save_started.elapsed().as_secs_f64(),
        tree_bytes(&saved) as f64 / 1e6,
        archive as f64 / 1e6,
        working_bytes as f64 / 1e6
    );
}

fn tree_bytes(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| {
            let meta = e.metadata().unwrap();
            if meta.is_dir() {
                tree_bytes(&e.path())
            } else {
                meta.len()
            }
        })
        .sum()
}
