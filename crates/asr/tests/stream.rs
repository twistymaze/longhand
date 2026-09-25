//! Live recognition in parts (KTD4, KTD5): the token-to-word builder, the
//! part policy against a scripted stream, and ignored tests against real
//! models.
//!
//! ```text
//! LONGHAND_MODELS_DIR=/path/to/models \
//! cargo test -p asr --release --test stream -- --ignored --nocapture every_catalog_model
//!
//! LONGHAND_ASR_MODEL=/path/to/streaming-model.gguf \
//! cargo test -p asr --release --test stream -- --ignored --nocapture a_streamed
//! ```
//!
//! The real-model tests read the AMI fixture in `crates/vad/tests/fixtures`,
//! or `LONGHAND_ASR_WAV` (mono 16 kHz PCM16) when it is set. They print only
//! counts and times, never text: the input may be a private recording.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use asr::{
    AsrError, Cut, LivePart, LiveStream, LiveText, LiveWord, PartPolicy, Parts, Recognizer,
    TimedToken, WindowText, join_words, stream_all, words_from_tokens,
};

const RATE: u64 = 16_000;
const MS: u64 = RATE / 1000;
/// Capture delivers 20 ms frames.
const FRAME: usize = 320;

fn token(text: &str, start_ms: u64, end_ms: u64) -> TimedToken {
    TimedToken {
        text: text.into(),
        start_ms,
        end_ms,
    }
}

fn texts(words: &[LiveWord]) -> Vec<&str> {
    words.iter().map(|w| w.text.as_str()).collect()
}

#[test]
fn sub_word_tokens_join_into_words_with_the_first_and_last_token_times() {
    let tokens = [
        token(" hel", 0, 120),
        token("lo", 120, 260),
        token(" world", 300, 520),
        token(",", 520, 540),
    ];
    let words = words_from_tokens(&tokens, 10_000);
    assert_eq!(
        words,
        vec![
            LiveWord {
                start_ms: 10_000,
                end_ms: 10_260,
                text: "hello".into()
            },
            LiveWord {
                start_ms: 10_300,
                end_ms: 10_540,
                text: "world,".into()
            },
        ]
    );
    assert_eq!(join_words(&words), "hello world,");
}

#[test]
fn a_word_starts_at_a_sentencepiece_marker_and_the_first_token_starts_a_word() {
    let tokens = [
        token("It", 0, 80),
        token("\u{2581}work", 100, 200),
        token("s", 200, 240),
        token("", 240, 240),
    ];
    assert_eq!(texts(&words_from_tokens(&tokens, 0)), vec!["It", "works"]);
}

#[test]
fn tokens_without_spaces_become_one_timed_unit_each() {
    // A Japanese hypothesis, then a Chinese one, as Nemotron 3.5 returns them.
    let tokens = [
        token("今日", 0, 200),
        token("は", 200, 300),
        token("いい", 300, 450),
        token("天気", 450, 700),
        token("。", 700, 720),
        token("你好", 1_000, 1_300),
    ];
    let words = words_from_tokens(&tokens, 0);
    assert_eq!(
        texts(&words),
        vec!["今日", "は", "いい", "天気", "。", "你好"]
    );
    assert_eq!((words[3].start_ms, words[3].end_ms), (450, 700));
    assert_eq!(join_words(&words), "今日はいい天気。你好");
}

#[test]
fn a_recognizer_without_a_stream_returns_none() {
    struct Batch;
    impl Recognizer for Batch {
        fn model_id(&self) -> &str {
            "batch"
        }
        fn transcribe(&mut self, _: &[f32]) -> Result<WindowText, AsrError> {
            Ok(WindowText::default())
        }
    }
    assert!(Batch.start_stream().unwrap().is_none());
    let mut parts = Parts::new(0, PartPolicy::default());
    assert!(
        stream_all(&mut Batch, &mut parts, &[0.0; 3_200], FRAME)
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_catalog_flag_and_the_model_both_decide_if_a_model_streams() {
    assert!(asr::engine::streams("m", true, true));
    assert!(!asr::engine::streams("m", false, false));
    // A disagreement logs a warning, and the note gets a full pass.
    assert!(!asr::engine::streams("m", true, false));
    assert!(!asr::engine::streams("m", false, true));
}

// --- the part policy, against a scripted stream ----------------------------

/// Audio whose every sample holds its own segment sample index, so a
/// scripted stream knows which span of the segment it hears. Exact in f32
/// up to 2^24 samples (17 minutes).
fn clock(start_ms: u64, end_ms: u64) -> Vec<f32> {
    (start_ms * MS..end_ms * MS).map(|i| i as f32).collect()
}

/// A word the script says, in segment milliseconds.
#[derive(Clone)]
struct Said {
    start_ms: u64,
    end_ms: u64,
    text: String,
}

/// One word every 400 ms, 300 ms long, from `from_ms` to `to_ms`.
fn talk(from_ms: u64, to_ms: u64) -> Vec<Said> {
    (from_ms..to_ms)
        .step_by(400)
        .filter(|t| t + 300 <= to_ms)
        .map(|t| Said {
            start_ms: t,
            end_ms: t + 300,
            text: format!("w{t}"),
        })
        .collect()
}

struct Scripted {
    said: Arc<Vec<Said>>,
    streams: Arc<AtomicUsize>,
    /// Fail the feed that hears this segment time.
    fail_at_ms: Option<u64>,
}

impl Scripted {
    fn new(said: Vec<Said>) -> Self {
        Self {
            said: Arc::new(said),
            streams: Arc::default(),
            fail_at_ms: None,
        }
    }
}

impl Recognizer for Scripted {
    fn model_id(&self) -> &str {
        "scripted"
    }
    fn transcribe(&mut self, _: &[f32]) -> Result<WindowText, AsrError> {
        Err(AsrError::Run("batch is not scripted".into()))
    }
    fn start_stream(&mut self) -> Result<Option<Box<dyn LiveStream + '_>>, AsrError> {
        self.streams.fetch_add(1, Ordering::SeqCst);
        Ok(Some(Box::new(ScriptedStream {
            said: &self.said,
            fail_at_ms: self.fail_at_ms,
            first_ms: None,
            samples: 0,
        })))
    }
}

struct ScriptedStream<'a> {
    said: &'a [Said],
    fail_at_ms: Option<u64>,
    first_ms: Option<u64>,
    samples: u64,
}

impl ScriptedStream<'_> {
    fn heard(&self) -> (u64, u64) {
        let first = self.first_ms.unwrap_or(0);
        (first, first + self.samples / MS)
    }
}

impl LiveStream for ScriptedStream<'_> {
    fn feed(&mut self, pcm: &[f32]) -> Result<bool, AsrError> {
        let first = *self.first_ms.get_or_insert(pcm[0] as u64 / MS);
        self.samples += pcm.len() as u64;
        let now = first + self.samples / MS;
        if self.fail_at_ms.is_some_and(|t| t <= now) {
            return Err(AsrError::Run("stream exploded".into()));
        }
        Ok(true)
    }

    fn text(&self) -> LiveText {
        LiveText::default()
    }

    /// Every word the stream heard any of, split into two tokens. A word cut
    /// at either edge comes back clipped to the heard audio, as a real model
    /// guesses at a partial word.
    fn finish(&mut self) -> Result<Vec<TimedToken>, AsrError> {
        let (from, to) = self.heard();
        let mut tokens = Vec::new();
        for w in self
            .said
            .iter()
            .filter(|w| w.end_ms > from && w.start_ms < to)
        {
            let start = w.start_ms.max(from) - from;
            let end = w.end_ms.min(to) - from;
            let mid = (start + end) / 2;
            tokens.push(token(&format!(" {}", &w.text[..1]), start, mid));
            tokens.push(token(&w.text[1..], mid, end));
        }
        Ok(tokens)
    }

    fn reset(&mut self) {}
}

/// Voiced everywhere except inside the given pauses, read off the clock.
fn voiced_except(pauses: Vec<(u64, u64)>) -> impl FnMut(&[f32]) -> bool + Send {
    move |frame: &[f32]| {
        let t = frame[0] as u64 / MS;
        !pauses.iter().any(|&(a, b)| t >= a && t < b)
    }
}

fn run(said: Vec<Said>, pauses: Vec<(u64, u64)>, end_ms: u64) -> (Vec<LivePart>, Vec<Said>, usize) {
    let mut recognizer = Scripted::new(said.clone());
    let mut parts = Parts::with_classifier(0, PartPolicy::default(), voiced_except(pauses));
    let result = stream_all(&mut recognizer, &mut parts, &clock(0, end_ms), FRAME)
        .unwrap()
        .expect("the scripted recognizer streams");
    (result, said, recognizer.streams.load(Ordering::SeqCst))
}

fn all_words(parts: &[LivePart]) -> Vec<(u64, u64, String)> {
    parts
        .iter()
        .flat_map(|p| &p.words)
        .map(|w| (w.start_ms, w.end_ms, w.text.clone()))
        .collect()
}

fn expected(said: &[Said]) -> Vec<(u64, u64, String)> {
    said.iter()
        .map(|w| (w.start_ms, w.end_ms, w.text.clone()))
        .collect()
}

#[test]
fn a_part_does_not_end_at_a_pause_before_four_minutes() {
    let minute = 60_000;
    // Pauses at 1 and 3 minutes, and one of 700 ms at 4:30.
    let pauses = vec![
        (minute, minute + 1_000),
        (3 * minute, 3 * minute + 5_000),
        (270_000, 270_700),
    ];
    let mut said = talk(0, minute);
    said.extend(talk(minute + 1_000, 3 * minute));
    said.extend(talk(3 * minute + 5_000, 270_000));
    said.extend(talk(270_700, 300_000));
    let (parts, said, streams) = run(said, pauses, 300_000);

    assert_eq!(parts.len(), 2, "one cut, at the pause after four minutes");
    assert_eq!(streams, 2);
    assert_eq!(parts[0].start_ms, 0);
    // The cut comes when the pause reaches 600 ms, to within one frame.
    let cut = parts[0].end_ms;
    assert!((270_600..=270_640).contains(&cut), "cut at {cut} ms");
    // No replay after a pause: the next part starts where the first ended.
    assert_eq!(parts[1].start_ms, cut);
    assert_eq!(parts[1].end_ms, 300_000);
    // Word times in the second part include its offset in the segment.
    assert_eq!(parts[1].words[0].start_ms, 270_700);
    assert_eq!(all_words(&parts), expected(&said));
}

#[test]
fn a_pause_under_600_ms_does_not_end_a_part() {
    let (parts, _, _) = run(talk(0, 250_000), vec![(245_000, 245_500)], 250_000);
    assert_eq!(parts.len(), 1);
    assert_eq!((parts[0].start_ms, parts[0].end_ms), (0, 250_000));
}

#[test]
fn a_part_with_no_pause_ends_at_eight_minutes_and_the_replay_keeps_every_word_once() {
    let end = 17 * 60_000;
    // Words start 150 ms after each 400 ms step, so a word crosses each cut
    // and each replay start.
    let (parts, said, streams) = run(talk(150, end), vec![], end);

    assert_eq!(parts.len(), 3);
    assert_eq!(streams, 3);
    assert_eq!((parts[0].start_ms, parts[0].end_ms), (0, 480_000));
    // The next part hears the last 2 s of the first part again.
    assert_eq!(
        (parts[1].start_ms, parts[1].end_ms),
        (478_000, 958_000),
        "the eight minutes include the replay"
    );
    assert_eq!(parts[2].start_ms, 956_000);
    assert_eq!(parts[2].end_ms, end);
    // No word is lost and no word repeats, and no clipped guess survives.
    assert_eq!(all_words(&parts), expected(&said));
}

#[test]
fn the_part_limits_come_from_the_policy() {
    let policy = PartPolicy {
        min_ms: 10_000,
        max_ms: 20_000,
        pause_ms: 600,
        replay_ms: 2_000,
    };
    let mut said = talk(0, 12_000);
    said.extend(talk(13_000, 50_000));
    let mut recognizer = Scripted::new(said.clone());
    let mut parts = Parts::with_classifier(0, policy, voiced_except(vec![(12_000, 13_000)]));
    let result = stream_all(&mut recognizer, &mut parts, &clock(0, 50_000), FRAME)
        .unwrap()
        .unwrap();
    let spans: Vec<_> = result.iter().map(|p| (p.start_ms, p.end_ms)).collect();
    let pause_cut = spans[0].1;
    assert!(
        (12_600..=12_640).contains(&pause_cut),
        "cut at {pause_cut} ms"
    );
    assert_eq!(
        spans,
        vec![
            (0, pause_cut),
            (pause_cut, pause_cut + 20_000),
            (pause_cut + 18_000, 50_000)
        ]
    );
    // The limit cut splits the word at 32 600 ms; it is kept once, whole.
    assert_eq!(all_words(&result), expected(&said));
}

#[test]
fn a_part_starts_at_the_segment_time_it_is_given() {
    // A segment clock that does not start at zero: a part after a gap.
    let said = talk(600_000, 610_000);
    let mut recognizer = Scripted::new(said.clone());
    let mut parts = Parts::with_classifier(600_000, PartPolicy::default(), |_: &[f32]| true);
    let result = stream_all(&mut recognizer, &mut parts, &clock(600_000, 610_000), FRAME)
        .unwrap()
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!((result[0].start_ms, result[0].end_ms), (600_000, 610_000));
    assert_eq!(all_words(&result), expected(&said));
}

#[test]
fn restart_at_starts_the_next_part_after_a_gap_without_a_replay() {
    let said = talk(0, 30_000);
    let mut recognizer = Scripted::new(said.clone());
    let mut parts = Parts::with_classifier(0, PartPolicy::default(), |_: &[f32]| true);
    let mut out = Vec::new();
    {
        let mut stream = recognizer.start_stream().unwrap().unwrap();
        for f in clock(0, 10_000).chunks(FRAME) {
            assert_eq!(parts.feed(&mut *stream, f).unwrap().cut, None);
        }
        out.push(parts.finish(&mut *stream).unwrap());
    }
    // Frames from 10 s to 12 s were dropped.
    parts.restart_at(12_000);
    {
        let mut stream = recognizer.start_stream().unwrap().unwrap();
        for f in clock(12_000, 30_000).chunks(FRAME) {
            parts.feed(&mut *stream, f).unwrap();
        }
        out.push(parts.finish(&mut *stream).unwrap());
    }
    let spans: Vec<_> = out.iter().map(|p| (p.start_ms, p.end_ms)).collect();
    assert_eq!(spans, vec![(0, 10_000), (12_000, 30_000)]);
    let kept: Vec<_> = expected(&said)
        .into_iter()
        .filter(|w| w.1 <= 10_000 || w.0 >= 12_000)
        .collect();
    // The clipped guesses at the gap edges stay: nothing else covers them.
    let got = all_words(&out);
    for w in &kept {
        assert!(got.contains(w), "{w:?} is missing");
    }
    assert_eq!(parts.part_start_ms(), 30_000);
}

#[test]
fn a_stream_error_is_returned_and_the_next_part_starts_after_it() {
    let mut recognizer = Scripted::new(talk(0, 10_000));
    recognizer.fail_at_ms = Some(5_000);
    let mut parts = Parts::with_classifier(0, PartPolicy::default(), |_: &[f32]| true);
    let err = stream_all(&mut recognizer, &mut parts, &clock(0, 10_000), FRAME).unwrap_err();
    assert!(matches!(err, AsrError::Run(_)));
    // The part is lost; a new part can start at the frame after the error.
    parts.restart_at(5_020);
    assert_eq!(parts.part_start_ms(), 5_020);
}

#[test]
fn a_cut_is_reported_with_its_reason() {
    let policy = PartPolicy {
        min_ms: 1_000,
        max_ms: 3_000,
        pause_ms: 600,
        replay_ms: 2_000,
    };
    let first_cut = |pauses: Vec<(u64, u64)>| {
        let mut recognizer = Scripted::new(vec![]);
        let mut parts = Parts::with_classifier(0, policy, voiced_except(pauses));
        let mut stream = recognizer.start_stream().unwrap().unwrap();
        let mut at_ms = 0;
        for f in clock(0, 5_000).chunks(FRAME) {
            at_ms += 20;
            let fed = parts.feed(&mut *stream, f).unwrap();
            assert!(fed.changed);
            if let Some(cut) = fed.cut {
                return (cut, at_ms);
            }
        }
        panic!("no cut");
    };
    assert_eq!(first_cut(vec![(1_000, 2_000)]), (Cut::Pause, 1_620));
    assert_eq!(first_cut(vec![]), (Cut::Limit, 3_000));
}

// --- real models (ignored) ------------------------------------------------

fn fixture() -> Vec<f32> {
    let path = std::env::var("LONGHAND_ASR_WAV").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../vad/tests/fixtures/ami-sdm-speech.wav"
        )
        .into()
    });
    let reader = hound::WavReader::open(&path).expect("fixture present");
    let spec = reader.spec();
    assert_eq!(
        (spec.channels, spec.sample_rate, spec.bits_per_sample),
        (1, RATE as u32, 16)
    );
    reader
        .into_samples::<i16>()
        .map(|s| s.expect("valid sample") as f32 / 32_768.0)
        .collect()
}

fn catalog_entry(path: &std::path::Path) -> &'static models::CatalogModel {
    let name = path.file_name().unwrap().to_string_lossy();
    models::CATALOG
        .iter()
        .find(|m| m.kind == models::ModelKind::Asr && m.filename == name)
        .expect("the model file is a catalog recognition model")
}

#[test]
#[ignore = "needs recognition models on disk"]
fn every_catalog_model_transcribes_with_timestamps() {
    let dir = std::path::PathBuf::from(
        std::env::var("LONGHAND_MODELS_DIR").expect("LONGHAND_MODELS_DIR is set"),
    );
    let audio = fixture();
    let audio_ms = audio.len() as u64 / MS;
    let mut ran = 0;
    for m in models::CATALOG
        .iter()
        .filter(|m| m.kind == models::ModelKind::Asr)
    {
        if !models::is_present(&dir, m) {
            eprintln!("{}: not on disk, skipped", m.id);
            continue;
        }
        let started = std::time::Instant::now();
        let mut engine = asr::Engine::load_with(&m.id, &dir.join(&m.filename), m.streaming)
            .unwrap_or_else(|e| panic!("{}: {e}", m.id));
        // The catalog flag and the model agree for every catalog entry.
        assert_eq!(
            engine.start_stream().unwrap().is_some(),
            m.streaming,
            "{}",
            m.id
        );
        let result = engine.transcribe(&audio).unwrap();
        assert!(!result.text.is_empty(), "{}: no text", m.id);
        assert!(!result.segments.is_empty(), "{}: no segments", m.id);
        assert!(
            result
                .segments
                .iter()
                .all(|s| s.start_ms <= s.end_ms && s.end_ms <= audio_ms),
            "{}: a segment time is outside the audio",
            m.id
        );
        assert!(
            result.segments.iter().any(|s| s.end_ms > s.start_ms),
            "{}: no segment has a duration",
            m.id
        );
        eprintln!(
            "{}: {} segments, {} characters, last segment ends at {} ms of {audio_ms}, {:.1} s",
            m.id,
            result.segments.len(),
            result.text.chars().count(),
            result.segments.last().unwrap().end_ms,
            started.elapsed().as_secs_f64()
        );
        ran += 1;
    }
    assert!(ran > 0, "no catalog model is in {}", dir.display());
}

fn stream_words(engine: &mut asr::Engine, policy: PartPolicy, audio: &[f32]) -> Vec<LivePart> {
    let mut parts = Parts::new(0, policy);
    stream_all(engine, &mut parts, audio, FRAME)
        .unwrap()
        .expect("the model streams")
}

#[test]
#[ignore = "needs a streaming recognition model on disk"]
fn a_streamed_fixture_matches_batch_word_times_and_a_pause_cut_keeps_its_words() {
    let path = std::path::PathBuf::from(
        std::env::var("LONGHAND_ASR_MODEL").expect("LONGHAND_ASR_MODEL is set"),
    );
    let entry = catalog_entry(&path);
    assert!(entry.streaming, "{} does not stream", entry.id);
    let mut engine = asr::Engine::load_with(&entry.id, &path, true).unwrap();
    let speech = fixture();

    // Word times: one stream against one batch run over the same audio.
    let streamed: Vec<LiveWord> = stream_words(&mut engine, PartPolicy::default(), &speech)
        .into_iter()
        .flat_map(|p| p.words)
        .collect();
    let batch = words_from_tokens(&engine.transcribe_timed(&speech).unwrap(), 0);
    assert!(!streamed.is_empty() && !batch.is_empty());
    let mut offsets: Vec<u64> = Vec::new();
    for w in &streamed {
        let key = w.text.to_lowercase();
        if let Some(b) = batch
            .iter()
            .filter(|b| b.text.to_lowercase() == key)
            .min_by_key(|b| b.start_ms.abs_diff(w.start_ms))
        {
            offsets.push(b.start_ms.abs_diff(w.start_ms));
        }
    }
    offsets.sort_unstable();
    let median = offsets[offsets.len() / 2];
    let p90 = offsets[offsets.len() * 9 / 10];
    eprintln!(
        "{}: {} streamed words, {} batch words, {} matched; start offset median {median} ms, 90th percentile {p90} ms, max {} ms",
        entry.id,
        streamed.len(),
        batch.len(),
        offsets.len(),
        offsets.last().unwrap()
    );
    assert!(
        offsets.len() * 2 >= streamed.len(),
        "fewer than half the words match"
    );
    assert!(median <= 100, "median offset {median} ms");

    // A cut at a pause: speech, 2 s of silence, the same speech again.
    let mut audio = speech.clone();
    audio.resize(audio.len() + 2 * RATE as usize, 0.0);
    audio.extend_from_slice(&speech);
    let whole = stream_words(&mut engine, PartPolicy::default(), &audio);
    let speech_ms = speech.len() as u64 / MS;
    let cut_policy = PartPolicy {
        min_ms: speech_ms,
        ..PartPolicy::default()
    };
    let cut = stream_words(&mut engine, cut_policy, &audio);
    let count = |parts: &[LivePart]| parts.iter().map(|p| p.words.len()).sum::<usize>();
    eprintln!(
        "one stream: {} parts, {} words; cut at the pause: {} parts ending at {:?} ms, {} words",
        whole.len(),
        count(&whole),
        cut.len(),
        cut.iter().map(|p| p.end_ms).collect::<Vec<_>>(),
        count(&cut)
    );
    assert_eq!(whole.len(), 1);
    assert_eq!(cut.len(), 2, "the stream is cut at the pause");
    assert!(count(&whole).abs_diff(count(&cut)) <= 1);
}
