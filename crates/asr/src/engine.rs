//! The recognition engine (KTD6). One `transcribe-cpp` model and session,
//! loaded once and reused for every window of every queued session.
//!
//! Backend start-up follows Handy's `init_transcribe_backend`; see
//! PROVENANCE.md. Unlike Handy, results keep their segments rather than
//! collapsing to text, because the join and the session reading surface need
//! times, and windows are padded to the minimum Whisper returns text for.

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
        let is_whisper = model.arch().to_ascii_lowercase().contains("whisper");
        let streaming = streams(
            model_id,
            catalog_streaming,
            model.capabilities().supports_streaming,
        );
        let native_timing = native_timing_kind(model.capabilities().max_timestamp_kind);
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
            part.transcript
                .words
                .iter()
                .filter_map(|word| {
                    let text = word.text.trim();
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

fn text_from_parts(parts: Vec<crate::recovery::DecodedPart>) -> WindowText {
    let mut result = WindowText::default();
    for part in parts {
        let limit_ms = part.samples as u64 / SAMPLES_PER_MS;
        let offset_ms = part.offset as u64 / SAMPLES_PER_MS;
        crate::recovery::append_text(&mut result.text, &part.transcript.text);
        for segment in part.transcript.segments {
            let text = segment.text.trim().to_string();
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

fn run_error(e: transcribe_cpp::Error) -> AsrError {
    AsrError::Run(e.to_string())
}

/// Tokens with text, with times clamped into `[0, limit_ms]`. Special
/// tokens such as `<|endoftext|>` are not speech.
fn timed_tokens(tokens: &[transcribe_cpp::Token], limit_ms: u64) -> Vec<TimedToken> {
    tokens
        .iter()
        .filter(|t| {
            let text = t.text.trim();
            let special = text.starts_with("<|") && text.ends_with("|>");
            !text.is_empty() && !special
        })
        .map(|t| {
            let start = (t.t0_ms.max(0) as u64).min(limit_ms);
            TimedToken {
                text: t.text.clone(),
                start_ms: start,
                end_ms: (t.t1_ms.max(0) as u64).clamp(start, limit_ms),
            }
        })
        .collect()
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
        let text = self.stream.text();
        LiveText {
            committed: text.committed,
            tentative: text.tentative,
        }
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
