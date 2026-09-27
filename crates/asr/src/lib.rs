//! Speech recognition (KTD6): `transcribe-cpp` behind a small engine, and the
//! VAD windowing that decides what each recognition call hears. Live
//! recognition streams audio in parts (KTD4, KTD5).

pub mod engine;
pub mod result;
pub mod stream;
pub mod windows;

pub use engine::{AsrError, AsrSegment, Engine, Recognizer, WindowText, pad_to_minimum};
pub use result::{
    Attribution, Finality, RecognitionId, RecognitionOutput, RecognitionRequest,
    RecognitionSegment, SampleRange, SpeakerKey, SpeakerScope, SpeakerTurn, TimingPrecision,
    normalize_transcript,
};
pub use stream::{
    Cut, Fed, LivePart, LiveStream, LiveText, LiveWord, PartPolicy, Parts, TimedToken, join_words,
    stream_all, unspaced, words_from_tokens,
};
pub use windows::{MAX_WINDOW_MS, Span, Voicing, plan_windows, voiced_ms};

/// The rate recognition consumes: mono 16 kHz, as the processing tap is.
pub const SAMPLE_RATE: u32 = vad::VAD_SAMPLE_RATE;
pub const SAMPLES_PER_MS: u64 = SAMPLE_RATE as u64 / 1000;
