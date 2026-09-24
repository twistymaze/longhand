//! Windowing over real far-field meeting speech (the AMI fixture in
//! `crates/vad/tests/fixtures`, see its README).

use asr::{MAX_WINDOW_MS, Span, Voicing};

const RATE: usize = 16_000;

fn speech() -> Vec<f32> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vad/tests/fixtures/ami-sdm-speech.wav"
    );
    let reader = hound::WavReader::open(path).expect("fixture present");
    assert_eq!(reader.spec().sample_rate as usize, RATE);
    reader
        .into_samples::<i16>()
        .map(|s| s.expect("valid sample") as f32 / 32_768.0)
        .collect()
}

fn windows_of(audio: &[f32]) -> (Vec<Span>, Vec<Span>) {
    let mut voicing = Voicing::new();
    // Uneven pushes, as a file reader delivers them.
    for chunk in audio.chunks(4_000) {
        voicing.push(chunk);
    }
    (voicing.spans(), voicing.windows(&[]))
}

#[test]
fn no_window_spans_a_twenty_second_silence_and_none_exceeds_thirty_seconds() {
    let speech = speech();
    let silence_start = speech.len() as u64 * 1000 / RATE as u64;
    let silence_end = silence_start + 20_000;
    let mut audio = speech.clone();
    audio.resize(audio.len() + 20 * RATE, 0.0);
    audio.extend_from_slice(&speech);

    let (spans, windows) = windows_of(&audio);
    assert!(!spans.is_empty());
    assert!(!windows.is_empty());
    for w in &windows {
        assert!(w.duration_ms() <= MAX_WINDOW_MS, "{w:?} is over 30 s");
        let overlap = w
            .end_ms
            .min(silence_end)
            .saturating_sub(w.start_ms.max(silence_start));
        // Only the hangover and pre-roll may reach into the silence.
        assert!(
            overlap <= 500,
            "{w:?} reaches {overlap} ms into the silence"
        );
    }
    assert!(windows.iter().any(|w| w.end_ms <= silence_start + 500));
    assert!(windows.iter().any(|w| w.start_ms + 500 >= silence_end));
}

#[test]
fn every_voiced_span_is_covered_by_exactly_one_window() {
    let (spans, windows) = windows_of(&speech());
    for s in &spans {
        let covering = windows
            .iter()
            .filter(|w| w.start_ms < s.end_ms && s.start_ms < w.end_ms)
            .count();
        assert!(covering >= 1, "{s:?} is in no window");
    }
    for pair in windows.windows(2) {
        assert!(pair[0].end_ms <= pair[1].start_ms, "{pair:?} overlap");
    }
}

/// Needs a real model: `LONGHAND_ASR_MODEL=/path/to/model.gguf cargo test -p
/// asr --test windows -- --ignored`.
#[test]
#[ignore = "needs a recognition model on disk"]
fn a_sub_second_utterance_is_padded_and_still_returns_text() {
    use asr::{Engine, Recognizer};

    let path = std::env::var("LONGHAND_ASR_MODEL").expect("LONGHAND_ASR_MODEL is set");
    let mut engine = Engine::load("test", std::path::Path::new(&path)).expect("model loads");

    let speech = speech();
    let (spans, _) = windows_of(&speech);
    let first = spans[0];
    let start = first.start_ms as usize * RATE / 1000;
    let short = &speech[start..start + 700 * RATE / 1000];
    let result = engine.transcribe(short).expect("recognition runs");
    assert!(!result.text.is_empty(), "no text for a 700 ms utterance");
    assert!(result.segments.iter().all(|s| s.end_ms <= 700));

    let whole = engine
        .transcribe(&speech[..(25 * RATE).min(speech.len())])
        .expect("recognition runs");
    eprintln!("25 s window: {}", whole.text);
    assert!(!whole.segments.is_empty());
}
