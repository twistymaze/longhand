//! Opt-in model checks. Output contains counts and times, never transcript text.
//! Set LONGHAND_ASR_MODEL, LONGHAND_ASR_WAV and optionally LONGHAND_ASR_SECONDS.
//! LONGHAND_ASR_MODE selects off (default) or live. Off transcribes independent
//! 30-second windows after Stop; live streams the audio. Optional
//! LONGHAND_ASR_RESULT writes private JSON to a new file in a temporary directory.

use asr::{Engine, Recognizer};

#[test]
#[ignore = "requires a verified recognition model and mono 16 kHz PCM16 audio"]
fn recognition_covers_each_independent_window() {
    let mode = std::env::var("LONGHAND_ASR_MODE").unwrap_or_else(|_| "off".into());
    assert!(["off", "live"].contains(&mode.as_str()), "invalid ASR mode");
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
    const WINDOW_MS: u64 = 30_000;
    let mut windows = Vec::new();
    let mut rows = 0;
    let mut timed_words = 0;
    for (index, chunk) in pcm.chunks(WINDOW_MS as usize * 16).enumerate() {
        let start_ms = index as u64 * WINDOW_MS;
        let length_ms = chunk.len() as u64 / 16;
        let text = engine
            .transcribe(chunk)
            .expect("segment recognition succeeds");
        assert!(
            !text.text.trim().is_empty(),
            "known speech window {index} produced no text"
        );
        for row in &text.segments {
            assert!(row.start_ms <= row.end_ms);
            assert!(row.end_ms <= length_ms);
        }
        let tokens = engine
            .transcribe_tokens(chunk)
            .expect("token recognition succeeds");
        let words = asr::words_from_tokens(&tokens, start_ms);
        assert!(
            !words.iter().all(|word| word.text.trim().is_empty()),
            "known speech window {index} produced no timed words"
        );
        for word in &words {
            assert!(word.start_ms >= start_ms);
            assert!(word.start_ms <= word.end_ms);
            assert!(word.end_ms <= start_ms + length_ms);
        }
        eprintln!(
            "window={index} samples={} rows={} words={} native_timing={}",
            chunk.len(),
            text.segments.len(),
            words.len(),
            engine.native_token_timestamps()
        );
        rows += text.segments.len();
        timed_words += words.len();
        windows.push(serde_json::json!({
            "start_ms": start_ms,
            "end_ms": start_ms + length_ms,
            "text": text,
            "words": words,
        }));
    }
    write_result(&serde_json::json!({ "mode": mode, "seconds": seconds, "windows": windows }));
    eprintln!("mode=off seconds={seconds} rows={rows} words={timed_words}");
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
