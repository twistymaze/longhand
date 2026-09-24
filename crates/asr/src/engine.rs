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
    Model, ModelOptions, RunExtension, RunOptions, Session, TimestampKind, WhisperRunOptions,
};

use crate::SAMPLES_PER_MS;

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

pub struct Engine {
    model_id: String,
    session: Session,
    is_whisper: bool,
}

impl Engine {
    pub fn load(model_id: &str, path: &Path) -> Result<Self, AsrError> {
        init_backends();
        let load_error = |e: transcribe_cpp::Error| AsrError::Load {
            path: path.display().to_string(),
            message: e.to_string(),
        };
        let model = Model::load_with(path, &ModelOptions::default()).map_err(load_error)?;
        let is_whisper = model.arch().to_ascii_lowercase().contains("whisper");
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
        })
    }
}

impl Recognizer for Engine {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn transcribe(&mut self, pcm: &[f32]) -> Result<WindowText, AsrError> {
        let original_ms = pcm.len() as u64 / SAMPLES_PER_MS;
        let audio = pad_to_minimum(pcm);
        // Each window is decoded on its own; nothing decoded before a silence
        // conditions what follows it (KTD6, the repetition-loop trap).
        let family = self.is_whisper.then(|| {
            RunExtension::Whisper(WhisperRunOptions {
                condition_on_prev_tokens: Some(false),
                ..Default::default()
            })
        });
        let options = RunOptions {
            timestamps: TimestampKind::Segment,
            family,
            ..Default::default()
        };
        let transcript = self
            .session
            .run(&audio, &options)
            .map_err(|e| AsrError::Run(e.to_string()))?;
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
}
