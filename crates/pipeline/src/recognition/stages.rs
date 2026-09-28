use super::{RESULT_KEY, Route, RouteResult, TaskGraph, failed, join_separate, validate_result};
use crate::{
    Control, SharedStore, StageError, checkpoint, lock, runner::EngineProvider, transcribe,
};
use asr::{
    Attribution, Finality, RecognitionId, RecognitionRequest, RecognitionSegment, SampleRange,
    SpeakerKey, SpeakerScope, SpeakerTurn, TimingPrecision,
};
use models::recognition::{ExecutionSpec, SpeakerCount};
use session_core::{Segment, Stage};
use std::{collections::BTreeMap, path::Path, sync::Mutex};

pub fn snapshot(store: &SharedStore, id: &str) -> Result<Option<ExecutionSpec>, StageError> {
    let record = lock(store)
        .session(id)?
        .ok_or_else(|| failed("session is absent"))?;
    let pending = lock(store).result_state(id, "pending-snapshot")?;
    let is_pending = pending.is_some();
    let Some(value) = pending.or(record.recognition_snapshot) else {
        return Ok(None);
    };
    let spec: ExecutionSpec = serde_json::from_value(value)?;
    TaskGraph::resolve(&spec)?;
    let count = if is_pending {
        spec.config.speaker_count
    } else {
        record.speaker_count.map(|c| match c {
            session_core::SpeakerCount::Exact(count) => SpeakerCount::Exact { count },
            session_core::SpeakerCount::Range { min, max } => SpeakerCount::Range { min, max },
        })
    };
    spec.validate_count(count)
        .map_err(|e| failed(e.to_string()))?;
    Ok(Some(spec))
}
/// A processing attempt keeps its generation across checkpoints. A replacement
/// that clears recognition checkpoints must create a fresh generation.
pub(super) fn generation(store: &SharedStore, id: &str) -> Result<String, StageError> {
    const KEY: &str = "recognition-generation-v1";
    if let Some(saved) = checkpoint::get(store, id, Stage::Transcribe, KEY)? {
        return Ok(saved);
    }
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let value = format!(
        "{id}:{}:{}:{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    checkpoint::put(store, id, Stage::Transcribe, KEY, &value)?;
    Ok(value)
}
pub fn saved_result(store: &SharedStore, id: &str) -> Result<Option<RouteResult>, StageError> {
    super::durable::load_result(store, id)
}

/// Execute one route-specific recognition stage. Other stages remain owned by
/// the runner.
#[allow(clippy::too_many_arguments)]
pub fn execute(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
    spec: &ExecutionSpec,
    provider: &dyn EngineProvider,
    stage: Stage,
    control: &dyn Control,
    progress: &mut dyn FnMut(transcribe::Progress),
) -> Result<(), StageError> {
    let graph = TaskGraph::resolve(spec)?;
    super::durable::prepare(store, id, dir, segments, spec)?;
    match stage {
        Stage::Diarize => analyze(
            store,
            id,
            dir,
            segments,
            spec,
            provider,
            graph.route,
            control,
        ),
        Stage::Transcribe => transcribe_route(
            store,
            id,
            dir,
            segments,
            spec,
            provider,
            graph.route,
            control,
            progress,
        ),
        Stage::Join => write_transcript(store, id, dir, segments),
        Stage::Verify => verify(store, id, dir, segments),
        _ => Err(failed("stage is not part of the recognition route")),
    }
}
fn ranges(dir: &Path, segments: &[Segment]) -> Result<Vec<SampleRange>, StageError> {
    segments
        .iter()
        .map(|s| {
            let reader = crate::assemble::open_proc(dir, s)?;
            Ok(SampleRange {
                start: s.offset_ms * 16,
                end: s.offset_ms * 16 + reader.duration() as u64,
            })
        })
        .collect()
}
#[allow(clippy::too_many_arguments)]
fn analyze(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
    spec: &ExecutionSpec,
    provider: &dyn EngineProvider,
    route: Route,
    control: &dyn Control,
) -> Result<(), StageError> {
    let generation = generation(store, id)?;
    let mut turns = Vec::new();
    let mut limitations = Vec::<String>::new();
    match route {
        Route::SeparatePyannote => {
            let analysis_scope = lock(store)
                .restart_recognition_epoch(id, &generation, 0, 0)?
                .scope;
            let mut analyzer = provider.analyzer_exact(spec).map_err(|e| match e {
                diarize::DiarizeError::MissingWeights(path) => StageError::Waiting(path),
                e => failed(e.to_string()),
            })?;
            let count = if lock(store).result_state(id, "pending-snapshot")?.is_some() {
                crate::reprocess::speaker_count(spec)
            } else {
                lock(store).session(id)?.and_then(|r| r.speaker_count)
            };
            crate::diarize::run(
                store,
                id,
                dir,
                segments,
                count,
                analyzer.as_mut(),
                control,
                &mut |_, _| {},
            )?;
            let crate::diarize::Speakers::Turns(native) = crate::diarize::speakers(store, id)?
            else {
                return Err(failed("requested speaker analysis did not complete"));
            };
            for turn in native {
                let segment = segments
                    .iter()
                    .find(|s| s.index == turn.segment)
                    .ok_or_else(|| failed("speaker turn references absent segment"))?;
                turns.push(SpeakerTurn {
                    range: SampleRange {
                        start: (segment.offset_ms + turn.start_ms) * 16,
                        end: (segment.offset_ms + turn.end_ms) * 16,
                    },
                    timing: TimingPrecision::NativeSegment,
                    speaker: SpeakerKey {
                        scope: analysis_scope.clone(),
                        input_start_sample: 0,
                        local_id: turn.speaker,
                    },
                    finality: Finality::Final,
                });
            }
        }
        Route::SeparateNemo | Route::CombinedNemo => {
            for (segment, range) in segments.iter().zip(ranges(dir, segments)?) {
                let result = super::nemo::run_file(
                    store,
                    id,
                    spec,
                    provider,
                    &dir.join(crate::proc_file(&segment.audio_file)),
                    range,
                    &generation,
                    false,
                    control,
                )?;
                turns.extend(result.speakers);
                limitations.extend(result.coverage_limitations);
            }
        }
        _ => return Err(failed("this route does not use separate speaker analysis")),
    }
    checkpoint::put(
        store,
        id,
        Stage::Diarize,
        "recognition-speaker-limitations-v1",
        &limitations,
    )?;
    super::durable::persist_speakers(store, id, &turns)?;
    checkpoint::mark_complete(store, id, Stage::Diarize, segments.len())
}
#[allow(clippy::too_many_arguments)]
fn transcribe_route(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
    spec: &ExecutionSpec,
    provider: &dyn EngineProvider,
    route: Route,
    control: &dyn Control,
    progress: &mut dyn FnMut(transcribe::Progress),
) -> Result<(), StageError> {
    let expected = ranges(dir, segments)?;
    let generation = generation(store, id)?;
    let speakers: Vec<SpeakerTurn> = if TaskGraph::resolve(spec)?.separate_analysis {
        super::durable::load_speakers(store, id)?
            .ok_or_else(|| failed("speaker task has not completed"))?
    } else {
        vec![]
    };
    let limitations = checkpoint::get(
        store,
        id,
        Stage::Diarize,
        "recognition-speaker-limitations-v1",
    )?
    .unwrap_or_default();
    let mut result = RouteResult {
        route,
        coverage: vec![],
        rows: vec![],
        speakers,
        coverage_limitations: limitations,
    };
    let pending = lock(store).result_state(id, "pending-snapshot")?.is_some();
    if pending && !matches!(route, Route::Integrated | Route::CombinedNemo) {
        let home = lock(store)
            .session(id)?
            .and_then(|r| r.home_path)
            .map(std::path::PathBuf::from);
        if let Some(home) = home {
            let doc = session_core::generation::manifest(&home)?;
            let mut rows = Vec::new();
            if let Some(header) = crate::reprocess::read_reusable(&home, &doc, spec, |block| {
                for mut row in block.rows {
                    row.id.scope.generation = generation.clone();
                    row.speaker = None;
                    row.speaker_finality = Finality::Final;
                    rows.push(row);
                }
            })? {
                result.rows = rows;
                result.coverage = header.coverage;
                if route != Route::Off {
                    join_separate(&mut result.rows, &result.speakers);
                }
                validate_result(&result, &expected)?;
                super::durable::persist_result(store, id, &result)?;
                return checkpoint::mark_complete(store, id, Stage::Transcribe, segments.len());
            }
        }
    }
    if matches!(
        route,
        Route::Off | Route::SeparateNemo | Route::SeparatePyannote
    ) {
        // Reuse successful live text and the established ordered gap-fill
        // contract. The fresh slot prevents reuse of a different artifact.
        let live_rows = crate::live::rows(store, id)?;
        let mut needs_batch = false;
        for segment in segments {
            if !live_rows.iter().any(|row| row.segment == segment.index)
                && !checkpoint::get::<bool>(
                    store,
                    id,
                    Stage::Transcribe,
                    &checkpoint::segment_done_key(segment.index),
                )?
                .unwrap_or(false)
            {
                needs_batch = true;
                break;
            }
        }
        // A completed live result needs no additional model load. New batch
        // work queries the actual loaded artifact's timestamp capabilities.
        let loaded = if needs_batch {
            Some(provider.load_exact(spec).map_err(failed)?)
        } else {
            None
        };
        let native_tokens = loaded
            .as_ref()
            .is_some_and(|engine| engine.native_token_timestamps());
        let slot = Mutex::new(loaded);
        let load = |_: &str| provider.load_exact(spec);
        let engine = transcribe::Slot {
            slot: &slot,
            model: &spec.config.asr_model,
            load: &load,
        };
        let cuts: transcribe::TurnCuts =
            if !native_tokens && matches!(route, Route::SeparatePyannote | Route::SeparateNemo) {
                segments
                    .iter()
                    .map(|segment| {
                        let start = segment.offset_ms * 16;
                        let end = start + segment.duration_ms * 16;
                        let mut turns: Vec<_> = result
                            .speakers
                            .iter()
                            .filter(|t| t.range.start < end && t.range.end > start)
                            .collect();
                        turns.sort_by_key(|t| t.range.start);
                        let mut previous = None;
                        let mut cuts = Vec::new();
                        for turn in turns {
                            if previous.is_some_and(|key: &SpeakerKey| key != &turn.speaker)
                                && turn.range.start > start
                            {
                                cuts.push((turn.range.start - start) / 16);
                            }
                            previous = Some(&turn.speaker);
                        }
                        cuts.sort_unstable();
                        cuts.dedup();
                        (segment.index, cuts)
                    })
                    .collect()
            } else {
                BTreeMap::new()
            };
        transcribe::run_with_timing(
            store,
            id,
            dir,
            segments,
            &cuts,
            &engine,
            native_tokens,
            control,
            progress,
        )?;
        for (ordinal, window) in transcribe::results(store, id)?.into_iter().enumerate() {
            let segment = segments
                .iter()
                .find(|s| s.index == window.segment)
                .ok_or_else(|| failed("text references absent segment"))?;
            let scope = SpeakerScope {
                generation: generation.clone(),
                epoch: segment.index as u64,
            };
            let mut pieces = Vec::new();
            if let Some(words) = window.words {
                pieces.extend(
                    words
                        .into_iter()
                        .map(|w| (w.start_ms, w.end_ms, w.text, TimingPrecision::NativeSegment)),
                );
                result
                    .coverage_limitations
                    .push("Word timestamps are model estimates, not exact word alignments.".into());
            } else if window.segments.is_empty() && !window.text.trim().is_empty() {
                pieces.push((
                    window.start_ms,
                    window.end_ms,
                    window.text,
                    TimingPrecision::SourceWindow,
                ));
            } else {
                pieces.extend(window.segments.into_iter().map(|s| {
                    if s.start_ms == 0 && s.end_ms == 0 {
                        (
                            window.start_ms,
                            window.end_ms,
                            s.text,
                            TimingPrecision::SourceWindow,
                        )
                    } else {
                        (
                            window.start_ms + s.start_ms,
                            window.start_ms + s.end_ms,
                            s.text,
                            TimingPrecision::NativeSegment,
                        )
                    }
                }));
            }
            for (index, (start, end, text, timing)) in pieces.into_iter().enumerate() {
                if text.trim().is_empty() {
                    continue;
                }
                result.rows.push(RecognitionSegment {
                    id: RecognitionId {
                        scope: scope.clone(),
                        input_start_sample: (segment.offset_ms + window.start_ms) * 16,
                        ordinal: ((ordinal as u64) << 32) | index as u64,
                    },
                    range: SampleRange {
                        start: (segment.offset_ms + start) * 16,
                        end: (segment.offset_ms + end) * 16,
                    },
                    timing: if start == end {
                        TimingPrecision::NativeTimePoint
                    } else {
                        timing
                    },
                    text,
                    speaker: None,
                    revision: 0,
                    text_finality: Finality::Final,
                    speaker_finality: Finality::Final,
                });
            }
        }
        result.coverage = expected.clone();
        if route != Route::Off {
            join_separate(&mut result.rows, &result.speakers);
        }
    } else {
        if route == Route::CombinedNemo && !crate::live::rows(store, id)?.is_empty() {
            return Err(failed(
                "saved live text lacks this route's speaker output; a second transcription pass requires an explicit replacement choice",
            ));
        }
        let mut recognizer = None;
        for (segment, range) in segments.iter().zip(&expected) {
            control.proceed()?;
            let output = {
                let mut own = RouteResult {
                    route,
                    coverage: vec![*range],
                    rows: vec![],
                    speakers: vec![],
                    coverage_limitations: vec![],
                };
                if route == Route::CombinedNemo {
                    own = super::nemo::run_file(
                        store,
                        id,
                        spec,
                        provider,
                        &dir.join(crate::proc_file(&segment.audio_file)),
                        *range,
                        &generation,
                        true,
                        control,
                    )?;
                } else {
                    let plan = transcribe::plan_segment(dir, segment, &[])?;
                    checkpoint::put(
                        store,
                        id,
                        Stage::Transcribe,
                        &checkpoint::plan_key(segment.index),
                        &plan,
                    )?;
                    let mut reader = crate::assemble::open_proc(dir, segment)?;
                    for (index, window) in plan.windows.iter().enumerate() {
                        control.proceed()?;
                        let seek = u32::try_from(window.start_ms * 16)
                            .map_err(|_| failed("audio seek overflow"))?;
                        let sample_count = (window.duration_ms() * 16)
                            .min(u64::from(reader.duration().saturating_sub(seek)));
                        let key = format!("native-window/{:08}/{index:08}", segment.index);
                        let saved = lock(store).recognition_block(id, &generation, &key)?;
                        let output = if let Some(saved) = saved {
                            asr::RecognitionOutput {
                                scope: saved.scope,
                                revision: saved.revision,
                                coverage: saved
                                    .coverage
                                    .committed
                                    .ok_or_else(|| failed("native window is provisional"))?,
                                text: String::new(),
                                segments: saved.rows,
                                speaker_turns: saved.speakers,
                            }
                        } else {
                            let engine = match &mut recognizer {
                                Some(engine) => engine,
                                None => {
                                    recognizer.insert(provider.load_exact(spec).map_err(failed)?)
                                }
                            };
                            reader.seek(seek)?;
                            let pcm = reader
                                .samples::<f32>()
                                .take(sample_count as usize)
                                .collect::<Result<Vec<_>, _>>()?;
                            engine
                                .transcribe_normalized(
                                    &pcm,
                                    &RecognitionRequest {
                                        scope: SpeakerScope {
                                            generation: generation.clone(),
                                            epoch: segment.index as u64,
                                        },
                                        start_sample: range.start + window.start_ms * 16,
                                        revision: 0,
                                        attribution: Attribution::Native,
                                    },
                                )
                                .map_err(|e| failed(e.to_string()))?
                        };
                        if output.coverage
                            != (SampleRange {
                                start: range.start + window.start_ms * 16,
                                end: range.start + window.start_ms * 16 + sample_count,
                            })
                        {
                            return Err(failed("native transcription coverage is incomplete"));
                        }
                        lock(store).commit_recognition(
                            id,
                            &session_core::recognition::OutputBlock {
                                key,
                                scope: output.scope.clone(),
                                revision: output.revision,
                                coverage: session_core::recognition::Coverage {
                                    received: Some(output.coverage),
                                    computed: Some(output.coverage),
                                    provisional: None,
                                    committed: Some(output.coverage),
                                },
                                rows: output.segments.clone(),
                                speakers: output.speaker_turns.clone(),
                            },
                        )?;
                        own.rows.extend(output.segments);
                        own.speakers.extend(output.speaker_turns);
                        progress(transcribe::Progress {
                            segment: segment.index,
                            windows_done: index + 1,
                            windows_total: plan.windows.len(),
                        });
                    }
                }
                own
            };
            result.coverage.extend(output.coverage);
            result.rows.extend(output.rows);
            result.speakers.extend(output.speakers);
            result
                .coverage_limitations
                .extend(output.coverage_limitations);
        }
    }
    if result
        .rows
        .iter()
        .any(|r| r.timing == TimingPrecision::NativeTimePoint)
    {
        result.coverage_limitations.push("Some text has a model time point with unknown duration; it contributes no timed speech coverage.".into());
    }
    if result
        .rows
        .iter()
        .any(|r| r.timing == TimingPrecision::SourceWindow)
    {
        result.coverage_limitations.push("Some text has source-window timing only. Timed word coverage cannot be established for that text.".into());
    }
    if route != Route::Off && result.rows.iter().any(|r| r.speaker.is_none()) {
        result
            .coverage_limitations
            .push("Some text has no unambiguous speaker assignment.".into());
    }
    if result.rows.iter().any(|r| r.speaker.is_some()) && super::found_speakers(&result).is_none() {
        result
            .coverage_limitations
            .push(super::SCOPED_LABELS_NOTE.into());
    }
    let invalidations: Vec<String> =
        checkpoint::get(store, id, Stage::Transcribe, "live-invalidation-reasons")?
            .unwrap_or_default();
    result.coverage_limitations.extend(invalidations);
    result.coverage_limitations.sort();
    result.coverage_limitations.dedup();
    if route == Route::CombinedNemo && TaskGraph::resolve(spec)?.separate_analysis {
        join_separate(&mut result.rows, &result.speakers);
    }
    validate_result(&result, &expected)?;
    super::durable::persist_result(store, id, &result)?;
    checkpoint::mark_complete(store, id, Stage::Transcribe, segments.len())
}
fn write_transcript(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
) -> Result<(), StageError> {
    let result = saved_result(store, id)?.ok_or_else(|| failed("recognition result is absent"))?;
    let transcript = super::transcript(&result);
    lock(store).set_found_speakers(id, super::found_speakers(&result))?;
    let temporary = dir.join("transcript.md.tmp");
    let text = if result.route == Route::Off {
        transcript.render_speaker_off()
    } else {
        transcript.render()
    };
    std::fs::write(&temporary, &text)?;
    crate::sync_file(&temporary)?;
    std::fs::rename(temporary, dir.join("transcript.md"))?;
    let spec = snapshot(store, id)?.ok_or_else(|| failed("recognition snapshot is absent"))?;
    let reference = super::durable::write_result(store, id, dir, &spec, &result, &text)?;
    checkpoint::put(
        store,
        id,
        Stage::Join,
        "recognition-artifact-reference-v1",
        &reference,
    )?;
    checkpoint::mark_complete(store, id, Stage::Join, segments.len())
}
fn verify(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
) -> Result<(), StageError> {
    let result = saved_result(store, id)?.ok_or_else(|| failed("recognition result is absent"))?;
    validate_result(&result, &ranges(dir, segments)?)?;
    for segment in segments {
        let plan = transcribe::plan_segment(dir, segment, &[])?;
        let start = segment.offset_ms * 16;
        let end = start + plan.duration_ms * 16;
        let own: Vec<_> = result
            .rows
            .iter()
            .filter(|r| r.range.start >= start && r.range.start < end)
            .collect();
        if plan.voiced_ms > 0 && own.is_empty() {
            return Err(failed(format!(
                "segment {} has speech but no text",
                segment.index
            )));
        }
        if own
            .iter()
            .any(|r| r.timing == TimingPrecision::SourceWindow)
        {
            // Preserve the evidence limit. Source-window text cannot establish
            // timed word coverage; accepted input must not masquerade as it.
            log::warn!(
                "segment {} has source-window timing; timed text coverage is unavailable",
                segment.index
            );
            continue;
        }
        let text: Vec<_> = own
            .iter()
            .filter(|r| r.timing != TimingPrecision::NativeTimePoint)
            .map(|r| asr::Span {
                start_ms: r.range.start.saturating_sub(start) / 16,
                end_ms: r.range.end.saturating_sub(start) / 16,
            })
            .collect();
        let coverage = crate::verify::measure(segment.index, &plan.voiced, &text);
        checkpoint::put(
            store,
            id,
            Stage::Verify,
            &checkpoint::segment_key(segment.index),
            &coverage,
        )?;
        if !coverage.passes() {
            let saved_plan: Option<transcribe::WindowPlan> = checkpoint::get(
                store,
                id,
                Stage::Transcribe,
                &checkpoint::plan_key(segment.index),
            )?;
            let redo = saved_plan
                .as_ref()
                .map(|p| {
                    (0..p.windows.len())
                        .map(|window| (segment.index, window))
                        .collect()
                })
                .unwrap_or_default();
            checkpoint::put(
                store,
                id,
                Stage::Verify,
                crate::verify::FAILED,
                &crate::verify::VerifyFailure {
                    segments: vec![coverage.clone()],
                    redo,
                },
            )?;
            return Err(failed(format!(
                "segment {} text covers {:.1}% of detected speech",
                segment.index,
                coverage.ratio() * 100.0
            )));
        }
    }
    checkpoint::mark_complete(store, id, Stage::Verify, segments.len())
}

/// A user-requested retry discards failed segment results, not successful live
/// rows or the original audio. Native state is reconstructed on the next run.
pub fn prepare_retry(store: &SharedStore, id: &str) -> Result<(), StageError> {
    let failure: Option<crate::verify::VerifyFailure> =
        checkpoint::get(store, id, Stage::Verify, crate::verify::FAILED)?;
    crate::verify::prepare_retry(store, id)?;
    let generation = generation(store, id)?;
    let guard = lock(store);
    // Native output is replayed after a failed verification. Its old completed
    // output remains in other generations for replacement recovery.
    guard.delete_recognition_prefix(id, &generation, "native-window/")?;
    for (key, _) in guard.checkpoints(id, Stage::Transcribe)? {
        if key.starts_with("recognition-epoch-") {
            guard.delete_checkpoint(id, Stage::Transcribe, &key)?;
        }
    }
    if let Some(failure) = failure {
        for segment in failure.segments {
            guard.delete_checkpoint(
                id,
                Stage::Transcribe,
                &format!("recognition-segment-{}", segment.segment),
            )?;
        }
    } else {
        // A structural/finality error has no timed coverage measurement.
        for (key, _) in guard.checkpoints(id, Stage::Transcribe)? {
            if key.starts_with("recognition-segment-") {
                guard.delete_checkpoint(id, Stage::Transcribe, &key)?;
            }
        }
    }
    guard.delete_checkpoint(id, Stage::Transcribe, RESULT_KEY)?;
    guard.delete_checkpoint(id, Stage::Transcribe, checkpoint::DONE)?;
    guard.delete_checkpoint(id, Stage::Join, checkpoint::DONE)?;
    guard.clear_checkpoints(id, Stage::Verify)?;
    Ok(())
}
