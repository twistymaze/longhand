//! Speech bounded by digital silence is found where it is and nowhere else.
//!
//! The speech is real far-field meeting audio (see tests/fixtures/README.md),
//! so it has genuine pauses inside it and the detector correctly reports more
//! than one voiced run. What is checked is the property ASR windowing and the
//! voiced-duration measure rely on: the voiced extent sits within 200 ms of
//! the speech on both sides, and nothing in the silence is voiced.

use vad::{
    EarshotVad, SmoothedVad, VAD_OFFLINE_HANGOVER_MS, VAD_ONSET_MS, VAD_SAMPLE_RATE, VadFrame,
    VoiceActivityDetector, frames_for_duration_ms,
};

const FRAME: usize = 256;
const SILENCE_SECS: usize = 60;
const TOLERANCE_SECS: f64 = 0.2;

fn load_fixture() -> Vec<f32> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ami-sdm-speech.wav"
    );
    let reader = hound::WavReader::open(path).expect("fixture present");
    assert_eq!(reader.spec().sample_rate, VAD_SAMPLE_RATE);
    reader
        .into_samples::<i16>()
        .map(|s| s.expect("valid sample") as f32 / 32_768.0)
        .collect()
}

/// Voiced runs in seconds. A run's end is the last frame the detector itself
/// judged voiced: the hangover frames the smoother appends after it are
/// padding, not detection, so they are taken off again.
fn voiced_runs(samples: &[f32]) -> Vec<(f64, f64)> {
    let onset = frames_for_duration_ms(VAD_ONSET_MS, FRAME);
    let hangover = frames_for_duration_ms(VAD_OFFLINE_HANGOVER_MS, FRAME);
    // Pre-roll of onset - 1 frames makes the first emitted frame the first voiced one.
    let mut vad = SmoothedVad::new(
        Box::new(EarshotVad::new(0.5).unwrap()),
        onset - 1,
        hangover,
        onset,
    );
    let frame_secs = FRAME as f64 / VAD_SAMPLE_RATE as f64;
    let hangover_secs = hangover as f64 * frame_secs;

    let mut runs = Vec::new();
    let mut open: Option<f64> = None;
    let mut last_emitted_end = 0.0;
    for (i, frame) in samples.chunks_exact(FRAME).enumerate() {
        let frame_end = (i + 1) as f64 * frame_secs;
        match vad.push_frame(frame).unwrap() {
            VadFrame::Speech(out) => {
                open.get_or_insert(frame_end - out.len() as f64 / VAD_SAMPLE_RATE as f64);
                last_emitted_end = frame_end;
            }
            VadFrame::Noise => {
                if let Some(start) = open.take() {
                    runs.push((start, last_emitted_end - hangover_secs));
                }
            }
        }
    }
    if let Some(start) = open {
        runs.push((start, last_emitted_end));
    }
    runs
}

#[test]
fn voiced_extent_matches_speech_bounded_by_silence() {
    let speech = load_fixture();
    let silence = vec![0.0_f32; SILENCE_SECS * VAD_SAMPLE_RATE as usize];
    let input: Vec<f32> = [&silence[..], &speech[..], &silence[..]].concat();

    let speech_start = SILENCE_SECS as f64;
    let speech_end = speech_start + speech.len() as f64 / VAD_SAMPLE_RATE as f64;

    let runs = voiced_runs(&input);
    assert!(!runs.is_empty(), "no speech detected");
    let (first, last) = (runs[0].0, runs[runs.len() - 1].1);

    assert!(
        (first - speech_start).abs() <= TOLERANCE_SECS,
        "voiced extent starts at {first:.3}s, speech at {speech_start:.3}s ({runs:?})"
    );
    assert!(
        (last - speech_end).abs() <= TOLERANCE_SECS,
        "voiced extent ends at {last:.3}s, speech at {speech_end:.3}s ({runs:?})"
    );
    for (start, end) in &runs {
        assert!(
            *start >= speech_start - TOLERANCE_SECS && *end <= speech_end + TOLERANCE_SECS,
            "voiced run {start:.3}-{end:.3}s falls in the silence"
        );
    }
}
