//! The recognition engine (KTD6). One `transcribe-cpp` model and session,
//! loaded once and reused for every window of every queued session.
//!
//! Backend start-up follows Handy's `init_transcribe_backend`; see
//! PROVENANCE.md. Unlike Handy, results keep their segments rather than
//! collapsing to text, because the join and the session reading surface need
//! times, and windows are padded to the minimum Whisper returns text for.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Once;

use serde::{Deserialize, Serialize};
use transcribe_cpp::{
    Diarize, Model, ModelOptions, RunExtension, RunOptions, Session, StreamOptions, TimestampKind,
    WhisperRunOptions,
};

use crate::SAMPLES_PER_MS;
use crate::stream::{LiveStream, LiveText, TimedToken};

/// Whisper returns zero segments, and no error, for input under a second
/// (KTD6); shorter windows are zero-padded to this length.
pub const MIN_WINDOW_MS: u64 = 1_000;

#[derive(Debug, thiserror::Error)]
pub enum AsrError {
    #[error("the recognition model could not be loaded from {path}: {message}")]
    Load { path: String, message: String },
    #[error("recognition failed: {0}")]
    Run(String),
}

/// One ASR segment, in milliseconds from the start of its window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsrSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WindowText {
    pub text: String,
    pub segments: Vec<AsrSegment>,
}

/// What the pipeline needs of recognition, so its stages are testable
/// without a model on disk.
pub trait Recognizer: Send {
    /// The catalog id of the loaded model.
    fn model_id(&self) -> &str;
    /// True when the loaded model supplies native word or token timestamps.
    /// The default token adapter estimates times from segments.
    fn native_token_timestamps(&self) -> bool {
        false
    }
    /// Transcribe one window of mono 16 kHz audio.
    fn transcribe(&mut self, pcm: &[f32]) -> Result<WindowText, AsrError>;
    /// Transcribe one window with token times, in milliseconds from the
    /// start of the window, for a gap fill after Stop (KTD6). Build words
    /// with [`words_from_tokens`](crate::words_from_tokens). The default
    /// divides the time of each segment of [`Recognizer::transcribe`]
    /// equally between the segment's words.
    fn transcribe_tokens(&mut self, pcm: &[f32]) -> Result<Vec<TimedToken>, AsrError> {
        Ok(segment_tokens(&self.transcribe(pcm)?.segments))
    }
    /// Start a live stream of mono 16 kHz audio (KTD5). `None` when the
    /// model does not stream. The stream borrows the recognizer, so the
    /// recognizer does no batch work while a stream is open.
    fn start_stream(&mut self) -> Result<Option<Box<dyn LiveStream + '_>>, AsrError> {
        Ok(None)
    }
}

/// One token for each word of `segments`, each word with an equal part of
/// its segment's time.
fn segment_tokens(segments: &[AsrSegment]) -> Vec<TimedToken> {
    let mut tokens = Vec::new();
    for segment in segments {
        let words: Vec<&str> = segment.text.split_whitespace().collect();
        let n = words.len() as u64;
        let length = segment.end_ms.saturating_sub(segment.start_ms);
        for (i, word) in (0u64..).zip(&words) {
            tokens.push(TimedToken {
                text: format!(" {word}"),
                start_ms: segment.start_ms + length * i / n,
                end_ms: segment.start_ms + length * (i + 1) / n,
            });
        }
    }
    tokens
}

/// Zero-pad `pcm` to [`MIN_WINDOW_MS`]; longer input is returned unchanged.
pub fn pad_to_minimum(pcm: &[f32]) -> std::borrow::Cow<'_, [f32]> {
    let min = (MIN_WINDOW_MS * SAMPLES_PER_MS) as usize;
    if pcm.len() >= min {
        return std::borrow::Cow::Borrowed(pcm);
    }
    let mut padded = Vec::with_capacity(min);
    padded.extend_from_slice(pcm);
    padded.resize(min, 0.0);
    std::borrow::Cow::Owned(padded)
}

static BACKENDS: Once = Once::new();

/// Route native diagnostics into `log` and register compute backends. A
/// no-op in a static (Metal) build; loads the per-ISA modules in a
/// `dynamic-backends` build. Must run before the first model load.
fn init_backends() {
    BACKENDS.call_once(|| {
        transcribe_cpp::init_logging();
        match transcribe_cpp::init_backends_default() {
            Ok(()) => log::info!(
                "transcribe-cpp initialized with {} compute device(s)",
                transcribe_cpp::device_count()
            ),
            Err(e) => log::warn!("failed to initialize transcribe-cpp backends: {e}"),
        }
    });
}

/// Whether a model streams: the catalog flag and the model's own capability
/// must agree (KTD5). A disagreement logs a warning, and the model does not
/// stream, so the note gets a full pass after Stop.
pub fn streams(model_id: &str, catalog: bool, model: bool) -> bool {
    if catalog != model {
        log::warn!(
            "{model_id}: the catalog says streaming is {catalog}, the model says {model}; \
             the model does not stream"
        );
    }
    catalog && model
}

pub struct Engine {
    model_id: String,
    session: Session,
    is_whisper: bool,
    streaming: bool,
    native_timing: Option<TimestampKind>,
    language: Option<String>,
}

impl Engine {
    /// Set the source language hint from the note configuration.
    pub fn set_language(&mut self, language: Option<String>) {
        self.language = language;
    }

    /// Load a model for batch recognition only.
    pub fn load(model_id: &str, path: &Path) -> Result<Self, AsrError> {
        Self::load_with(model_id, path, false)
    }

    /// Load a model. `catalog_streaming` is the catalog's `streaming` flag;
    /// the model streams only when its capabilities agree (KTD5).
    pub fn load_with(
        model_id: &str,
        path: &Path,
        catalog_streaming: bool,
    ) -> Result<Self, AsrError> {
        init_backends();
        let load_error = |e: transcribe_cpp::Error| AsrError::Load {
            path: path.display().to_string(),
            message: e.to_string(),
        };
        let model = Model::load_with(path, &ModelOptions::default()).map_err(load_error)?;
        let capabilities = model.capabilities().map_err(load_error)?;
        let is_whisper = model.arch().to_ascii_lowercase().contains("whisper");
        let streaming = streams(model_id, catalog_streaming, capabilities.supports_streaming);
        let native_timing = native_timing_kind(capabilities.max_timestamp_kind);
        log::info!(
            "loaded {model_id} ({} on {})",
            model.arch(),
            model.backend()
        );
        let session = model.session().map_err(load_error)?;
        Ok(Self {
            model_id: model_id.to_string(),
            session,
            is_whisper,
            streaming,
            native_timing,
            language: None,
        })
    }

    /// Transcribe one window with token times, in milliseconds from the
    /// start of the window. Build words with
    /// [`words_from_tokens`](crate::words_from_tokens).
    pub fn transcribe_timed(&mut self, pcm: &[f32]) -> Result<Vec<TimedToken>, AsrError> {
        let options = RunOptions {
            language: self.language.clone(),
            timestamps: self.native_timing.unwrap_or(TimestampKind::Token),
            diarize: Diarize::Off,
            family: self.family(),
            ..Default::default()
        };
        let parts = self.decode_parts(pcm, &options)?;
        Ok(tokens_from_parts(parts, options.timestamps))
    }

    fn decode_parts(
        &mut self,
        pcm: &[f32],
        options: &RunOptions,
    ) -> Result<Vec<crate::recovery::DecodedPart>, AsrError> {
        crate::recovery::decode(pcm, &mut |audio| {
            self.session.run(&pad_to_minimum(audio), options)
        })
        .map_err(run_error)
    }

    /// Each window is decoded on its own; nothing decoded before a silence
    /// conditions what follows it (KTD6, the repetition-loop trap).
    fn family(&self) -> Option<RunExtension> {
        self.is_whisper.then(|| {
            RunExtension::Whisper(WhisperRunOptions {
                condition_on_prev_tokens: Some(false),
                ..Default::default()
            })
        })
    }
}

fn native_timing_kind(maximum: TimestampKind) -> Option<TimestampKind> {
    match maximum {
        TimestampKind::Word | TimestampKind::Token => Some(maximum),
        _ => None,
    }
}

fn tokens_from_parts(
    parts: Vec<crate::recovery::DecodedPart>,
    timing: TimestampKind,
) -> Vec<TimedToken> {
    let mut tokens = Vec::new();
    for part in parts {
        let offset_ms = part.offset as u64 / SAMPLES_PER_MS;
        let limit_ms = part.samples as u64 / SAMPLES_PER_MS;
        let local = if timing == TimestampKind::Word {
            word_tokens(&part.transcript.words, limit_ms)
        } else {
            timed_tokens(&part.transcript.tokens, limit_ms)
        };
        for mut token in local {
            token.start_ms += offset_ms;
            token.end_ms += offset_ms;
            tokens.push(token);
        }
    }
    tokens
}

/// The text that a NeMo model gives for a character outside its vocabulary.
/// transcribe-cpp 0.3.0 and later remove it; the removal here stays as a
/// guard. It is not speech.
const UNKNOWN: &str = "<unk>";

/// `text` without the unknown marker. Where a marker is removed, each run of
/// whitespace becomes its first character, so no double space stays and a
/// leading word boundary stays.
fn without_unknown(text: &str) -> Cow<'_, str> {
    if !text.contains(UNKNOWN) {
        return Cow::Borrowed(text);
    }
    let mut clean = String::with_capacity(text.len());
    for c in text.replace(UNKNOWN, "").chars() {
        if c.is_whitespace() && clean.ends_with(char::is_whitespace) {
            continue;
        }
        clean.push(c);
    }
    Cow::Owned(clean)
}

fn text_from_parts(parts: Vec<crate::recovery::DecodedPart>) -> WindowText {
    let mut result = WindowText::default();
    for part in parts {
        let limit_ms = part.samples as u64 / SAMPLES_PER_MS;
        let offset_ms = part.offset as u64 / SAMPLES_PER_MS;
        crate::recovery::append_text(&mut result.text, &without_unknown(&part.transcript.text));
        for segment in part.transcript.segments {
            let text = without_unknown(&segment.text).trim().to_string();
            if text.is_empty() {
                continue;
            }
            let start = (segment.t0_ms.max(0) as u64).min(limit_ms);
            let end = (segment.t1_ms.max(0) as u64).clamp(start, limit_ms);
            result.segments.push(AsrSegment {
                start_ms: offset_ms + start,
                end_ms: offset_ms + end,
                text,
            });
        }
    }
    result
}

/// Word rows as timed tokens, with times clamped into `[0, limit_ms]`.
fn word_tokens(words: &[transcribe_cpp::Word], limit_ms: u64) -> Vec<TimedToken> {
    words
        .iter()
        .filter_map(|word| {
            let text = without_unknown(&word.text);
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            let start_ms = (word.t0_ms.max(0) as u64).min(limit_ms);
            Some(TimedToken {
                // The adapter exposes complete words through the token
                // interface. Keep a word boundary for words_from_tokens.
                text: format!(" {text}"),
                start_ms,
                end_ms: (word.t1_ms.max(0) as u64).clamp(start_ms, limit_ms),
            })
        })
        .collect()
}

/// The timed words of a stream snapshot, in stream time. The returned
/// timing kind selects the collection to read. Other kinds give no words.
fn open_words(transcript: &transcribe_cpp::Transcript) -> Vec<crate::LiveWord> {
    let tokens = match transcript.timestamp_kind {
        TimestampKind::Token => timed_tokens(&transcript.tokens, u64::MAX),
        TimestampKind::Word => word_tokens(&transcript.words, u64::MAX),
        _ => return Vec::new(),
    };
    crate::words_from_tokens(&tokens, 0)
}

fn run_error(e: transcribe_cpp::Error) -> AsrError {
    AsrError::Run(e.to_string())
}

/// Tokens with text, with times clamped into `[0, limit_ms]`. Special
/// tokens such as `<|endoftext|>` and the unknown marker are not speech. A
/// token that keeps only a word boundary after the unknown marker is removed
/// stays, because the boundary starts the next word.
fn timed_tokens(tokens: &[transcribe_cpp::Token], limit_ms: u64) -> Vec<TimedToken> {
    tokens
        .iter()
        .filter_map(|t| {
            let text = without_unknown(&t.text);
            let trimmed = text.trim();
            let special = trimmed.starts_with("<|") && trimmed.ends_with("|>");
            let boundary = trimmed.is_empty() && !text.is_empty() && text != t.text.as_str();
            if special || (trimmed.is_empty() && !boundary) {
                return None;
            }
            let start = (t.t0_ms.max(0) as u64).min(limit_ms);
            Some(TimedToken {
                text: text.into_owned(),
                start_ms: start,
                end_ms: (t.t1_ms.max(0) as u64).clamp(start, limit_ms),
            })
        })
        .collect()
}

/// The live text without unknown markers. Where a marker was removed, the
/// tentative text does not start with a second space after the committed text.
fn live_text(text: transcribe_cpp::StreamText) -> LiveText {
    let committed = without_unknown(&text.committed);
    let mut tentative = without_unknown(&text.tentative);
    let removed = committed != text.committed.as_str() || tentative != text.tentative.as_str();
    if removed && committed.ends_with(char::is_whitespace) {
        tentative = Cow::Owned(tentative.trim_start().to_string());
    }
    LiveText {
        committed: committed.into_owned(),
        tentative: tentative.into_owned(),
    }
}

/// A `transcribe-cpp` stream on the engine's session. The stream holds the
/// model's compute lease from its start until it finishes, resets or drops.
struct EngineStream<'a> {
    stream: transcribe_cpp::Stream<'a>,
}

impl LiveStream for EngineStream<'_> {
    fn feed(&mut self, pcm: &[f32]) -> Result<bool, AsrError> {
        let update = self.stream.feed(pcm).map_err(run_error)?;
        Ok(update.committed_changed || update.tentative_changed)
    }

    fn text(&self) -> LiveText {
        live_text(self.stream.text())
    }

    fn finish(&mut self) -> Result<Vec<TimedToken>, AsrError> {
        self.stream.finalize().map_err(run_error)?;
        // A stream gives timed tokens only; its words and segments stay
        // empty, so words are built from the tokens.
        Ok(timed_tokens(&self.stream.snapshot().tokens, u64::MAX))
    }

    fn reset(&mut self) {
        self.stream.reset();
    }

    fn open_words(&self) -> Vec<crate::LiveWord> {
        open_words(&self.stream.snapshot())
    }
}

impl Recognizer for Engine {
    fn native_token_timestamps(&self) -> bool {
        self.native_timing.is_some()
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn transcribe(&mut self, pcm: &[f32]) -> Result<WindowText, AsrError> {
        let options = RunOptions {
            language: self.language.clone(),
            timestamps: TimestampKind::Segment,
            diarize: Diarize::Off,
            family: self.family(),
            ..Default::default()
        };
        Ok(text_from_parts(self.decode_parts(pcm, &options)?))
    }

    fn transcribe_tokens(&mut self, pcm: &[f32]) -> Result<Vec<TimedToken>, AsrError> {
        self.transcribe_timed(pcm)
    }

    fn start_stream(&mut self) -> Result<Option<Box<dyn LiveStream + '_>>, AsrError> {
        if !self.streaming {
            return Ok(None);
        }
        let options = RunOptions {
            language: self.language.clone(),
            timestamps: TimestampKind::Token,
            diarize: Diarize::Off,
            ..Default::default()
        };
        let stream = self
            .session
            .stream(&options, &StreamOptions::default())
            .map_err(run_error)?;
        Ok(Some(Box::new(EngineStream { stream })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_word_timing_keeps_models_on_the_uncut_audio_path() {
        assert_eq!(
            native_timing_kind(TimestampKind::Word),
            Some(TimestampKind::Word)
        );
        assert_eq!(
            native_timing_kind(TimestampKind::Token),
            Some(TimestampKind::Token)
        );
        assert_eq!(native_timing_kind(TimestampKind::Segment), None);
    }

    fn recovered_parts() -> Vec<crate::recovery::DecodedPart> {
        [" first", " second"]
            .into_iter()
            .enumerate()
            .map(|(index, text)| crate::recovery::DecodedPart {
                offset: index * 16_000,
                samples: if index == 0 { 16_000 } else { 16_003 },
                transcript: transcribe_cpp::Transcript {
                    text: text.into(),
                    segments: vec![transcribe_cpp::Segment {
                        t0_ms: 100,
                        t1_ms: 1_100,
                        text: text.into(),
                        ..Default::default()
                    }],
                    words: vec![transcribe_cpp::Word {
                        t0_ms: 100,
                        t1_ms: 1_100,
                        text: text.trim().into(),
                        ..Default::default()
                    }],
                    tokens: vec![transcribe_cpp::Token {
                        t0_ms: 100,
                        t1_ms: 1_100,
                        text: text.into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            })
            .collect()
    }

    #[test]
    fn native_word_rows_keep_boundaries_times_and_recovered_offsets() {
        let mut parts = recovered_parts();
        for part in &mut parts {
            part.transcript.tokens.clear();
        }
        let tokens = tokens_from_parts(parts, TimestampKind::Word);
        let words = crate::words_from_tokens(&tokens, 80_000);
        assert_eq!(crate::join_words(&words), "first second");
        assert_eq!(
            words
                .iter()
                .map(|w| (w.start_ms, w.end_ms))
                .collect::<Vec<_>>(),
            vec![(80_100, 81_000), (81_100, 82_000)]
        );
    }

    #[test]
    fn recovered_plain_and_token_output_keep_order_offsets_and_clipped_tails() {
        let text = text_from_parts(recovered_parts());
        assert_eq!(text.text, "first second");
        assert_eq!(
            text.segments,
            vec![
                AsrSegment {
                    start_ms: 100,
                    end_ms: 1_000,
                    text: "first".into()
                },
                AsrSegment {
                    start_ms: 1_100,
                    end_ms: 2_000,
                    text: "second".into()
                },
            ]
        );
        let tokens = tokens_from_parts(recovered_parts(), TimestampKind::Token);
        assert_eq!(
            tokens
                .iter()
                .map(|t| (t.text.as_str(), t.start_ms, t.end_ms))
                .collect::<Vec<_>>(),
            vec![(" first", 100, 1_000), (" second", 1_100, 2_000)]
        );
        assert_eq!(
            crate::join_words(&crate::words_from_tokens(&tokens, 0)),
            "first second"
        );
    }

    /// A NeMo model gives `<unk>` for a character outside its vocabulary.
    /// The marker is not speech, so no output path keeps it.
    #[test]
    fn unknown_markers_are_removed_from_text_segments_tokens_and_words() {
        let token = |text: &str, t0_ms| transcribe_cpp::Token {
            t0_ms,
            t1_ms: t0_ms + 100,
            text: text.into(),
            ..Default::default()
        };
        let word = |text: &str, t0_ms| transcribe_cpp::Word {
            t0_ms,
            t1_ms: t0_ms + 100,
            text: text.into(),
            ..Default::default()
        };
        let text = " the caf<unk> is <unk> open <unk>";
        let part = || crate::recovery::DecodedPart {
            offset: 0,
            samples: 16_000,
            transcript: transcribe_cpp::Transcript {
                text: text.into(),
                segments: vec![
                    transcribe_cpp::Segment {
                        t0_ms: 0,
                        t1_ms: 500,
                        text: text.into(),
                        ..Default::default()
                    },
                    transcribe_cpp::Segment {
                        t0_ms: 500,
                        t1_ms: 600,
                        text: " <unk> ".into(),
                        ..Default::default()
                    },
                ],
                tokens: vec![
                    token("\u{2581}the", 0),
                    token("\u{2581}caf", 100),
                    token("<unk>", 200),
                    token("\u{2581}is", 300),
                    token(" <unk>", 400),
                    token("open", 500),
                    token("\u{2581}<unk>", 600),
                    token("ok", 700),
                ],
                words: vec![
                    word("the", 0),
                    word("caf<unk>", 100),
                    word("is", 300),
                    word("<unk>", 400),
                    word("open", 500),
                    word("<unk>", 600),
                    word("ok", 700),
                ],
                ..Default::default()
            },
        };

        let plain = text_from_parts(vec![part()]);
        assert_eq!(plain.text, "the caf is open");
        assert_eq!(
            plain.segments,
            vec![AsrSegment {
                start_ms: 0,
                end_ms: 500,
                text: "the caf is open".into()
            }]
        );

        for timing in [TimestampKind::Token, TimestampKind::Word] {
            let tokens = tokens_from_parts(vec![part()], timing);
            assert!(
                tokens.iter().all(|t| !t.text.contains("<unk>")),
                "{timing:?}: {tokens:?}"
            );
            let words = crate::words_from_tokens(&tokens, 0);
            assert_eq!(
                spans(&words),
                vec![
                    (0, 100, "the"),
                    (100, 200, "caf"),
                    (300, 400, "is"),
                    (500, 600, "open"),
                    (700, 800, "ok")
                ],
                "{timing:?}"
            );
        }
    }

    #[test]
    fn live_text_and_plain_text_lose_unknown_markers_and_their_double_spaces() {
        let text = live_text(transcribe_cpp::StreamText {
            committed: "a <unk> caf<unk>".into(),
            tentative: "<unk> b".into(),
            ..Default::default()
        });
        assert_eq!(text.committed, "a caf");
        assert_eq!(text.tentative, " b");
        let text = live_text(transcribe_cpp::StreamText {
            committed: "hello <unk>".into(),
            tentative: " world".into(),
            ..Default::default()
        });
        assert_eq!(text.committed + &text.tentative, "hello world");
        assert_eq!(without_unknown("\u{2581}<unk>"), "\u{2581}");
        assert!(matches!(
            without_unknown("no  marker"),
            std::borrow::Cow::Borrowed("no  marker")
        ));
    }

    fn snapshot(kind: TimestampKind) -> transcribe_cpp::Transcript {
        let token = |text: &str, t0_ms, t1_ms| transcribe_cpp::Token {
            t0_ms,
            t1_ms,
            text: text.into(),
            ..Default::default()
        };
        transcribe_cpp::Transcript {
            timestamp_kind: kind,
            tokens: vec![
                token("\u{2581}hel", 0, 100),
                token("lo", 100, 200),
                token(" wor", 300, 400),
                token("ld,", 400, 500),
                token("<|endoftext|>", 500, 500),
            ],
            ..Default::default()
        }
    }

    fn spans(words: &[crate::LiveWord]) -> Vec<(u64, u64, &str)> {
        words
            .iter()
            .map(|w| (w.start_ms, w.end_ms, w.text.as_str()))
            .collect()
    }

    #[test]
    fn open_words_start_at_a_space_or_a_sentencepiece_marker() {
        let words = open_words(&snapshot(TimestampKind::Token));
        assert_eq!(spans(&words), vec![(0, 200, "hello"), (300, 500, "world,")]);
    }

    #[test]
    fn open_words_are_absent_when_the_returned_timing_is_not_token_or_word() {
        for kind in [
            TimestampKind::None,
            TimestampKind::Auto,
            TimestampKind::Segment,
        ] {
            assert!(open_words(&snapshot(kind)).is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn open_words_come_from_the_word_rows_when_the_returned_timing_is_word() {
        // Test the returned collection, not only the kind: a word-timed model
        // can return no token rows.
        let mut transcript = snapshot(TimestampKind::Word);
        transcript.tokens.clear();
        transcript.words = vec![
            transcribe_cpp::Word {
                t0_ms: 0,
                t1_ms: 200,
                text: "hello".into(),
                ..Default::default()
            },
            transcribe_cpp::Word {
                t0_ms: 300,
                t1_ms: 500,
                text: "world,".into(),
                ..Default::default()
            },
        ];
        assert_eq!(
            spans(&open_words(&transcript)),
            vec![(0, 200, "hello"), (300, 500, "world,")]
        );
    }

    #[test]
    fn a_short_window_is_padded_to_one_second_and_a_long_one_is_left_alone() {
        let short = vec![0.5f32; 700 * SAMPLES_PER_MS as usize];
        let padded = pad_to_minimum(&short);
        assert_eq!(padded.len(), 16_000);
        assert!(padded[..short.len()].iter().all(|&s| s == 0.5));
        assert!(padded[short.len()..].iter().all(|&s| s == 0.0));

        let long = vec![0.1f32; 20_000];
        assert!(matches!(
            pad_to_minimum(&long),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn segment_tokens_divide_each_segment_equally_between_its_words() {
        let segment = |start_ms, end_ms, text: &str| AsrSegment {
            start_ms,
            end_ms,
            text: text.into(),
        };
        let segments = [
            segment(1_000, 1_900, "one two three"),
            segment(3_000, 3_400, " four "),
            segment(4_000, 4_000, ""),
        ];
        let tokens = segment_tokens(&segments);
        let times: Vec<(&str, u64, u64)> = tokens
            .iter()
            .map(|t| (t.text.as_str(), t.start_ms, t.end_ms))
            .collect();
        assert_eq!(
            times,
            vec![
                (" one", 1_000, 1_300),
                (" two", 1_300, 1_600),
                (" three", 1_600, 1_900),
                (" four", 3_000, 3_400),
            ]
        );
        let words = crate::words_from_tokens(&tokens, 10_000);
        assert_eq!(words[0].start_ms, 11_000);
        assert_eq!(crate::join_words(&words), "one two three four");
    }
}
