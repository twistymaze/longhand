use super::{Route, RouteResult, failed, join_separate};
use crate::{Control, StageError, runner::EngineProvider};
use asr::{
    Finality, RecognitionId, RecognitionSegment, SampleRange, SpeakerKey, SpeakerScope,
    SpeakerTurn, TimingPrecision,
};
use models::recognition::ExecutionSpec;
use nemo_runtime::Helper;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

/// A defined native epoch. Checkpoints inside it do not reset native state.
pub const MAX_EPOCH_SAMPLES: u64 = 20 * 60 * 16000;

pub(super) fn run_file(
    spec: &ExecutionSpec,
    provider: &dyn EngineProvider,
    path: &Path,
    range: SampleRange,
    generation: &str,
    combined: bool,
    control: &dyn Control,
) -> Result<RouteResult, StageError> {
    let helper = provider.nemo_helper().map_err(StageError::Waiting)?;
    let speaker_id = match spec.config.speaker_source {
        models::recognition::SpeakerSource::Sortformer => "sortformer-v2-q80",
        models::recognition::SpeakerSource::Nemotron => "nemotron-3-diarization-q80",
        _ => return Err(failed("NeMo speaker artifact is not selected")),
    };
    let speaker = spec
        .artifacts
        .iter()
        .find(|a| a.id == speaker_id)
        .ok_or_else(|| failed("speaker artifact is absent from snapshot"))?;
    let speaker = provider
        .artifact_path(speaker)
        .map_err(StageError::Waiting)?;
    let asr = if combined {
        Some(
            provider
                .artifact_path(
                    spec.artifacts
                        .first()
                        .ok_or_else(|| failed("ASR artifact is absent"))?,
                )
                .map_err(StageError::Waiting)?,
        )
    } else {
        None
    };
    let native_route = if !combined {
        "separate"
    } else if spec.config.asr_model == "parakeet-tdt-0.6b-v3-q80" {
        "combined_offline"
    } else {
        "combined_stream"
    };
    let mut result = RouteResult {
        route: if combined {
            Route::CombinedNemo
        } else {
            Route::SeparateNemo
        },
        coverage: vec![],
        rows: vec![],
        speakers: vec![],
        coverage_limitations: vec![],
    };
    let mut start = range.start;
    while start < range.end {
        control.proceed()?;
        let end = range.end.min(start + MAX_EPOCH_SAMPLES);
        let scope = SpeakerScope {
            generation: generation.into(),
            epoch: start,
        };
        let input = EpochInput {
            helper: &helper,
            speaker: &speaker,
            asr: asr.as_deref(),
            path,
            native_route,
            language: spec.config.language.as_deref(),
            file_start: range.start,
            range: SampleRange { start, end },
            scope,
        };
        let requested_gpu = provider.nemo_gpu();
        let epoch = match run_epoch(&input, requested_gpu, control) {
            Ok(epoch) => epoch,
            Err(StageError::Failed(message)) if requested_gpu >= 0 => {
                // No output from a failed epoch is committed. Replay its complete
                // input on CPU and record the fallback in the result.
                control.proceed()?;
                let reason = format!(
                    "Accelerated NeMo attempt failed; epoch {start} replayed on CPU: {message}"
                );
                log::warn!("{reason}");
                result.coverage_limitations.push(reason);
                run_epoch(&input, -1, control)?
            }
            Err(error) => return Err(error),
        };
        result.coverage.extend(epoch.coverage);
        result.rows.extend(epoch.rows);
        result.speakers.extend(epoch.speakers);
        start = end;
    }
    if native_route == "combined_offline" {
        join_separate(&mut result.rows, &result.speakers);
    }
    Ok(result)
}

struct EpochInput<'a> {
    helper: &'a Path,
    speaker: &'a Path,
    asr: Option<&'a Path>,
    path: &'a Path,
    native_route: &'a str,
    language: Option<&'a str>,
    file_start: u64,
    range: SampleRange,
    scope: SpeakerScope,
}
struct Output {
    scope: SpeakerScope,
    start: u64,
    words: BTreeMap<(u64, u64), RecognitionSegment>,
    texts: BTreeMap<u64, String>,
    turns: Vec<SpeakerTurn>,
    pending: Option<(u64, Vec<SpeakerTurn>)>,
    coverage: Option<SampleRange>,
}
impl Output {
    fn process(&mut self, value: &Value) -> Result<(), StageError> {
        let event = value["event"].as_str().unwrap_or("");
        match event {
            "error" => {
                return Err(failed(
                    value["message"]
                        .as_str()
                        .unwrap_or("NeMo recognition failed"),
                ));
            }
            "segments_begin" => {
                if self.pending.is_some() {
                    return Err(failed("overlapping speaker snapshots"));
                }
                self.pending = Some((number(value, "revision")?, vec![]));
            }
            "segments" => {
                let revision = number(value, "revision")?;
                let (pending, turns) = self
                    .pending
                    .as_mut()
                    .ok_or_else(|| failed("speaker snapshot has no beginning"))?;
                if *pending != revision {
                    return Err(failed("speaker snapshot revision mismatch"));
                }
                for item in array(value, "segments")? {
                    let local_id = number(item, "speaker")? as u32;
                    if local_id == 0 {
                        continue;
                    }
                    turns.push(SpeakerTurn {
                        range: bounds(item)?,
                        timing: TimingPrecision::NativeSegment,
                        speaker: SpeakerKey {
                            scope: self.scope.clone(),
                            input_start_sample: self.start,
                            local_id,
                        },
                        finality: finality(item["speaker_final"].as_bool().unwrap_or(false)),
                    });
                }
            }
            "segments_end" => {
                let (revision, turns) = self
                    .pending
                    .take()
                    .ok_or_else(|| failed("speaker snapshot has no beginning"))?;
                if revision != number(value, "revision")? {
                    return Err(failed("speaker snapshot revision mismatch"));
                }
                self.turns = turns;
            }
            "text" => {
                self.texts.insert(
                    number(value, "result_id")?,
                    value["text"].as_str().unwrap_or("").into(),
                );
            }
            "words" => {
                let result_id = number(value, "result_id")?;
                let revision = number(value, "revision")?;
                for word in array(value, "words")? {
                    let index = number(word, "index")?;
                    let speaker = number(word, "speaker")? as u32;
                    let text = word["text"]
                        .as_str()
                        .ok_or_else(|| failed("word text is absent"))?
                        .to_string();
                    let range = bounds(word)?;
                    let row = RecognitionSegment {
                        id: RecognitionId {
                            scope: self.scope.clone(),
                            input_start_sample: self.start,
                            ordinal: (result_id << 32) | index,
                        },
                        range,
                        timing: if range.start == range.end {
                            TimingPrecision::NativeTimePoint
                        } else {
                            TimingPrecision::NativeSegment
                        },
                        text,
                        speaker: (speaker > 0).then(|| SpeakerKey {
                            scope: self.scope.clone(),
                            input_start_sample: self.start,
                            local_id: speaker,
                        }),
                        revision,
                        text_finality: finality(value["text_final"].as_bool().unwrap_or(false)),
                        speaker_finality: finality(
                            word["speaker_final"].as_bool().unwrap_or(false),
                        ),
                    };
                    let key = (result_id, index);
                    if self
                        .words
                        .get(&key)
                        .is_none_or(|old| old.revision < revision)
                    {
                        self.words.insert(key, row);
                    }
                }
            }
            "punctuation" => {
                let result_id = number(value, "result_id")?;
                let punctuation = value["text"].as_str().unwrap_or("");
                if let Some((_, word)) = self
                    .words
                    .range_mut((result_id, 0)..=(result_id, u64::MAX))
                    .next_back()
                {
                    word.text.push_str(punctuation);
                    word.revision = number(value, "revision")?;
                }
                if let Some(text) = self.texts.get_mut(&result_id) {
                    text.push_str(punctuation);
                }
            }
            "finished" => {
                if self.pending.is_some()
                    || value["text_final"] != true
                    || value["speaker_final"] != true
                {
                    return Err(failed("NeMo finished with incomplete output"));
                }
                self.coverage = Some(SampleRange {
                    start: number(value, "coverage_start")?,
                    end: number(value, "coverage_end")?,
                });
            }
            "hello" | "ready" | "ack" | "interim" => {}
            _ => return Err(failed("unknown NeMo output event")),
        }
        Ok(())
    }
}
fn finality(value: bool) -> Finality {
    if value {
        Finality::Final
    } else {
        Finality::Provisional
    }
}
fn number(value: &Value, field: &str) -> Result<u64, StageError> {
    value[field]
        .as_u64()
        .ok_or_else(|| failed(format!("invalid NeMo field: {field}")))
}
fn array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, StageError> {
    value[field]
        .as_array()
        .ok_or_else(|| failed(format!("invalid NeMo array: {field}")))
}
fn bounds(value: &Value) -> Result<SampleRange, StageError> {
    let range = SampleRange {
        start: number(value, "start")?,
        end: number(value, "end")?,
    };
    if range.end < range.start {
        return Err(failed("invalid NeMo time interval"));
    }
    Ok(range)
}
fn until(
    helper: &mut Helper,
    output: &mut Output,
    event: &str,
    control: &dyn Control,
) -> Result<Value, StageError> {
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        // Always drain output while inference completes. A capture suspension
        // takes effect before the next push, not in the pipe-drain loop.
        if control.is_cancelled() {
            return Err(StageError::Cancelled);
        }
        match helper.receive(Duration::from_millis(100)) {
            Ok(frame) => {
                output.process(&frame.metadata)?;
                if frame.metadata["event"] == event {
                    return Ok(frame.metadata);
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::TimedOut && Instant::now() < deadline => {}
            Err(error) => return Err(failed(format!("NeMo helper: {error}"))),
        }
    }
}
fn run_epoch(
    input: &EpochInput<'_>,
    gpu: i32,
    control: &dyn Control,
) -> Result<RouteResult, StageError> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    input.scope.generation.hash(&mut hash);
    let mut helper = Helper::spawn(input.helper, hash.finish(), input.scope.epoch)
        .map_err(|e| failed(e.to_string()))?;
    let mut output = Output {
        scope: input.scope.clone(),
        start: input.range.start,
        words: BTreeMap::new(),
        texts: BTreeMap::new(),
        turns: vec![],
        pending: None,
        coverage: None,
    };
    helper.send(json!({"command":"start", "route":input.native_route, "speaker_model":input.speaker, "asr_model":input.asr, "gpu":gpu, "language":input.language.unwrap_or(""), "epoch_start":input.range.start, "max_epoch_samples":MAX_EPOCH_SAMPLES}),vec![]).map_err(|e| failed(e.to_string()))?;
    until(&mut helper, &mut output, "ready", control)?;
    let mut reader = hound::WavReader::open(input.path)?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != 16000 {
        return Err(failed("NeMo input must be canonical mono 16 kHz"));
    }
    reader.seek(
        u32::try_from(input.range.start - input.file_start)
            .map_err(|_| failed("audio seek exceeds WAV capacity"))?,
    )?;
    let mut at = input.range.start;
    while at < input.range.end {
        control.proceed()?;
        let count = (input.range.end - at).min(nemo_runtime::MAX_PCM_SAMPLES as u64) as usize;
        let pcm: Vec<f32> = if spec.sample_format == hound::SampleFormat::Float {
            reader
                .samples::<f32>()
                .take(count)
                .collect::<Result<_, _>>()?
        } else {
            reader
                .samples::<i16>()
                .take(count)
                .map(|s| s.map(|s| s as f32 / 32768.0))
                .collect::<Result<_, _>>()?
        };
        if pcm.len() != count {
            return Err(failed("audio ended before requested coverage"));
        }
        helper
            .send(json!({"command":"push", "offset":at}), pcm)
            .map_err(|e| failed(e.to_string()))?;
        let ack = until(&mut helper, &mut output, "ack", control)?;
        at += count as u64;
        if number(&ack, "accepted_end")? != at {
            return Err(failed("NeMo input acknowledgement mismatch"));
        }
    }
    helper
        .send(json!({"command":"finish"}), vec![])
        .map_err(|e| failed(e.to_string()))?;
    until(&mut helper, &mut output, "finished", control)?;
    if output.coverage != Some(input.range) {
        return Err(failed("NeMo terminal coverage is incomplete"));
    }
    for (id, text) in &output.texts {
        if !text.trim().is_empty() && !output.words.keys().any(|(result, _)| result == id) {
            return Err(failed(
                "NeMo returned text without corresponding timed words",
            ));
        }
    }
    Ok(RouteResult {
        route: if input.native_route == "separate" {
            Route::SeparateNemo
        } else {
            Route::CombinedNemo
        },
        coverage: vec![input.range],
        rows: output.words.into_values().collect(),
        speakers: output.turns,
        coverage_limitations: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output() -> Output {
        Output {
            scope: SpeakerScope {
                generation: "test".into(),
                epoch: 1,
            },
            start: 32000,
            words: BTreeMap::new(),
            texts: BTreeMap::new(),
            turns: vec![],
            pending: None,
            coverage: None,
        }
    }
    #[test]
    fn speaker_revision_does_not_make_provisional_labels_final() {
        let mut out = output();
        let patch = |revision, speaker, stable| json!({"event":"words","result_id":1,"revision":revision,"text_final":true,"words":[{"index":0,"text":"Hello","start":32000,"end":48000,"speaker":speaker,"speaker_final":stable}]});
        out.process(&patch(1, 1, false)).unwrap();
        assert_eq!(out.words[&(1, 0)].speaker_finality, Finality::Provisional);
        out.process(&patch(2, 2, true)).unwrap();
        out.process(&patch(1, 1, false)).unwrap();
        assert_eq!(out.words.len(), 1);
        assert_eq!(out.words[&(1, 0)].speaker.as_ref().unwrap().local_id, 2);
        assert_eq!(out.words[&(1, 0)].speaker_finality, Finality::Final);
        out.process(&json!({"event":"punctuation","result_id":1,"revision":3,"text":"."}))
            .unwrap();
        assert_eq!(out.words[&(1, 0)].text, "Hello.");
    }
    #[test]
    fn partial_speaker_snapshot_cannot_replace_completed_output() {
        let mut out = output();
        out.process(&json!({"event":"segments_begin","revision":1}))
            .unwrap();
        out.process(&json!({"event":"segments","revision":1,"segments":[{"start":32000,"end":48000,"speaker":1,"speaker_final":true}]})).unwrap();
        assert!(out.turns.is_empty());
        assert!(out.process(&json!({"event":"finished","coverage_start":32000,"coverage_end":48000,"text_final":true,"speaker_final":true})).is_err());
        out.process(&json!({"event":"segments_end","revision":1}))
            .unwrap();
        assert_eq!(out.turns.len(), 1);
        out.process(&json!({"event":"segments_begin","revision":2}))
            .unwrap();
        assert_eq!(out.turns.len(), 1);
        out.process(&json!({"event":"segments_end","revision":2}))
            .unwrap();
        assert!(out.turns.is_empty());
    }
    #[test]
    fn terminal_time_point_retains_native_speaker_and_unknown_duration() {
        let mut out = output();
        out.process(&json!({"event":"words","result_id":1,"revision":1,"text_final":true,"words":[{"index":0,"text":"tail","start":48000,"end":48000,"speaker":1,"speaker_final":true}]})).unwrap();
        assert_eq!(out.words[&(1, 0)].timing, TimingPrecision::NativeTimePoint);
        assert_eq!(
            out.words[&(1, 0)].range,
            SampleRange {
                start: 48000,
                end: 48000
            }
        );
        assert!(out.words[&(1, 0)].speaker.is_some());
    }
}
