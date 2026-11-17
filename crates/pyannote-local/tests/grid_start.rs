//! A part of a cut segment keeps the analysis windows of the original
//! segment on their original grid (KTD18 of the forgotten-recording plan).
use pyannote_local::segmentation::{WINDOW_SAMPLES, WindowActivity, frames_for_samples};
use pyannote_local::{Analysis, DiarizationConfig, WindowEmbedding, reconstruct};

const STEP: usize = 31_860;
const WINDOWS: usize = 20;

/// Windows `first..first + windows` of the original grid, with window
/// `first` at `grid_start`. Speaker 0 talks in the original frames
/// `speech`.
fn analysis(
    first: usize,
    windows: usize,
    total: usize,
    grid_start: i64,
    speech: std::ops::Range<usize>,
) -> Analysis {
    let frames = frames_for_samples(WINDOW_SAMPLES);
    Analysis {
        total,
        step: STEP,
        grid_start,
        activities: (first..first + windows)
            .map(|w| WindowActivity {
                frames: (0..frames)
                    .map(|f| [speech.contains(&(w * STEP / 270 + f)), false, false])
                    .collect(),
            })
            .collect(),
        embeddings: (0..windows)
            .map(|w| WindowEmbedding {
                window: w,
                local_speaker: 0,
                embedding: vec![1.0; 4],
                anchor: true,
            })
            .collect(),
    }
}

fn original(speech: std::ops::Range<usize>) -> Vec<pyannote_local::SpeakerSegment> {
    let total = (WINDOWS - 1) * STEP + WINDOW_SAMPLES;
    reconstruct(
        &DiarizationConfig::default(),
        &analysis(0, WINDOWS, total, 0, speech),
        &[0; WINDOWS],
        1,
    )
}

/// The turns of a part that starts at sample `edge` of the original
/// segment and keeps the original windows from `first`.
fn part(
    edge: usize,
    first: usize,
    speech: std::ops::Range<usize>,
) -> Vec<pyannote_local::SpeakerSegment> {
    let total = (WINDOWS - 1) * STEP + WINDOW_SAMPLES;
    let grid_start = (first * STEP) as i64 - edge as i64;
    reconstruct(
        &DiarizationConfig::default(),
        &analysis(first, WINDOWS - first, total - edge, grid_start, speech),
        &vec![0; WINDOWS - first],
        1,
    )
}

#[test]
fn a_part_keeps_original_windows_on_a_grid_that_starts_after_its_first_sample() {
    let edge = 3 * STEP + 5_000;
    // The windows that lie fully inside the part: original windows 4..20.
    // The speech starts after the start of window 4.
    let turns = part(edge, edge.div_ceil(STEP), 600..900);
    let shift = edge as f64 / 16_000.0;
    let original = original(600..900);
    assert!(
        (turns[0].start - (original[0].start - shift)).abs() < 0.02,
        "part start {} original start {} shift {shift}",
        turns[0].start,
        original[0].start
    );
    assert!((turns[0].end - (original[0].end - shift)).abs() < 0.02);
}

#[test]
fn a_window_that_starts_before_the_first_sample_votes_only_after_it() {
    // The part starts inside the speech. Window 3 starts 20 000 samples
    // before the part.
    let edge = 3 * STEP + 20_000;
    let turns = part(edge, edge / STEP, 400..700);
    let shift = edge as f64 / 16_000.0;
    let original = original(400..700);
    assert!(original[0].start < shift);
    // The turn starts at the first frame of the part.
    assert_eq!(
        turns[0].start,
        pyannote_local::segmentation::frame_start_sample(0) as f64 / 16_000.0
    );
    assert!(
        (turns[0].end - (original[0].end - shift)).abs() < 0.02,
        "part end {} original end {} shift {shift}",
        turns[0].end,
        original[0].end
    );
}
