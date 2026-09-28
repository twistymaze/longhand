//! Route-specific recognition on the canonical 16 kHz recording timeline.
//! Processed-input coverage and timed-text coverage are different evidence.
pub mod durable;
mod nemo;
mod stages;
pub use stages::{execute, prepare_retry, saved_result, snapshot};

use crate::StageError;
use asr::{Finality, RecognitionSegment, SampleRange, SpeakerTurn, TimingPrecision};
use models::recognition::{ExecutionSpec, SpeakerSource};
use serde::{Deserialize, Serialize};
use session_core::Stage;

pub const RESULT_KEY: &str = "recognition-result-v1";
pub const SPEAKERS_KEY: &str = "recognition-speakers-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Route {
    Off,
    Integrated,
    SeparatePyannote,
    SeparateNemo,
    CombinedNemo,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGraph {
    pub route: Route,
    pub separate_analysis: bool,
    pub stages: Vec<Stage>,
}
impl TaskGraph {
    pub fn resolve(spec: &ExecutionSpec) -> Result<Self, StageError> {
        if spec.schema_version != 1 {
            return Err(failed("unsupported recognition snapshot version"));
        }
        for runtime in &spec.runtimes {
            let expected = match runtime.name.as_str() {
                "transcribe.cpp" => models::recognition::TRANSCRIBE_REVISION,
                "NeMo-Speech.cpp" => models::recognition::NEMO_REVISION,
                "onnx-runtime" => "1.22.0",
                _ => return Err(failed("unsupported recognition runtime")),
            };
            if runtime.revision != expected {
                return Err(failed("the note requires a different runtime revision"));
            }
        }
        let native_nemo = spec
            .artifacts
            .first()
            .is_some_and(|a| a.format == "nemo-gguf");
        let route = match (spec.config.speaker_source, native_nemo) {
            (SpeakerSource::Off, false) => Route::Off,
            (SpeakerSource::Integrated, false) => Route::Integrated,
            (SpeakerSource::Pyannote, false) => Route::SeparatePyannote,
            (SpeakerSource::Sortformer | SpeakerSource::Nemotron, false) => Route::SeparateNemo,
            (SpeakerSource::Sortformer | SpeakerSource::Nemotron, true) => Route::CombinedNemo,
            _ => {
                return Err(failed(
                    "this NeMo ASR artifact requires a compatible separate speaker model",
                ));
            }
        };
        let separate_analysis = matches!(route, Route::SeparatePyannote | Route::SeparateNemo);
        let mut stages = vec![Stage::Assemble];
        if separate_analysis {
            stages.push(Stage::Diarize);
        }
        stages.extend([
            Stage::Transcribe,
            Stage::Join,
            Stage::Verify,
            Stage::Summarize,
            Stage::Save,
        ]);
        Ok(Self {
            route,
            separate_analysis,
            stages,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteResult {
    pub route: Route,
    /// Completed processing intervals, including silence. Not word coverage.
    pub coverage: Vec<SampleRange>,
    pub rows: Vec<RecognitionSegment>,
    pub speakers: Vec<SpeakerTurn>,
    /// Explicit uncertainty retained with the neutral result.
    pub coverage_limitations: Vec<String>,
}

pub(crate) fn failed(message: impl Into<String>) -> StageError {
    StageError::Failed(message.into())
}

/// Assign only a unique greatest overlap. Do not assign the nearest speaker
/// across a silent gap, break an overlap tie, or treat a source window as words.
pub fn join_separate(rows: &mut [RecognitionSegment], turns: &[SpeakerTurn]) {
    for row in rows {
        row.speaker = None;
        row.speaker_finality = Finality::Final;
        if row.timing == TimingPrecision::SourceWindow {
            continue;
        }
        let mut overlaps: Vec<(&asr::SpeakerKey, u64)> = Vec::new();
        for turn in turns.iter().filter(|t| t.finality == Finality::Final) {
            let overlap = row
                .range
                .end
                .min(turn.range.end)
                .saturating_sub(row.range.start.max(turn.range.start));
            if overlap == 0 {
                continue;
            }
            if let Some((_, total)) = overlaps.iter_mut().find(|(key, _)| **key == turn.speaker) {
                *total += overlap;
            } else {
                overlaps.push((&turn.speaker, overlap));
            }
        }
        overlaps.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        if let Some((speaker, greatest)) = overlaps.first()
            && overlaps.get(1).is_none_or(|(_, next)| next < greatest)
        {
            row.speaker = Some((*speaker).clone());
        }
    }
}

pub fn validate_result(result: &RouteResult, expected: &[SampleRange]) -> Result<(), StageError> {
    if result.coverage.iter().any(|r| r.end < r.start) {
        return Err(failed("invalid processed-input interval"));
    }
    if expected
        .iter()
        .any(|required| !covers(&result.coverage, *required))
    {
        return Err(failed("recognition did not process all required audio"));
    }
    for row in &result.rows {
        if (row.range.end < row.range.start
            || (row.range.end == row.range.start && row.timing != TimingPrecision::NativeTimePoint))
            || !result
                .coverage
                .iter()
                .any(|c| c.start <= row.range.start && c.end >= row.range.end)
        {
            return Err(failed(
                "recognition text is outside completed input coverage",
            ));
        }
        if row.text_finality != Finality::Final || row.speaker_finality != Finality::Final {
            return Err(failed("recognition has provisional text or speaker labels"));
        }
        if result.route == Route::Off && row.speaker.is_some() {
            return Err(failed("speaker recognition is off but labels are present"));
        }
    }
    if result.route != Route::Off
        && result.rows.iter().any(|r| !r.text.trim().is_empty())
        && !result.rows.iter().any(|r| r.speaker.is_some())
    {
        return Err(failed(
            "requested speaker recognition returned no usable labels",
        ));
    }
    if result
        .speakers
        .iter()
        .any(|t| t.finality != Finality::Final)
    {
        return Err(failed("speaker analysis is not final"));
    }
    Ok(())
}

/// Test the union of completed intervals, including multiple native epochs.
pub fn covers(ranges: &[SampleRange], required: SampleRange) -> bool {
    let mut ranges = ranges.to_vec();
    ranges.sort_by_key(|r| r.start);
    let mut at = required.start;
    for range in ranges {
        if range.start > at {
            break;
        }
        at = at.max(range.end);
    }
    at >= required.end
}

/// Render readable turns without discarding the neutral rows' timing precision.
pub fn transcript(result: &RouteResult) -> session_core::folder::Transcript {
    use session_core::folder::{Transcript, TranscriptLine};
    let mut keys: Vec<asr::SpeakerKey> = Vec::new();
    let mut lines = Vec::new();
    let mut open: Option<(Option<u32>, u64, u64, String)> = None;
    let mut rows: Vec<_> = result.rows.iter().collect();
    rows.sort_by_key(|r| r.range.start);
    for row in rows {
        let text = row.text.trim();
        if text.is_empty() {
            continue;
        }
        let speaker = row.speaker.as_ref().map(|key| {
            let index = keys
                .iter()
                .position(|known| known == key)
                .unwrap_or_else(|| {
                    keys.push(key.clone());
                    keys.len() - 1
                });
            (index + 1) as u32
        });
        let start = row.range.start / 16;
        let end = row.range.end / 16;
        match &mut open {
            Some((current, _, last_end, paragraph))
                if *current == speaker
                    && start.saturating_sub(*last_end) <= crate::join::PARAGRAPH_PAUSE_MS
                    && !crate::join::paragraph_full(paragraph) =>
            {
                if !paragraph.ends_with(asr::unspaced)
                    && !text.starts_with(asr::unspaced)
                    && !text.starts_with([',', '.', ':', ';', '!', '?'])
                {
                    paragraph.push(' ');
                }
                paragraph.push_str(text);
                *last_end = (*last_end).max(end);
            }
            _ => {
                if let Some((speaker, position_ms, _, text)) = open.take() {
                    lines.push(TranscriptLine::Turn {
                        position_ms,
                        speaker,
                        text,
                    });
                }
                open = Some((speaker, start, end, text.into()));
            }
        }
    }
    if let Some((speaker, position_ms, _, text)) = open {
        lines.push(TranscriptLine::Turn {
            position_ms,
            speaker,
            text,
        });
    }
    Transcript {
        has_speakers: result.route != Route::Off,
        lines,
    }
}

/// Native labels identify slots inside one namespace. Multiple independent
/// namespaces cannot establish the number of distinct people in a recording.
pub fn found_speakers(result: &RouteResult) -> Option<u32> {
    if result.route == Route::Off {
        return None;
    }
    let mut keys: Vec<&asr::SpeakerKey> = Vec::new();
    for key in result.rows.iter().filter_map(|r| r.speaker.as_ref()) {
        if keys.first().is_some_and(|first| {
            first.scope != key.scope || first.input_start_sample != key.input_start_sample
        }) {
            return None;
        }
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if keys.is_empty() {
        None
    } else {
        Some(keys.len() as u32)
    }
}

pub const SCOPED_LABELS_NOTE: &str = "Speaker labels restart at independent analysis windows or stream epochs. Different labels can refer to the same person. The number of distinct people is unknown.";
