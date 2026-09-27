//! Backend-neutral recognition rows on the canonical 16 kHz audio timeline.

use serde::{Deserialize, Serialize};
use transcribe_cpp::{Diarize, Transcript};

use crate::{AsrError, SAMPLES_PER_MS};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attribution {
    Off,
    Native,
}

impl Attribution {
    pub(crate) fn diarize(self) -> Diarize {
        match self {
            Self::Off => Diarize::Off,
            Self::Native => Diarize::On,
        }
    }
}

/// A half-open interval on the canonical recording timeline, in samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerScope {
    pub generation: String,
    pub epoch: u64,
}

/// Batch calls do not establish speaker continuity between input windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerKey {
    pub scope: SpeakerScope,
    pub input_start_sample: u64,
    pub local_id: u32,
}

/// A replay of one input with the same scope keeps each ordered row's identity.
/// A revision replaces that input's complete row list, including removed rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognitionId {
    pub scope: SpeakerScope,
    pub input_start_sample: u64,
    pub ordinal: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Finality {
    Provisional,
    Final,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimingPrecision {
    /// Model-provided segment bounds. These are not word alignments.
    NativeSegment,
    /// A native segment whose end was clipped by an explicit adapter tolerance.
    /// This does not improve or guarantee the model's timestamp accuracy.
    NativeSegmentClipped,
    /// A model timestamp with no known duration, including a clipped tail word.
    /// Its range is a single point and contributes no timed speech coverage.
    NativeTimePoint,
    /// Only the complete source window is known. Row order remains meaningful.
    SourceWindow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognitionRequest {
    /// Use a new epoch after a restart. Never reuse a generation for new analysis.
    pub scope: SpeakerScope,
    pub start_sample: u64,
    pub revision: u64,
    pub attribution: Attribution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognitionSegment {
    pub id: RecognitionId,
    pub range: SampleRange,
    pub timing: TimingPrecision,
    pub text: String,
    pub speaker: Option<SpeakerKey>,
    pub revision: u64,
    pub text_finality: Finality,
    pub speaker_finality: Finality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerTurn {
    pub range: SampleRange,
    pub timing: TimingPrecision,
    pub speaker: SpeakerKey,
    pub finality: Finality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognitionOutput {
    /// Identifies the complete snapshot, including an empty replacement.
    pub scope: SpeakerScope,
    pub revision: u64,
    /// Input accepted by the completed call, including silence. This does not
    /// prove that the model decoded every spoken word or rule out truncation.
    pub coverage: SampleRange,
    pub text: String,
    pub segments: Vec<RecognitionSegment>,
    pub speaker_turns: Vec<SpeakerTurn>,
}

impl RecognitionRequest {
    pub(crate) fn coverage(&self, samples: usize) -> Result<SampleRange, AsrError> {
        if self.scope.generation.is_empty() {
            return Err(AsrError::Run("recognition generation is empty".into()));
        }
        let end = self
            .start_sample
            .checked_add(samples as u64)
            .ok_or_else(|| AsrError::Run("recognition sample range overflow".into()))?;
        Ok(SampleRange {
            start: self.start_sample,
            end,
        })
    }

    fn speaker(&self, local_id: i32) -> Option<SpeakerKey> {
        (self.attribution == Attribution::Native && local_id > 0).then(|| SpeakerKey {
            scope: self.scope.clone(),
            input_start_sample: self.start_sample,
            local_id: local_id as u32,
        })
    }
}

/// Preserve native batch output. Untimed rows use the whole input window;
/// they are never divided into invented word or speaker times.
pub fn normalize_transcript(
    transcript: Transcript,
    samples: usize,
    request: &RecognitionRequest,
) -> Result<RecognitionOutput, AsrError> {
    normalize_with_end_tolerance(transcript, samples, request, 0)
}

pub(crate) fn normalize_moss_transcript(
    transcript: Transcript,
    samples: usize,
    request: &RecognitionRequest,
) -> Result<RecognitionOutput, AsrError> {
    // The pinned MOSS runtime returned 30020 ms for a 30000 ms input.
    // Its parser rounds decimal timestamps to milliseconds; it specifies no
    // 20 ms quantization. This is an observed adapter tolerance, not a model rule.
    normalize_with_end_tolerance(transcript, samples, request, 20)
}

pub(crate) fn normalize_multitalker_transcript(
    transcript: Transcript,
    samples: usize,
    request: &RecognitionRequest,
) -> Result<RecognitionOutput, AsrError> {
    // The pinned bundle has subsampling=8, hop=160 and sample_rate=16000.
    // parakeet/model.cpp builds times on this 80 ms encoder frame grid.
    // multitalker.cpp retains the terminal encoder frame beyond the input.
    // Only an end on that grid can use the one-frame tail allowance.
    const FRAME_MS: u64 = 80;
    let padded_samples = (samples as u64).max(crate::engine::MIN_WINDOW_MS * SAMPLES_PER_MS);
    for end_ms in transcript
        .segments
        .iter()
        .filter(|row| !row.text.trim().is_empty())
        .map(|row| row.t1_ms)
        .chain(
            transcript
                .speaker_segments
                .iter()
                .filter(|row| request.attribution == Attribution::Native && row.speaker_id > 0)
                .map(|row| row.t1_ms),
        )
    {
        if end_ms >= 0
            && (end_ms as u64).saturating_mul(SAMPLES_PER_MS) > padded_samples
            && !(end_ms as u64).is_multiple_of(FRAME_MS)
        {
            return Err(AsrError::Run(format!(
                "Multitalker segment end is outside its frame grid: end_ms={end_ms}, input_samples={samples}, frame_ms={FRAME_MS}"
            )));
        }
    }
    normalize_with_end_tolerance(transcript, samples, request, FRAME_MS)
}

fn normalize_with_end_tolerance(
    transcript: Transcript,
    samples: usize,
    request: &RecognitionRequest,
    end_tolerance_ms: u64,
) -> Result<RecognitionOutput, AsrError> {
    let coverage = request.coverage(samples)?;
    let mut segments = Vec::new();
    for (ordinal, row) in transcript.segments.into_iter().enumerate() {
        let text = row.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        let (range, timing) =
            segment_range_with_tolerance(row.t0_ms, row.t1_ms, coverage, end_tolerance_ms)?;
        segments.push(RecognitionSegment {
            id: RecognitionId {
                scope: request.scope.clone(),
                input_start_sample: request.start_sample,
                ordinal: ordinal as u64,
            },
            range,
            timing,
            text,
            speaker: request.speaker(row.speaker_id),
            revision: request.revision,
            text_finality: Finality::Final,
            speaker_finality: Finality::Final,
        });
    }
    let text = transcript.text.trim().to_string();
    if segments.is_empty() && !text.is_empty() {
        let (range, timing) = segment_range(0, 0, coverage)?;
        segments.push(RecognitionSegment {
            id: RecognitionId {
                scope: request.scope.clone(),
                input_start_sample: request.start_sample,
                ordinal: 0,
            },
            range,
            timing,
            text: text.clone(),
            speaker: None,
            revision: request.revision,
            text_finality: Finality::Final,
            speaker_finality: Finality::Final,
        });
    }
    let mut speaker_turns = Vec::new();
    for turn in transcript.speaker_segments {
        if let Some(speaker) = request.speaker(turn.speaker_id) {
            let (range, timing) =
                segment_range_with_tolerance(turn.t0_ms, turn.t1_ms, coverage, end_tolerance_ms)?;
            speaker_turns.push(SpeakerTurn {
                range,
                timing,
                speaker,
                finality: Finality::Final,
            });
        }
    }
    Ok(RecognitionOutput {
        scope: request.scope.clone(),
        revision: request.revision,
        coverage,
        text,
        segments,
        speaker_turns,
    })
}

fn segment_range(
    start_ms: i64,
    end_ms: i64,
    coverage: SampleRange,
) -> Result<(SampleRange, TimingPrecision), AsrError> {
    segment_range_with_tolerance(start_ms, end_ms, coverage, 0)
}

fn segment_range_with_tolerance(
    start_ms: i64,
    end_ms: i64,
    coverage: SampleRange,
    end_tolerance_ms: u64,
) -> Result<(SampleRange, TimingPrecision), AsrError> {
    let invalid = || {
        AsrError::Run(format!(
            "recognition returned invalid segment times: start_ms={start_ms}, end_ms={end_ms}, input_start_sample={}, input_end_sample={}, samples_per_ms={SAMPLES_PER_MS}",
            coverage.start, coverage.end,
        ))
    };
    if coverage.start == coverage.end || start_ms < 0 || end_ms < start_ms {
        return Err(invalid());
    }
    if start_ms == 0 && end_ms == 0 {
        return Ok((coverage, TimingPrecision::SourceWindow));
    }
    let start = (start_ms as u64)
        .checked_mul(SAMPLES_PER_MS)
        .ok_or_else(invalid)?;
    let end = (end_ms as u64)
        .checked_mul(SAMPLES_PER_MS)
        .ok_or_else(invalid)?;
    let length = coverage.end - coverage.start;
    if start >= length || end <= start {
        return Err(invalid());
    }
    // One-second engine padding is separate from the observed MOSS tolerance.
    let padded_length = length.max(crate::engine::MIN_WINDOW_MS * SAMPLES_PER_MS);
    if end > padded_length.saturating_add(end_tolerance_ms * SAMPLES_PER_MS) {
        return Err(invalid());
    }
    let timing = if end > padded_length {
        log::warn!(
            "clipped native segment end: start_ms={start_ms}, end_ms={end_ms}, input_start_sample={}, input_end_sample={}, tolerance_ms={end_tolerance_ms}",
            coverage.start,
            coverage.end
        );
        TimingPrecision::NativeSegmentClipped
    } else {
        TimingPrecision::NativeSegment
    };
    Ok((
        SampleRange {
            start: coverage.start + start,
            end: coverage.start + end.min(length),
        },
        timing,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_requests_attribution_and_off_explicitly_disables_it() {
        assert_eq!(Attribution::Native.diarize(), Diarize::On);
        assert_eq!(Attribution::Off.diarize(), Diarize::Off);
    }

    #[test]
    fn moss_observed_end_tolerance_is_explicit_and_bounded() {
        let input = SampleRange {
            start: 2_400_000,
            end: 2_880_000,
        };
        let (range, precision) = segment_range_with_tolerance(27_620, 30_020, input, 20).unwrap();
        assert_eq!(
            range,
            SampleRange {
                start: 2_841_920,
                end: 2_880_000
            }
        );
        assert_eq!(precision, TimingPrecision::NativeSegmentClipped);
        assert!(segment_range_with_tolerance(27_620, 30_021, input, 20).is_err());
        assert!(segment_range_with_tolerance(30_001, 30_020, input, 20).is_err());
        assert!(segment_range_with_tolerance(-1, 30_020, input, 20).is_err());
        assert!(segment_range_with_tolerance(29_000, 28_000, input, 20).is_err());
        assert!(segment_range(27_620, 30_020, input).is_err());
        assert_eq!(
            segment_range_with_tolerance(27_620, 30_000, input, 20)
                .unwrap()
                .1,
            TimingPrecision::NativeSegment
        );
    }

    #[test]
    fn multitalker_terminal_frame_is_clipped_only_on_its_frame_grid() {
        let request = RecognitionRequest {
            scope: SpeakerScope {
                generation: "frame-test".into(),
                epoch: 1,
            },
            start_sample: 480_000,
            revision: 0,
            attribution: Attribution::Native,
        };
        let transcript = |start, end| Transcript {
            text: "Test".into(),
            segments: vec![transcribe_cpp::Segment {
                t0_ms: start,
                t1_ms: end,
                text: "Test".into(),
                speaker_id: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        let output =
            normalize_multitalker_transcript(transcript(640, 30_080), 480_000, &request).unwrap();
        assert_eq!(output.segments[0].range.end, 960_000);
        assert_eq!(
            output.segments[0].timing,
            TimingPrecision::NativeSegmentClipped
        );
        assert!(
            normalize_multitalker_transcript(transcript(640, 30_081), 480_000, &request).is_err()
        );
        assert!(
            normalize_multitalker_transcript(transcript(640, 30_079), 480_000, &request).is_err()
        );
        assert!(
            normalize_multitalker_transcript(transcript(640, 30_160), 480_000, &request).is_err()
        );
        assert!(
            normalize_multitalker_transcript(transcript(30_000, 30_080), 480_000, &request)
                .is_err()
        );
        assert!(normalize_transcript(transcript(640, 30_080), 480_000, &request).is_err());
    }
}
