use super::{Route, RouteResult, failed, join_separate};
use crate::live::{Step, next_step};
use crate::progress::SpeakerMeter;
use crate::{Control, StageError, runner::EngineProvider};
use asr::{
    Finality, RecognitionId, RecognitionSegment, SampleRange, SpeakerKey, SpeakerScope,
    SpeakerTurn, TimingPrecision,
};
use models::recognition::ExecutionSpec;
use nemo_runtime::Helper;
use serde_json::{Value, json};
use session_core::recognition::{LIVE_NEMO_PREFIX, NATIVE_FINAL_PREFIX};
use std::{
    path::Path,
    time::{Duration, Instant},
};

/// A defined native epoch. Checkpoints inside it do not reset native state.
pub const MAX_EPOCH_SAMPLES: u64 = 20 * 60 * 16000;

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredEpoch {
    scope: SpeakerScope,
    range: SampleRange,
    rows: usize,
    speakers: usize,
    limitations: Vec<String>,
    #[serde(default)]
    output: Option<super::durable::EpochOutput>,
}
fn epoch_prefix(scope: &SpeakerScope) -> String {
    format!("{NATIVE_FINAL_PREFIX}{:020}/", scope.epoch)
}
fn epoch_key(start: u64) -> String {
    format!("recognition-epoch-{start:020}")
}
/// The completed epoch `start..end` of this generation, if one is saved.
fn saved_epoch(
    store: &crate::SharedStore,
    session: &str,
    stage: session_core::Stage,
    generation: &str,
    start: u64,
    end: u64,
) -> Result<Option<StoredEpoch>, StageError> {
    let saved: Option<StoredEpoch> =
        crate::checkpoint::get(store, session, stage, &epoch_key(start))?;
    Ok(saved.filter(|saved| {
        saved.scope.generation == generation && saved.range == (SampleRange { start, end })
    }))
}
/// The samples of `range` in saved separate-speaker epochs. [`run_file`]
/// restores these epochs with no new input.
pub(super) fn saved_samples(
    store: &crate::SharedStore,
    session: &str,
    generation: &str,
    range: SampleRange,
) -> Result<u64, StageError> {
    let mut samples = 0;
    let mut start = range.start;
    while start < range.end {
        let end = range.end.min(start + MAX_EPOCH_SAMPLES);
        let stage = session_core::Stage::Diarize;
        if saved_epoch(store, session, stage, generation, start, end)?.is_some() {
            samples += end - start;
        }
        start = end;
    }
    Ok(samples)
}
fn persist_epoch(
    store: &crate::SharedStore,
    id: &str,
    scope: &SpeakerScope,
    result: &RouteResult,
    native_route: &str,
) -> Result<StoredEpoch, StageError> {
    use session_core::recognition::{Coverage, OutputBlock};
    let output = match native_route {
        "asr_only" => super::durable::EpochOutput {
            text: true,
            speakers: false,
        },
        "separate" => super::durable::EpochOutput {
            text: false,
            speakers: true,
        },
        "combined_offline" | "combined_stream" => super::durable::EpochOutput {
            text: true,
            speakers: true,
        },
        _ => return Err(failed("unknown native epoch output modality")),
    };
    let prefix = epoch_prefix(scope);
    let rows = result.rows.iter().enumerate().map(|(i, row)| OutputBlock {
        key: format!("{prefix}row/{i:020}"),
        scope: scope.clone(),
        revision: row.revision,
        coverage: Coverage {
            received: Some(row.range),
            computed: Some(row.range),
            provisional: None,
            committed: Some(row.range),
        },
        rows: vec![row.clone()],
        speakers: vec![],
    });
    let speakers = result
        .speakers
        .iter()
        .enumerate()
        .map(|(i, turn)| OutputBlock {
            key: format!("{prefix}speaker/{i:020}"),
            scope: scope.clone(),
            revision: 0,
            coverage: Coverage {
                received: Some(turn.range),
                computed: Some(turn.range),
                provisional: None,
                committed: Some(turn.range),
            },
            rows: vec![],
            speakers: vec![turn.clone()],
        });
    super::durable::commit_blocks(store, id, rows.chain(speakers))?;
    Ok(StoredEpoch {
        scope: scope.clone(),
        range: result.coverage[0],
        rows: result.rows.len(),
        speakers: result.speakers.len(),
        limitations: result.coverage_limitations.clone(),
        output: Some(output),
    })
}
fn restore_epoch(
    store: &crate::SharedStore,
    id: &str,
    record: &StoredEpoch,
    route: Route,
) -> Result<RouteResult, StageError> {
    let mut result = RouteResult {
        route,
        coverage: vec![record.range],
        rows: vec![],
        speakers: vec![],
        coverage_limitations: record.limitations.clone(),
    };
    let prefix = epoch_prefix(&record.scope);
    let mut after = String::new();
    loop {
        let page =
            crate::lock(store).recognition_page(id, &record.scope.generation, &prefix, &after)?;
        if page.is_empty() {
            break;
        }
        for block in page {
            after = block.key;
            result.rows.extend(block.rows);
            result.speakers.extend(block.speakers);
        }
    }
    if result.rows.len() != record.rows || result.speakers.len() != record.speakers {
        return Err(failed("durable epoch output is incomplete"));
    }
    Ok(result)
}

#[allow(
    clippy::too_many_arguments,
    reason = "The route binds storage, execution identity, audio, and cancellation for one file"
)]
pub(super) fn run_file(
    store: &crate::SharedStore,
    session: &str,
    spec: &ExecutionSpec,
    provider: &dyn EngineProvider,
    path: &Path,
    range: SampleRange,
    generation: &str,
    combined: bool,
    control: &dyn Control,
    mut speakers: Option<&mut SpeakerMeter>,
) -> Result<RouteResult, StageError> {
    let helper = provider.nemo_helper().map_err(StageError::Waiting)?;
    let text_only =
        combined && spec.enabled_live_transcription != spec.enabled_live_speaker_analysis;
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
    let speaker = if text_only {
        None
    } else {
        Some(
            provider
                .artifact_path(speaker)
                .map_err(StageError::Waiting)?,
        )
    };
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
    let native_route = if text_only {
        "asr_only"
    } else if !combined {
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
        let stage = if combined {
            session_core::Stage::Transcribe
        } else {
            session_core::Stage::Diarize
        };
        let key = epoch_key(start);
        if let Some(saved) = saved_epoch(store, session, stage, generation, start, end)? {
            // [`saved_samples`] counted this epoch before new input.
            let epoch = restore_epoch(store, session, &saved, result.route)?;
            result.coverage.extend(epoch.coverage);
            result.rows.extend(epoch.rows);
            result.speakers.extend(epoch.speakers);
            result
                .coverage_limitations
                .extend(epoch.coverage_limitations);
            start = end;
            continue;
        }
        let scope = SpeakerScope {
            generation: generation.into(),
            epoch: start,
        };
        let input = EpochInput {
            store,
            session,
            helper: &helper,
            speaker: speaker.as_deref(),
            asr: asr.as_deref(),
            path,
            native_route,
            streaming: spec.config.asr_model != "parakeet-tdt-0.6b-v3-q80",
            language: spec.config.language.as_deref(),
            file_start: range.start,
            range: SampleRange { start, end },
            scope,
        };
        let requested_gpu = provider.nemo_gpu();
        let (mut epoch, scope) =
            match run_epoch(&input, requested_gpu, control, speakers.as_deref_mut()) {
                Ok(epoch) => epoch,
                Err(StageError::Failed(message)) if requested_gpu >= 0 => {
                    // No output from a failed epoch is committed. Replay its complete
                    // input on CPU and record the fallback in the result. The
                    // meter holds its value until the replay passes it (KTD2).
                    control.proceed()?;
                    let reason = format!(
                        "Accelerated NeMo attempt failed; epoch {start} replayed on CPU: {message}"
                    );
                    log::warn!("{reason}");
                    let (mut output, scope) =
                        run_epoch(&input, -1, control, speakers.as_deref_mut())?;
                    output.coverage_limitations.push(reason);
                    (output, scope)
                }
                Err(error) => return Err(error),
            };
        if native_route == "combined_offline" {
            join_separate(&mut epoch.rows, &epoch.speakers);
        }
        let record = persist_epoch(store, session, &scope, &epoch, native_route)?;
        crate::checkpoint::put(store, session, stage, &key, &record)?;
        crate::lock(store).delete_recognition_prefix(
            session,
            &scope.generation,
            &format!("{LIVE_NEMO_PREFIX}{:020}/", scope.epoch),
        )?;
        if let Some(meter) = speakers.as_deref_mut() {
            meter.finished(end - start);
        }
        result.coverage.extend(epoch.coverage);
        result.rows.extend(epoch.rows);
        result.speakers.extend(epoch.speakers);
        result
            .coverage_limitations
            .extend(epoch.coverage_limitations);
        start = end;
    }
    if native_route == "combined_offline" {
        join_separate(&mut result.rows, &result.speakers);
    }
    Ok(result)
}

struct EpochInput<'a> {
    store: &'a crate::SharedStore,
    session: &'a str,
    helper: &'a Path,
    speaker: Option<&'a Path>,
    asr: Option<&'a Path>,
    path: &'a Path,
    native_route: &'a str,
    streaming: bool,
    language: Option<&'a str>,
    file_start: u64,
    range: SampleRange,
    scope: SpeakerScope,
}
struct Output {
    store: crate::SharedStore,
    session: String,
    scope: SpeakerScope,
    start: u64,
    pending: Option<(u64, u64)>,
    turns: Option<(u64, u64)>,
    coverage: Option<SampleRange>,
    interim_result: Option<u64>,
}
impl Output {
    fn prefix(&self, kind: &str) -> String {
        format!("{LIVE_NEMO_PREFIX}{:020}/{kind}/", self.scope.epoch)
    }
    fn block(
        &self,
        key: String,
        revision: u64,
        range: Option<SampleRange>,
        rows: Vec<RecognitionSegment>,
        speakers: Vec<SpeakerTurn>,
    ) -> session_core::recognition::OutputBlock {
        session_core::recognition::OutputBlock {
            key,
            scope: self.scope.clone(),
            revision,
            coverage: session_core::recognition::Coverage {
                received: range,
                computed: range,
                provisional: range,
                committed: None,
            },
            rows,
            speakers,
        }
    }
    fn put(&self, block: &session_core::recognition::OutputBlock) -> Result<(), StageError> {
        crate::lock(&self.store).commit_recognition(&self.session, block)?;
        Ok(())
    }
    fn visit(
        &self,
        prefix: &str,
        mut visitor: impl FnMut(session_core::recognition::OutputBlock) -> Result<(), StageError>,
    ) -> Result<(), StageError> {
        let mut after = String::new();
        loop {
            let page = crate::lock(&self.store).recognition_page(
                &self.session,
                &self.scope.generation,
                prefix,
                &after,
            )?;
            if page.is_empty() {
                break;
            }
            for block in page {
                after = block.key.clone();
                visitor(block)?;
            }
        }
        Ok(())
    }
    fn process(&mut self, value: &Value) -> Result<(), StageError> {
        match value["event"].as_str().unwrap_or("") {
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
                self.pending = Some((number(value, "revision")?, 0));
            }
            "segments" => {
                let revision = number(value, "revision")?;
                let (pending, mut count) = self
                    .pending
                    .ok_or_else(|| failed("speaker snapshot has no beginning"))?;
                if pending != revision {
                    return Err(failed("speaker snapshot revision mismatch"));
                }
                for item in array(value, "segments")? {
                    let local_id = u32::try_from(number(item, "speaker")?)
                        .map_err(|_| failed("speaker ID overflow"))?;
                    if local_id == 0 {
                        continue;
                    }
                    let range = bounds(item)?;
                    let turn = SpeakerTurn {
                        range,
                        timing: TimingPrecision::NativeSegment,
                        speaker: SpeakerKey {
                            scope: self.scope.clone(),
                            input_start_sample: self.start,
                            local_id,
                        },
                        finality: finality(item["speaker_final"].as_bool().unwrap_or(false)),
                    };
                    self.put(&self.block(
                        format!("{}{:020}/{count:020}", self.prefix("turn"), revision),
                        revision,
                        Some(range),
                        vec![],
                        vec![turn],
                    ))?;
                    count += 1;
                }
                self.pending = Some((revision, count));
            }
            "segments_end" => {
                let pending = self
                    .pending
                    .take()
                    .ok_or_else(|| failed("speaker snapshot has no beginning"))?;
                if pending.0 != number(value, "revision")? {
                    return Err(failed("speaker snapshot revision mismatch"));
                }
                if self.turns.is_none_or(|old| old.0 < pending.0) {
                    // The marker selects only a complete snapshot. Commit it before
                    // deleting old rows so a crash cannot expose a partial revision.
                    self.put(&self.block(
                        format!("{LIVE_NEMO_PREFIX}{:020}/turn-commit", self.scope.epoch),
                        pending.0,
                        None,
                        vec![],
                        vec![],
                    ))?;
                    let old = self.turns.replace(pending);
                    if let Some((revision, _)) = old {
                        crate::lock(&self.store).delete_recognition_prefix(
                            &self.session,
                            &self.scope.generation,
                            &format!("{}{:020}/", self.prefix("turn"), revision),
                        )?;
                    }
                } else if self.turns.is_some_and(|old| old.0 != pending.0) {
                    crate::lock(&self.store).delete_recognition_prefix(
                        &self.session,
                        &self.scope.generation,
                        &format!("{}{:020}/", self.prefix("turn"), pending.0),
                    )?;
                }
            }
            "text" => {
                if !value["text"].as_str().unwrap_or("").trim().is_empty() {
                    let id = number(value, "result_id")?;
                    self.put(&self.block(
                        format!("{}{id:020}", self.prefix("text")),
                        number(value, "revision")?,
                        None,
                        vec![],
                        vec![],
                    ))?;
                }
            }
            "words" => {
                let result = number(value, "result_id")?;
                let revision = number(value, "revision")?;
                if self.interim_result == Some(result) {
                    crate::lock(&self.store).delete_recognition_prefix(
                        &self.session,
                        &self.scope.generation,
                        &self.prefix("interim"),
                    )?;
                    self.interim_result = None;
                }
                if result > u32::MAX as u64 {
                    return Err(failed("result ID overflow"));
                }
                for word in array(value, "words")? {
                    let index = number(word, "index")?;
                    if index > u32::MAX as u64 {
                        return Err(failed("word ID overflow"));
                    }
                    let speaker = u32::try_from(number(word, "speaker")?)
                        .map_err(|_| failed("speaker ID overflow"))?;
                    let range = bounds(word)?;
                    let row = RecognitionSegment {
                        id: RecognitionId {
                            scope: self.scope.clone(),
                            input_start_sample: self.start,
                            ordinal: (result << 32) | index,
                        },
                        range,
                        timing: if range.start == range.end {
                            TimingPrecision::NativeTimePoint
                        } else {
                            TimingPrecision::NativeSegment
                        },
                        text: word["text"]
                            .as_str()
                            .ok_or_else(|| failed("word text is absent"))?
                            .into(),
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
                    self.put(&self.block(
                        format!("{}{result:020}/{index:020}", self.prefix("word")),
                        revision,
                        Some(range),
                        vec![row],
                        vec![],
                    ))?;
                }
            }
            "punctuation" => {
                let result = number(value, "result_id")?;
                let revision = number(value, "revision")?;
                let prefix = format!("{}{result:020}/", self.prefix("word"));
                let block = crate::lock(&self.store).recognition_last(
                    &self.session,
                    &self.scope.generation,
                    &prefix,
                )?;
                if let Some(mut block) = block
                    && revision > block.revision
                {
                    for row in &mut block.rows {
                        row.text.push_str(value["text"].as_str().unwrap_or(""));
                        row.revision = revision;
                    }
                    block.revision = revision;
                    self.put(&block)?;
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
            "ack" => {
                let end = number(value, "accepted_end")?;
                let mut progress = self.block(
                    format!("{}progress", self.prefix("input")),
                    end,
                    None,
                    vec![],
                    vec![],
                );
                progress.coverage.received = Some(SampleRange {
                    start: self.start,
                    end,
                });
                self.put(&progress)?;
            }
            "interim" => {
                let result = number(value, "result_id")?;
                let revision = number(value, "revision")?;
                let words = array(value, "words")?;
                let mut rows = Vec::new();
                for (index, word) in words.iter().enumerate() {
                    let range = bounds(word)?;
                    let speaker = word["speaker"].as_u64().unwrap_or(0) as u32;
                    rows.push(RecognitionSegment {
                        id: RecognitionId {
                            scope: self.scope.clone(),
                            input_start_sample: self.start,
                            ordinal: (result << 32) | index as u64,
                        },
                        range,
                        timing: if range.start == range.end {
                            TimingPrecision::NativeTimePoint
                        } else {
                            TimingPrecision::NativeSegment
                        },
                        text: word["text"].as_str().unwrap_or("").into(),
                        speaker: (speaker > 0).then(|| SpeakerKey {
                            scope: self.scope.clone(),
                            input_start_sample: self.start,
                            local_id: speaker,
                        }),
                        revision,
                        text_finality: Finality::Provisional,
                        speaker_finality: Finality::Provisional,
                    });
                }
                crate::lock(&self.store).delete_recognition_prefix(
                    &self.session,
                    &self.scope.generation,
                    &self.prefix("interim"),
                )?;
                for (index, rows) in rows.chunks(256).enumerate() {
                    let range = SampleRange {
                        start: rows.iter().map(|r| r.range.start).min().unwrap(),
                        end: rows.iter().map(|r| r.range.end).max().unwrap(),
                    };
                    self.put(&self.block(
                        format!("{}{index:020}", self.prefix("interim")),
                        revision,
                        Some(range),
                        rows.to_vec(),
                        vec![],
                    ))?;
                }
                self.interim_result = Some(result);
            }
            "hello" | "ready" => {}
            _ => return Err(failed("unknown NeMo output event")),
        }
        Ok(())
    }
    fn result(&self, route: Route) -> Result<RouteResult, StageError> {
        let mut result = RouteResult {
            route,
            coverage: vec![
                self.coverage
                    .ok_or_else(|| failed("native output is incomplete"))?,
            ],
            rows: vec![],
            speakers: vec![],
            coverage_limitations: vec![],
        };
        self.visit(&self.prefix("text"), |b| {
            let id = b
                .key
                .rsplit('/')
                .next()
                .ok_or_else(|| failed("invalid text key"))?;
            if crate::lock(&self.store)
                .recognition_last(
                    &self.session,
                    &self.scope.generation,
                    &format!("{}{id}/", self.prefix("word")),
                )?
                .is_none()
            {
                return Err(failed(
                    "NeMo returned text without corresponding timed words",
                ));
            }
            Ok(())
        })?;
        self.visit(&self.prefix("word"), |b| {
            result.rows.extend(b.rows);
            Ok(())
        })?;
        if let Some((revision, _)) = self.turns {
            self.visit(&format!("{}{:020}/", self.prefix("turn"), revision), |b| {
                result.speakers.extend(b.speakers);
                Ok(())
            })?;
        }
        Ok(result)
    }
    #[cfg(test)]
    fn word(&self, result: u64, index: u64) -> RecognitionSegment {
        crate::lock(&self.store)
            .recognition_block(
                &self.session,
                &self.scope.generation,
                &format!("{}{result:020}/{index:020}", self.prefix("word")),
            )
            .unwrap()
            .unwrap()
            .rows
            .remove(0)
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
/// `speakers` measures the input that the helper accepts. Helper startup
/// and finish give no fraction (KTD3).
fn run_epoch(
    input: &EpochInput<'_>,
    gpu: i32,
    control: &dyn Control,
    mut speakers: Option<&mut SpeakerMeter>,
) -> Result<(RouteResult, SpeakerScope), StageError> {
    use std::hash::{Hash, Hasher};
    if let Some(meter) = speakers.as_deref_mut() {
        meter.unmeasured();
    }
    let mut hash = std::hash::DefaultHasher::new();
    input.scope.generation.hash(&mut hash);
    let replay = crate::lock(input.store).restart_recognition_epoch(
        input.session,
        &input.scope.generation,
        input.range.start,
        input.range.end,
    )?;
    let mut helper = Helper::spawn(input.helper, hash.finish(), replay.scope.epoch)
        .map_err(|e| failed(e.to_string()))?;
    let mut output = Output {
        store: input.store.clone(),
        session: input.session.into(),
        scope: replay.scope,
        start: input.range.start,
        turns: None,
        pending: None,
        coverage: None,
        interim_result: None,
    };
    helper.send(json!({"command":"start", "route":input.native_route, "speaker_model":input.speaker, "asr_model":input.asr, "gpu":gpu, "language":input.language.unwrap_or(""), "epoch_start":input.range.start, "max_epoch_samples":MAX_EPOCH_SAMPLES,"streaming":input.streaming}),vec![]).map_err(|e| failed(e.to_string()))?;
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
        if let Some(meter) = speakers.as_deref_mut() {
            meter.at(at - input.range.start);
        }
    }
    if let Some(meter) = speakers {
        meter.unmeasured();
    }
    helper
        .send(json!({"command":"finish"}), vec![])
        .map_err(|e| failed(e.to_string()))?;
    until(&mut helper, &mut output, "finished", control)?;
    if output.coverage != Some(input.range) {
        return Err(failed("NeMo terminal coverage is incomplete"));
    }
    Ok((
        output.result(if input.native_route == "separate" {
            Route::SeparateNemo
        } else {
            Route::CombinedNemo
        })?,
        output.scope,
    ))
}

/// A native stream keeps its state across capture notifications and disk flushes.
struct LiveNative {
    helper: Helper,
    output: Output,
    route: Route,
    end: u64,
    native_route: String,
}
impl LiveNative {
    fn receive(
        &mut self,
        event: &str,
        control: &dyn Control,
        sink: &mut dyn crate::live::LiveSink,
    ) -> Result<(), StageError> {
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            if control.is_cancelled() {
                return Err(StageError::Cancelled);
            }
            match self.helper.receive(Duration::from_millis(100)) {
                Ok(frame) => {
                    let value = &frame.metadata;
                    if value["event"] == "hello"
                        && !value["routes"].as_array().is_some_and(|routes| {
                            routes.iter().any(|route| route == &self.native_route)
                        })
                    {
                        return Err(failed(
                            "the native helper does not support the selected live route",
                        ));
                    }
                    let cleared_interim = value["event"] == "words"
                        && self.output.interim_result == value["result_id"].as_u64();
                    self.output.process(value)?;
                    if cleared_interim {
                        sink.interim_snapshot(&self.output.scope, number(value, "revision")?, true);
                        sink.interim_snapshot(
                            &self.output.scope,
                            number(value, "revision")?,
                            false,
                        );
                    }
                    if value["event"] == "segments_begin" || value["event"] == "segments_end" {
                        sink.snapshot(
                            &self.output.scope,
                            number(value, "revision")?,
                            value["event"] == "segments_begin",
                        );
                    } else if value["event"] == "interim" {
                        let revision = number(value, "revision")?;
                        sink.interim_snapshot(&self.output.scope, revision, true);
                        self.output.visit(&self.output.prefix("interim"), |block| {
                            sink.recognition(&block);
                            Ok(())
                        })?;
                        sink.interim_snapshot(&self.output.scope, revision, false);
                    } else if value["event"] == "words" {
                        let result = number(value, "result_id")?;
                        for word in array(value, "words")? {
                            let index = number(word, "index")?;
                            let key =
                                format!("{}{result:020}/{index:020}", self.output.prefix("word"));
                            if let Some(block) = crate::lock(&self.output.store).recognition_block(
                                &self.output.session,
                                &self.output.scope.generation,
                                &key,
                            )? {
                                sink.recognition(&block);
                            }
                        }
                    } else if value["event"] == "segments" {
                        let revision = number(value, "revision")?;
                        let count = self.output.pending.map_or(0, |(_, count)| count);
                        let length = array(value, "segments")?
                            .iter()
                            .filter(|s| s["speaker"].as_u64().unwrap_or(0) > 0)
                            .count() as u64;
                        for index in count.saturating_sub(length)..count {
                            let key = format!(
                                "{}{:020}/{index:020}",
                                self.output.prefix("turn"),
                                revision
                            );
                            if let Some(block) = crate::lock(&self.output.store).recognition_block(
                                &self.output.session,
                                &self.output.scope.generation,
                                &key,
                            )? {
                                sink.recognition(&block);
                            }
                        }
                    }
                    if value["event"] == event {
                        if event == "ack" && number(value, "accepted_end")? != self.end {
                            return Err(failed("live input acknowledgement mismatch"));
                        }
                        return Ok(());
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::TimedOut
                        && Instant::now() < deadline => {}
                Err(error) => return Err(failed(format!("live NeMo helper: {error}"))),
            }
        }
    }
    fn push(
        &mut self,
        pcm: Vec<f32>,
        control: &dyn Control,
        sink: &mut dyn crate::live::LiveSink,
    ) -> Result<(), StageError> {
        let start = self.end;
        self.end += pcm.len() as u64;
        self.helper
            .send(json!({"command":"push", "offset":start}), pcm)
            .map_err(|e| failed(e.to_string()))?;
        self.receive("ack", control, sink)
    }
    fn finish(
        mut self,
        control: &dyn Control,
        sink: &mut dyn crate::live::LiveSink,
    ) -> Result<(), StageError> {
        self.helper
            .send(json!({"command":"finish"}), vec![])
            .map_err(|e| failed(e.to_string()))?;
        self.receive("finished", control, sink)?;
        let range = SampleRange {
            start: self.output.start,
            end: self.end,
        };
        if self.output.coverage != Some(range) {
            return Err(failed("live terminal coverage is incomplete"));
        }
        let mut result = self.output.result(self.route)?;
        if self.native_route == "combined_offline" {
            join_separate(&mut result.rows, &result.speakers);
        }
        let record = persist_epoch(
            &self.output.store,
            &self.output.session,
            &self.output.scope,
            &result,
            &self.native_route,
        )?;
        let stage = if self.route == Route::SeparateNemo {
            session_core::Stage::Diarize
        } else {
            session_core::Stage::Transcribe
        };
        crate::checkpoint::put(
            &self.output.store,
            &self.output.session,
            stage,
            &format!("recognition-epoch-{:020}", range.start),
            &record,
        )?;
        Ok(())
    }
}

pub(crate) fn run_live(
    store: &crate::SharedStore,
    config: &crate::live::LiveConfig,
    provider: &dyn EngineProvider,
    cancel: &tokio_util::sync::CancellationToken,
    end: &crate::live::CaptureEnd,
    sink: &mut dyn crate::live::LiveSink,
) -> Result<(), StageError> {
    let spec = config
        .snapshot
        .as_ref()
        .ok_or_else(|| failed("live recognition snapshot is absent"))?;
    let archive = config
        .archive
        .clone()
        .ok_or_else(|| failed("live recording source is absent"))?;
    let nemo_asr = super::TaskGraph::resolve(spec)?.route == Route::CombinedNemo;
    let combined = nemo_asr && spec.enabled_live_transcription;
    let text_only = combined && !spec.enabled_live_speaker_analysis;
    let helper_path = provider.nemo_helper().map_err(StageError::Waiting)?;
    let speaker_id = match spec.config.speaker_source {
        models::recognition::SpeakerSource::Sortformer => "sortformer-v2-q80",
        models::recognition::SpeakerSource::Nemotron => "nemotron-3-diarization-q80",
        _ => return Err(failed("live NeMo speaker artifact is absent")),
    };
    let artifact = spec
        .artifacts
        .iter()
        .find(|a| a.id == speaker_id)
        .ok_or_else(|| failed("live speaker artifact is absent"))?;
    let speaker = if text_only {
        None
    } else {
        Some(
            provider
                .artifact_path(artifact)
                .map_err(StageError::Waiting)?,
        )
    };
    let asr = if combined {
        Some(
            provider
                .artifact_path(&spec.artifacts[0])
                .map_err(StageError::Waiting)?,
        )
    } else {
        None
    };
    let generation = super::stages::generation(store, &config.note_id)?;
    let control = LiveControl(cancel.clone());
    let mut reader = crate::live::archive::ArchiveReader::new(archive.clone());
    let mut resampler = crate::resample::StreamResampler::new(archive.rate, 16000);
    let mut pending = Vec::new();
    let mut active: Option<LiveNative> = None;
    let mut at = archive.offset_ms * 16;
    let mut flushed = false;
    loop {
        control.proceed()?;
        if *end.lock().unwrap_or_else(|e| e.into_inner()) == Some(crate::live::LiveEnd::Discard) {
            return Ok(());
        }
        if !flushed {
            match next_step(end, || reader.next_frame())? {
                Step::Frame(frame) => {
                    resampler.push(&frame.pcm, &mut |s| pending.extend_from_slice(s))
                }
                Step::Finish => {
                    resampler.finish(&mut |s| pending.extend_from_slice(s));
                    flushed = true;
                }
                Step::Wait => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            }
        }
        if pending.len() < 16000 && !flushed {
            continue;
        }
        if !pending.is_empty() {
            if active.is_none() {
                let replay = crate::lock(store).restart_recognition_epoch(
                    &config.note_id,
                    &generation,
                    at,
                    at + MAX_EPOCH_SAMPLES,
                )?;
                let mut helper = Helper::spawn(&helper_path, 1, replay.scope.epoch)
                    .map_err(|e| failed(e.to_string()))?;
                let native_route = if text_only {
                    "asr_only"
                } else if !combined {
                    "separate"
                } else if spec.config.asr_model == "parakeet-tdt-0.6b-v3-q80" {
                    "combined_offline"
                } else {
                    "combined_stream"
                };
                helper.send(json!({"command":"start", "route":native_route,"speaker_model":speaker,"asr_model":asr,"gpu":provider.nemo_gpu(),"language":spec.config.language.as_deref().unwrap_or(""),"epoch_start":at,"max_epoch_samples":MAX_EPOCH_SAMPLES,"streaming":spec.config.asr_model != "parakeet-tdt-0.6b-v3-q80"}),vec![]).map_err(|e| failed(e.to_string()))?;
                let mut native = LiveNative {
                    helper,
                    output: Output {
                        store: store.clone(),
                        session: config.note_id.clone(),
                        scope: replay.scope,
                        start: at,
                        turns: None,
                        pending: None,
                        coverage: None,
                        interim_result: None,
                    },
                    route: if text_only {
                        Route::Off
                    } else if combined {
                        Route::CombinedNemo
                    } else {
                        Route::SeparateNemo
                    },
                    end: at,
                    native_route: native_route.into(),
                };
                native.receive("ready", &control, sink)?;
                active = Some(native);
            }
            let native = active.as_mut().unwrap();
            let count = pending
                .len()
                .min(16000)
                .min((native.output.start + MAX_EPOCH_SAMPLES - at) as usize);
            let pcm = pending.drain(..count).collect();
            native.push(pcm, &control, sink)?;
            at += count as u64;
            if at == native.output.start + MAX_EPOCH_SAMPLES {
                active.take().unwrap().finish(&control, sink)?;
                crate::live::archive::save_proof(store, config, &reader)?;
            }
        } else if flushed {
            if let Some(native) = active.take() {
                native.finish(&control, sink)?;
            }
            crate::live::archive::save_proof(store, config, &reader)?;
            return Ok(());
        }
    }
}
struct LiveControl(tokio_util::sync::CancellationToken);
impl Control for LiveControl {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
    fn proceed(&self) -> Result<(), StageError> {
        if self.is_cancelled() {
            Err(StageError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output() -> Output {
        let mut store = session_core::Store::open_in_memory().unwrap();
        store
            .create_session(
                "test",
                chrono::DateTime::parse_from_rfc3339("2026-09-27T10:00:00-05:00").unwrap(),
                None,
            )
            .unwrap();
        Output {
            store: std::sync::Arc::new(std::sync::Mutex::new(store)),
            session: "test".into(),
            scope: SpeakerScope {
                generation: "test".into(),
                epoch: 1,
            },
            start: 32000,
            turns: None,
            pending: None,
            coverage: None,
            interim_result: None,
        }
    }
    #[test]
    fn persisted_epoch_modality_uses_native_operation_even_for_empty_combined_route() {
        let out = output();
        let result = RouteResult {
            route: Route::CombinedNemo,
            rows: vec![],
            speakers: vec![],
            coverage: vec![SampleRange {
                start: 0,
                end: 16000,
            }],
            coverage_limitations: vec![],
        };
        for (native_route, text, speakers) in [
            ("asr_only", true, false),
            ("separate", false, true),
            ("combined_stream", true, true),
        ] {
            let record =
                persist_epoch(&out.store, "test", &out.scope, &result, native_route).unwrap();
            let saved = serde_json::to_value(record).unwrap();
            assert_eq!(
                saved["output"],
                json!({ "text": text, "speakers": speakers })
            );
            assert_eq!(saved["rows"], 0);
            assert_eq!(saved["speakers"], 0);
        }
    }

    #[test]
    fn speaker_snapshot_marker_excludes_incomplete_revisions_and_keeps_empty_completion() {
        let mut out = output();
        let marker_key = format!("nemo/{:020}/turn-commit", out.scope.epoch);
        let marker = |out: &Output| {
            crate::lock(&out.store)
                .recognition_block(&out.session, &out.scope.generation, &marker_key)
                .unwrap()
        };
        out.process(&json!({"event":"segments_begin","revision":1}))
            .unwrap();
        out.process(&json!({"event":"segments","revision":1,"segments":[{"start":32000,"end":48000,"speaker":1,"speaker_final":true}]})).unwrap();
        assert!(marker(&out).is_none());
        out.process(&json!({"event":"segments_end","revision":1}))
            .unwrap();
        assert_eq!(marker(&out).unwrap().revision, 1);
        out.process(&json!({"event":"segments_begin","revision":2}))
            .unwrap();
        out.process(&json!({"event":"segments","revision":2,"segments":[{"start":32000,"end":48000,"speaker":2,"speaker_final":true}]})).unwrap();
        assert_eq!(marker(&out).unwrap().revision, 1);
        assert!(
            out.process(&json!({"event":"segments_end","revision":3}))
                .is_err()
        );
        assert_eq!(marker(&out).unwrap().revision, 1);
        out.process(&json!({"event":"segments_begin","revision":4}))
            .unwrap();
        out.process(&json!({"event":"segments_end","revision":4}))
            .unwrap();
        let complete = marker(&out).unwrap();
        assert_eq!(complete.revision, 4);
        assert!(complete.speakers.is_empty());
        out.process(&json!({"event":"segments_begin","revision":3}))
            .unwrap();
        out.process(&json!({"event":"segments_end","revision":3}))
            .unwrap();
        assert_eq!(marker(&out).unwrap().revision, 4);
    }

    #[test]
    fn long_interim_is_bounded_and_does_not_enter_final_results() {
        let mut out = output();
        let words: Vec<_> = (0..600)
            .map(|i| json!({"text":"word", "start":32000+i*100,"end":32100+i*100,"speaker":1}))
            .collect();
        out.process(&json!({"event":"interim","result_id":1,"revision":1,"words":words}))
            .unwrap();
        let mut count = 0;
        out.visit(&out.prefix("interim"), |block| {
            assert!(block.rows.len() <= 256);
            count += block.rows.len();
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 600);
        out.process(&json!({"event":"words","result_id":1,"revision":2,"text_final":true,"words":[{"index":0,"text":"final", "start":32000,"end":32100,"speaker":1,"speaker_final":true}]})).unwrap();
        let mut interim = 0;
        out.visit(&out.prefix("interim"), |block| {
            interim += block.rows.len();
            Ok(())
        })
        .unwrap();
        assert_eq!(interim, 0);
        assert_eq!(out.word(1, 0).text, "final");
    }

    #[test]
    fn speaker_revision_does_not_make_provisional_labels_final() {
        let mut out = output();
        let patch = |revision, speaker, stable| json!({"event":"words","result_id":1,"revision":revision,"text_final":true,"words":[{"index":0,"text":"Hello","start":32000,"end":48000,"speaker":speaker,"speaker_final":stable}]});
        out.process(&patch(1, 1, false)).unwrap();
        assert_eq!(out.word(1, 0).speaker_finality, Finality::Provisional);
        out.process(&patch(2, 2, true)).unwrap();
        out.process(&patch(1, 1, false)).unwrap();
        assert_eq!(
            crate::lock(&out.store)
                .recognition_page("test", "test", &out.prefix("word"), "")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(out.word(1, 0).speaker.as_ref().unwrap().local_id, 2);
        assert_eq!(out.word(1, 0).speaker_finality, Finality::Final);
        out.process(&json!({"event":"punctuation","result_id":1,"revision":3,"text":"."}))
            .unwrap();
        assert_eq!(out.word(1, 0).text, "Hello.");
    }
    #[test]
    fn partial_speaker_snapshot_cannot_replace_completed_output() {
        let mut out = output();
        out.process(&json!({"event":"segments_begin","revision":1}))
            .unwrap();
        out.process(&json!({"event":"segments","revision":1,"segments":[{"start":32000,"end":48000,"speaker":1,"speaker_final":true}]})).unwrap();
        assert!(out.turns.is_none_or(|(_, count)| count == 0));
        assert!(out.process(&json!({"event":"finished","coverage_start":32000,"coverage_end":48000,"text_final":true,"speaker_final":true})).is_err());
        out.process(&json!({"event":"segments_end","revision":1}))
            .unwrap();
        assert_eq!(out.turns.unwrap().1, 1);
        out.process(&json!({"event":"segments_begin","revision":2}))
            .unwrap();
        assert_eq!(out.turns.unwrap().1, 1);
        out.process(&json!({"event":"segments_end","revision":2}))
            .unwrap();
        assert!(out.turns.is_none_or(|(_, count)| count == 0));
    }
    #[test]
    fn terminal_time_point_retains_native_speaker_and_unknown_duration() {
        let mut out = output();
        out.process(&json!({"event":"words","result_id":1,"revision":1,"text_final":true,"words":[{"index":0,"text":"tail","start":48000,"end":48000,"speaker":1,"speaker_final":true}]})).unwrap();
        assert_eq!(out.word(1, 0).timing, TimingPrecision::NativeTimePoint);
        assert_eq!(
            out.word(1, 0).range,
            SampleRange {
                start: 48000,
                end: 48000
            }
        );
        assert!(out.word(1, 0).speaker.is_some());
    }
}
