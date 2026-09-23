//! A real recording through the real runner and model (U8 verification).
//!
//! ```text
//! LONGHAND_E2E_WAV=/path/to/mono-16bit.wav \
//! LONGHAND_ASR_MODEL=/path/to/model \
//! LONGHAND_WEIGHTS_DIR=/path/to/diarization/weights \
//! cargo test -p pipeline --release --test end_to_end -- --ignored --nocapture
//! ```
//!
//! The WAV is fed through the chunk writer at its own rate, as capture
//! would, then assembled, diarized, transcribed and joined. Only counts and timings are
//! printed, never transcript text: the input may be a private recording.

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

struct RealModel(String);

impl EngineProvider for RealModel {
    fn ready(&self, _: &str) -> bool {
        true
    }
    fn load(&self, model_id: &str) -> Result<Box<dyn Recognizer>, String> {
        Engine::load(model_id, std::path::Path::new(&self.0))
            .map(|e| Box::new(e) as Box<dyn Recognizer>)
            .map_err(|e| e.to_string())
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
    let audio: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32_768.0)
        .collect();
    let seconds = audio.len() as f64 / spec.sample_rate as f64;

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

    let stages = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&stages);
    let started = Instant::now();
    let runner = Runner::new(Environment {
        store: Arc::clone(&store),
        working_root: root.path().to_path_buf(),
        engines: Box::new(RealModel(model)),
        suspended: Box::new(|| false),
        events: Box::new(move |e| {
            if let pipeline::RunnerEvent::StageStarted { stage, .. } = e {
                sink.lock().unwrap().push((stage, started.elapsed()));
            }
        }),
    });
    let (_, outcome) = runner.run_next().await.unwrap();
    let total = started.elapsed();
    assert_eq!(outcome, Outcome::Finished);

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

    let stages = stages.lock().unwrap();
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
}
