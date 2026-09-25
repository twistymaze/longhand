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
    Model, ModelOptions, RunExtension, RunOptions, Session, StreamOptions, TimestampKind,
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
}

impl Engine {
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
        })
    }

    /// Transcribe one window with token times, in milliseconds from the
    /// start of the window. Build words with
    /// [`words_from_tokens`](crate::words_from_tokens).
    pub fn transcribe_timed(&mut self, pcm: &[f32]) -> Result<Vec<TimedToken>, AsrError> {
        let original_ms = pcm.len() as u64 / SAMPLES_PER_MS;
        let audio = pad_to_minimum(pcm);
        let options = RunOptions {
            timestamps: TimestampKind::Token,
            family: self.family(),
            ..Default::default()
        };
        let transcript = self.session.run(&audio, &options).map_err(run_error)?;
        Ok(timed_tokens(&transcript.tokens, original_ms))
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
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn transcribe(&mut self, pcm: &[f32]) -> Result<WindowText, AsrError> {
        let original_ms = pcm.len() as u64 / SAMPLES_PER_MS;
        let audio = pad_to_minimum(pcm);
        let options = RunOptions {
            timestamps: TimestampKind::Segment,
            family: self.family(),
            ..Default::default()
        };
        let transcript = self.session.run(&audio, &options).map_err(run_error)?;
        let segments = transcript
            .segments
            .into_iter()
            .filter_map(|s| {
                let text = s.text.trim().to_string();
                if text.is_empty() {
                    return None;
                }
                // Padding is silence the speaker never said; keep times
                // inside the audio that was recorded.
                let start = (s.t0_ms.max(0) as u64).min(original_ms);
                let end = (s.t1_ms.max(0) as u64).clamp(start, original_ms);
                Some(AsrSegment {
                    start_ms: start,
                    end_ms: end,
                    text,
                })
            })
            .collect();
        Ok(WindowText {
            text: transcript.text.trim().to_string(),
            segments,
        })
    }

    fn transcribe_tokens(&mut self, pcm: &[f32]) -> Result<Vec<TimedToken>, AsrError> {
        self.transcribe_timed(pcm)
    }

    fn start_stream(&mut self) -> Result<Option<Box<dyn LiveStream + '_>>, AsrError> {
        if !self.streaming {
            return Ok(None);
        }
        let options = RunOptions {
            timestamps: TimestampKind::Token,
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
