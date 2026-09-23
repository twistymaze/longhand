//! Speech recognition (KTD6): `transcribe-cpp` behind a small engine, and the
//! VAD windowing that decides what each recognition call hears.

pub mod engine;
pub mod windows;

pub use engine::{AsrError, AsrSegment, Engine, Recognizer, WindowText, pad_to_minimum};
pub use windows::{MAX_WINDOW_MS, Span, Voicing, plan_windows, voiced_ms};

/// The rate recognition consumes: mono 16 kHz, as the processing tap is.
pub const SAMPLE_RATE: u32 = vad::VAD_SAMPLE_RATE;
pub const SAMPLES_PER_MS: u64 = SAMPLE_RATE as u64 / 1000;
