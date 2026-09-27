//! Opt-in model checks. Output contains counts and times, never transcript text.
//! Set LONGHAND_ASR_MODEL, LONGHAND_ASR_WAV and optionally LONGHAND_ASR_SECONDS.
//! LONGHAND_ASR_MODE selects native (default), off, or live. Optional
//! LONGHAND_ASR_RESULT writes private JSON to a new file in a temporary directory.

use asr::{Attribution, Engine, RecognitionRequest, Recognizer, SpeakerScope, TimingPrecision};

struct BoundsLogger;
static BOUNDS_LOGGER: BoundsLogger = BoundsLogger;

impl log::Log for BoundsLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target() == "asr::result" && metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

#[test]
#[ignore = "requires a verified native speaker model and mono 16 kHz PCM16 audio"]
fn native_labels_cover_each_independent_window() {
    log::set_logger(&BOUNDS_LOGGER).expect("install numeric bounds logger");
    log::set_max_level(log::LevelFilter::Warn);
    let mode = std::env::var("LONGHAND_ASR_MODE").unwrap_or_else(|_| "native".into());
    assert!(
        ["native", "off", "live"].contains(&mode.as_str()),
        "invalid ASR mode"
    );
    let model = std::env::var("LONGHAND_ASR_MODEL").expect("model path");
    let audio = std::env::var("LONGHAND_ASR_WAV").expect("audio path");
    let seconds: usize = std::env::var("LONGHAND_ASR_SECONDS")
        .ok()
        .map(|value| value.parse().expect("seconds"))
        .unwrap_or(300);
    let mut reader = hound::WavReader::open(audio).expect("audio opens");
    assert_eq!(reader.spec().sample_rate, 16_000);
    assert_eq!(reader.spec().channels, 1);
    assert_eq!(reader.spec().bits_per_sample, 16);
    let pcm: Vec<_> = reader
        .samples::<i16>()
        .take(seconds * 16_000)
        .map(|sample| sample.unwrap() as f32 / 32_768.0)
        .collect();
    assert_eq!(
        pcm.len(),
        seconds * 16_000,
        "audio must contain the requested duration"
    );
    let mut engine =
        Engine::load_with("native-test", std::path::Path::new(&model), mode == "live").unwrap();
    if mode == "live" {
        let mut parts = asr::Parts::new(0, Default::default());
        let result = asr::stream_all(&mut engine, &mut parts, &pcm, 320)
            .expect("live recognition succeeds")
            .expect("selected artifact supports live text");
        assert!(
            !result
                .iter()
                .flat_map(|part| &part.words)
                .all(|word| word.text.trim().is_empty()),
            "known speech produced no live text"
        );
        assert_eq!(result.first().unwrap().start_ms, 0);
        assert_eq!(result.last().unwrap().end_ms, seconds as u64 * 1_000);
        eprintln!(
            "mode=live seconds={seconds} parts={} words={}",
            result.len(),
            result.iter().map(|part| part.words.len()).sum::<usize>()
        );
        write_result(&serde_json::json!({ "mode": mode, "seconds": seconds, "parts": result }));
        return;
    }
    let attribution = if mode == "native" {
        Attribution::Native
    } else {
        Attribution::Off
    };
    let mut snapshots = Vec::new();
    let mut next_sample = 0;
    let mut labels = 0;
    let mut rows = 0;
    for (epoch, chunk) in pcm.chunks(30 * 16_000).enumerate() {
        let request = RecognitionRequest {
            scope: SpeakerScope {
                generation: "native-test".into(),
                epoch: epoch as u64,
            },
            start_sample: next_sample,
            revision: 0,
            attribution,
        };
        let result = engine.transcribe_normalized(chunk, &request).unwrap();
        assert!(
            !result.text.trim().is_empty(),
            "known speech window {epoch} produced no text"
        );
        if attribution == Attribution::Off {
            assert!(result.segments.iter().all(|row| row.speaker.is_none()));
            assert!(result.speaker_turns.is_empty());
        }
        assert_eq!(result.coverage.start, next_sample);
        next_sample += chunk.len() as u64;
        assert_eq!(result.coverage.end, next_sample);
        for row in &result.segments {
            assert!(row.range.start >= result.coverage.start);
            assert!(row.range.end <= result.coverage.end);
            assert!(row.range.start < row.range.end);
            if row.timing == TimingPrecision::SourceWindow {
                assert_eq!(row.range, result.coverage);
            }
            if let Some(speaker) = &row.speaker {
                assert_eq!(speaker.scope, request.scope);
                labels += 1;
            }
        }
        rows += result.segments.len();
        eprintln!(
            "window={epoch} samples={} rows={} attributed={} source_window={}",
            chunk.len(),
            result.segments.len(),
            result
                .segments
                .iter()
                .filter(|row| row.speaker.is_some())
                .count(),
            result
                .segments
                .iter()
                .filter(|row| row.timing == TimingPrecision::SourceWindow)
                .count()
        );
        snapshots.push(result);
    }
    assert_eq!(next_sample, pcm.len() as u64);
    if attribution == Attribution::Native {
        assert!(labels > 0, "native model returned no attributed text");
        let off = engine
            .transcribe_normalized(
                &pcm[..pcm.len().min(30 * 16_000)],
                &RecognitionRequest {
                    scope: SpeakerScope {
                        generation: "off-test".into(),
                        epoch: 0,
                    },
                    start_sample: 0,
                    revision: 0,
                    attribution: Attribution::Off,
                },
            )
            .expect("Off transcribes without native attribution");
        assert!(
            !off.text.trim().is_empty(),
            "known speech produced no Off text"
        );
        assert!(off.segments.iter().all(|row| row.speaker.is_none()));
        assert!(off.speaker_turns.is_empty());
    }
    write_result(&serde_json::json!({ "mode": mode, "seconds": seconds, "windows": snapshots }));
    eprintln!("seconds={seconds} processed_samples={next_sample} rows={rows} attributed={labels}");
}

fn write_result(value: &serde_json::Value) {
    let Ok(path) = std::env::var("LONGHAND_ASR_RESULT") else {
        return;
    };
    let path = std::path::Path::new(&path);
    assert!(path.is_absolute(), "result path must be absolute");
    let parent = path
        .parent()
        .expect("result directory")
        .canonicalize()
        .expect("result directory exists");
    let temporary = std::env::temp_dir()
        .canonicalize()
        .expect("temporary directory");
    let system_temporary = std::path::Path::new("/tmp").canonicalize().ok();
    assert!(
        parent.starts_with(temporary)
            || system_temporary.is_some_and(|root| parent.starts_with(root)),
        "private results must be in a temporary directory"
    );
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    assert!(
        !parent.starts_with(repository),
        "private results must not be in the repository"
    );
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .expect("create a new private result file");
    serde_json::to_writer(file, value).expect("write private result");
}
