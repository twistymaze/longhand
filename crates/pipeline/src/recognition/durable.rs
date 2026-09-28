//! Exact compatibility checks precede checkpoint reuse and stage shortcuts.
use super::{RouteResult, failed};
use crate::{SharedStore, StageError, checkpoint, lock};
use models::recognition::{ExecutionSpec, SpeakerSource};
use session_core::{Segment, Stage, recognition::*};
use std::path::Path;

pub const IDENTITY_KEY: &str = "recognition-identity-v1";
pub const LIVE_IDENTITY_KEY: &str = "recognition-liveidentity-v1";

fn hash(value: &impl serde::Serialize) -> Result<String, StageError> {
    Ok(digest(&serde_json::to_vec(value)?))
}
pub(crate) fn spec_fingerprints(
    audio: String,
    spec: &ExecutionSpec,
) -> Result<Fingerprints, StageError> {
    if spec.artifacts.is_empty() {
        return Err(failed("recognition snapshot has no ASR artifact"));
    }
    // A separate speaker runtime cannot change the transcription identity.
    let asr_runtime = if spec
        .artifacts
        .first()
        .is_some_and(|a| a.format == "nemo-gguf")
    {
        "NeMo-Speech.cpp"
    } else {
        "transcribe.cpp"
    };
    let runtime = spec.runtimes.iter().find(|r| r.name == asr_runtime);
    let integrated = spec.config.speaker_source == SpeakerSource::Integrated;
    Ok(Fingerprints {
        execution: hash(spec)?,
        asr: hash(&(
            spec.artifacts.first(),
            runtime,
            &spec.config.language,
            integrated,
        ))?,
        speakers: hash(&(
            spec.config.speaker_source,
            spec.config.speaker_count,
            &spec.artifacts[1..],
            &spec.runtimes,
        ))?,
        audio,
        preprocessing: hash(&spec.preprocessing_revision)?,
    })
}
pub fn fingerprints(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
    spec: &ExecutionSpec,
) -> Result<Fingerprints, StageError> {
    let mut value = spec_fingerprints(
        crate::assemble::recognition_audio_identity(store, id, dir, segments)?,
        spec,
    )?;
    let count = lock(store)
        .session(id)?
        .ok_or_else(|| failed("session is absent"))?
        .speaker_count;
    value.speakers = hash(&(value.speakers, count))?;
    value.execution = hash(&(value.execution, count))?;
    Ok(value)
}
/// Stamp only after the caller proves the live intervals came from this durable
/// recording and this preprocessing revision. Pre-Opus capture PCM and decoded
/// audio need not have equal bytes. A caller must establish recording provenance
/// and exclude each changed echo-removal interval before calling this function.
/// This function records that proof; hashing the final WAV alone is not proof.
pub fn stamp_live_identity(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
    spec: &ExecutionSpec,
) -> Result<(), StageError> {
    for segment in segments {
        if !live_source_matches(store, id, dir, segment, spec)? {
            return Err(failed(
                "live recognition has no matching durable source and preprocessing proof",
            ));
        }
    }
    checkpoint::put(
        store,
        id,
        Stage::Transcribe,
        LIVE_IDENTITY_KEY,
        &fingerprints(store, id, dir, segments, spec)?,
    )
}
fn live_source_matches(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segment: &Segment,
    spec: &ExecutionSpec,
) -> Result<bool, StageError> {
    let proof: Option<crate::live::archive::SourceProof> = checkpoint::get(
        store,
        id,
        Stage::Transcribe,
        &crate::live::archive::key(segment.index),
    )?;
    let recorded: Option<ExecutionSpec> = checkpoint::get(
        store,
        id,
        Stage::Transcribe,
        &format!("live-snapshot-{:08}", segment.index),
    )?;
    let (Some(proof), Some(recorded)) = (proof, recorded) else {
        return Ok(false);
    };
    // Assemble uses final level estimates and echo removal on stereo input.
    // All live stereo samples can differ, including samples outside echo spans.
    if segment.channels != 1 || recorded != *spec {
        return Ok(false);
    }
    let mut reader = crate::live::archive::ArchiveReader::new(crate::live::archive::Archive {
        dir: dir.join(crate::RECOVERY_DIR),
        segment: segment.index,
        offset_ms: segment.offset_ms,
        rate: proof.rate,
    });
    while reader.position() < proof.samples && reader.next_frame()?.is_some() {}
    let covered_end = segment.offset_ms * 16 + proof.samples * 16000 / u64::from(proof.rate);
    if reader.proof() != proof {
        return Ok(false);
    }
    if crate::live::segment_rows(store, id, segment.index)?
        .iter()
        .any(|row| (segment.offset_ms + row.end_ms) * 16 > covered_end)
    {
        return Ok(false);
    }
    for stage in [Stage::Transcribe, Stage::Diarize] {
        for (key, value) in checkpoint::all(store, id, stage)? {
            let start = key
                .strip_prefix("recognition-epoch-")
                .and_then(|s| s.parse::<u64>().ok());
            if start.is_some_and(|at| {
                at >= segment.offset_ms * 16 && at < (segment.offset_ms + segment.duration_ms) * 16
            }) {
                let record: serde_json::Value = serde_json::from_str(&value)?;
                if record["range"]["end"]
                    .as_u64()
                    .is_none_or(|end| end > covered_end)
                {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

/// Bind live output only after checking its exact capture samples. An altered
/// preprocessing span invalidates text and labels together before final work.
fn bind_live(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
    spec: &ExecutionSpec,
    next: &Fingerprints,
) -> Result<(), StageError> {
    if checkpoint::get::<Fingerprints>(store, id, Stage::Transcribe, LIVE_IDENTITY_KEY)?.as_ref()
        == Some(next)
    {
        return Ok(());
    }
    let mut has_source = false;
    let mut valid = true;
    for segment in segments {
        let source: Option<crate::live::archive::SourceProof> = checkpoint::get(
            store,
            id,
            Stage::Transcribe,
            &crate::live::archive::key(segment.index),
        )?;
        if source.is_none() {
            valid = false;
            continue;
        }
        has_source = true;
        if !live_source_matches(store, id, dir, segment, spec)? {
            valid = false;
            let mut reasons: Vec<String> =
                checkpoint::get(store, id, Stage::Transcribe, "live-invalidation-reasons")?
                    .unwrap_or_default();
            let reason = if segment.channels == 2 {
                format!(
                    "Live text and speaker output for segment {} were invalidated because final stereo level and echo preprocessing differs from the live mix. This segment required recognition of the cleaned audio.",
                    segment.index + 1
                )
            } else {
                format!(
                    "Live text and speaker output for segment {} could not be verified against the saved audio and preprocessing identity. Recognition used the verified recorded audio.",
                    segment.index + 1
                )
            };
            if !reasons.contains(&reason) {
                reasons.push(reason);
            }
            checkpoint::put(
                store,
                id,
                Stage::Transcribe,
                "live-invalidation-reasons",
                &reasons,
            )?;
            crate::live::delete_segment(store, id, segment.index)?;
            // Consume the rejected proof once. Later prepare calls must keep
            // epochs computed from the final processing audio.
            lock(store).delete_checkpoint(
                id,
                Stage::Transcribe,
                &crate::live::archive::key(segment.index),
            )?;
            lock(store).delete_checkpoint(
                id,
                Stage::Transcribe,
                &format!("live-snapshot-{:08}", segment.index),
            )?;
            lock(store).delete_checkpoint(
                id,
                Stage::Diarize,
                &crate::live::features::key(segment.index),
            )?;
            let start = segment.offset_ms * 16;
            let end = start + segment.duration_ms * 16;
            for stage in [Stage::Transcribe, Stage::Diarize] {
                for (key, _) in checkpoint::all(store, id, stage)? {
                    if key
                        .strip_prefix("recognition-epoch-")
                        .and_then(|s| s.parse::<u64>().ok())
                        .is_some_and(|at| at >= start && at < end)
                    {
                        lock(store).delete_checkpoint(id, stage, &key)?;
                    }
                }
            }
        }
    }
    if has_source {
        if valid {
            checkpoint::put(store, id, Stage::Transcribe, LIVE_IDENTITY_KEY, next)?;
        }
        // Completed native epochs retain their verified durable input. Invalid
        // epochs above are absent and must be computed from the cleaned input.
        if checkpoint::get::<Fingerprints>(store, id, Stage::Transcribe, IDENTITY_KEY)?.is_none() {
            checkpoint::put(store, id, Stage::Transcribe, IDENTITY_KEY, next)?;
        }
    }
    Ok(())
}
/// Clear only incompatible dependent work. A missing fingerprint is not proof.
pub fn prepare(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
    spec: &ExecutionSpec,
) -> Result<Fingerprints, StageError> {
    let next = fingerprints(store, id, dir, segments, spec)?;
    bind_live(store, id, dir, segments, spec, &next)?;
    let old: Option<Fingerprints> = checkpoint::get(store, id, Stage::Transcribe, IDENTITY_KEY)?;
    if checkpoint::is_complete(store, id, Stage::Join, segments.len())? {
        let reference: Option<ArtifactReference> =
            checkpoint::get(store, id, Stage::Join, "recognition-artifact-reference-v1")?;
        if let Some(reference) = reference {
            let header = read_artifact(&dir.join(ARTIFACT_FILE), &reference, |_| Ok(()))
                .map_err(|e| failed(e.to_string()))?;
            if file_digest(&dir.join(crate::join::TRANSCRIPT_FILE))? != header.transcript {
                return Err(failed(
                    "Transcript changed outside recognition. Choose Transcribe Again explicitly; saved work is preserved.",
                ));
            }
        }
    }
    let live = crate::live::rows(store, id)?;
    let live_identity: Option<Fingerprints> =
        checkpoint::get(store, id, Stage::Transcribe, LIVE_IDENTITY_KEY)?;
    let live_matches = live_identity.as_ref().is_some_and(|f| {
        f.asr == next.asr && f.audio == next.audio && f.preprocessing == next.preprocessing
    });
    if !live.is_empty() && !live_matches && spec.config.speaker_source != SpeakerSource::Integrated
    {
        return Err(failed(
            "Saved live text has no compatible audio and model identity. Choose Transcribe Again explicitly.",
        ));
    }
    if old.as_ref() == Some(&next) {
        return Ok(next);
    }
    let asr_matches = old.as_ref().is_some_and(|f| {
        f.asr == next.asr && f.audio == next.audio && f.preprocessing == next.preprocessing
    });
    let result = super::saved_result(store, id)?;
    let timed = asr_matches
        && result.as_ref().is_some_and(|r| {
            r.rows
                .iter()
                .all(|row| row.timing != TimingPrecision::SourceWindow)
        });
    let guard = lock(store);
    for stage in [
        Stage::Diarize,
        Stage::Join,
        Stage::Verify,
        Stage::Summarize,
        Stage::Save,
    ] {
        if stage == Stage::Diarize {
            // Individual feature windows carry snapshot and PCM hashes. Keep
            // them for verification; discard the unverified completed index.
            for (key, _) in guard.checkpoints(id, stage)? {
                if !key.starts_with("live-feature-") {
                    guard.delete_checkpoint(id, stage, &key)?;
                }
            }
        } else {
            guard.clear_checkpoints(id, stage)?;
        }
    }
    for (key, _) in guard.checkpoints(id, Stage::Transcribe)? {
        let keep_live = live_matches && (key.contains("-live-") || key == LIVE_IDENTITY_KEY);
        let keep_asr = timed && (key.starts_with("seg-") || key == "recognition-generation-v1");
        if !keep_live && !keep_asr && key != "live-invalidation-reasons" {
            guard.delete_checkpoint(id, Stage::Transcribe, &key)?;
        }
    }
    drop(guard);
    checkpoint::put(store, id, Stage::Transcribe, IDENTITY_KEY, &next)?;
    Ok(next)
}

pub fn write_result(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    spec: &ExecutionSpec,
    result: &RouteResult,
    text: &str,
) -> Result<ArtifactReference, StageError> {
    let generation = super::stages::generation(store, id)?;
    let header = ArtifactHeader {
        version: ARTIFACT_VERSION,
        generation: generation.clone(),
        snapshot: serde_json::to_value(spec)?,
        fingerprints: checkpoint::get(store, id, Stage::Transcribe, IDENTITY_KEY)?
            .ok_or_else(|| failed("recognition identity is absent"))?,
        transcript: digest(text.as_bytes()),
        limitations: result.coverage_limitations.clone(),
        coverage: result.coverage.clone(),
    };
    let block = |key: String,
                 scope: SpeakerScope,
                 revision: u64,
                 range: SampleRange,
                 rows: Vec<RecognitionSegment>,
                 speakers: Vec<SpeakerTurn>| OutputBlock {
        key,
        scope,
        revision,
        coverage: Coverage {
            received: Some(range),
            computed: Some(range),
            provisional: None,
            committed: Some(range),
        },
        rows,
        speakers,
    };
    let coverage = result.coverage.iter().enumerate().map(|(i, r)| {
        block(
            format!("coverage-{i:08}"),
            SpeakerScope {
                generation: generation.clone(),
                epoch: 0,
            },
            0,
            *r,
            vec![],
            vec![],
        )
    });
    let rows = result.rows.iter().enumerate().map(|(i, r)| {
        block(
            format!("row-{i:012}"),
            r.id.scope.clone(),
            r.revision,
            r.range,
            vec![r.clone()],
            vec![],
        )
    });
    let speakers = result.speakers.iter().enumerate().map(|(i, s)| {
        block(
            format!("speaker-{i:012}"),
            s.speaker.scope.clone(),
            0,
            s.range,
            vec![],
            vec![s.clone()],
        )
    });
    write_artifact(
        &dir.join(ARTIFACT_FILE),
        &header,
        coverage.chain(rows).chain(speakers),
    )
    .map_err(|e| failed(e.to_string()))
}

/// The caller can present explicit retranscription for a legacy or edited note.
/// Import does not publish files or change a prior finished generation.
pub fn reusable_artifact(
    dir: &Path,
    reference: Option<&ArtifactReference>,
    next: &Fingerprints,
    mut visitor: impl FnMut(OutputBlock) -> Result<(), StageError>,
) -> Result<Reuse, StageError> {
    let Some(reference) = reference else {
        return Ok(Reuse::TranscribeAgain);
    };
    let transcript = std::fs::read(dir.join(crate::join::TRANSCRIPT_FILE))?;
    let mut timed = true;
    let header = read_artifact(&dir.join(ARTIFACT_FILE), reference, |b| {
        timed &= b.timed_asr();
        Ok(())
    })
    .map_err(|e| failed(e.to_string()))?;
    let reuse = header.reuse(next, &transcript);
    if reuse == Reuse::TranscribeAgain || (reuse == Reuse::TimedAsr && !timed) {
        return Ok(Reuse::TranscribeAgain);
    }
    // read_artifact verifies all bytes before calling the consumer.
    read_artifact(&dir.join(ARTIFACT_FILE), reference, |mut b| {
        if reuse == Reuse::TimedAsr {
            b.coverage.provisional = b.coverage.committed.take();
            b.speakers.clear();
            for row in &mut b.rows {
                row.speaker = None;
                row.speaker_finality = Finality::Provisional;
            }
        }
        visitor(b).map_err(|e| ArtifactError::Invalid(e.to_string()))
    })
    .map_err(|e| failed(e.to_string()))?;
    Ok(reuse)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ResultIndex {
    version: u32,
    route: super::Route,
    generation: String,
    prefix: String,
    coverage: Vec<SampleRange>,
    limitations: Vec<String>,
    rows: usize,
    speakers: usize,
}
pub(super) fn persist_result(
    store: &SharedStore,
    id: &str,
    result: &RouteResult,
) -> Result<(), StageError> {
    let generation = super::stages::generation(store, id)?;
    let sequence = lock(store)
        .restart_recognition_epoch(id, &generation, 0, 0)?
        .scope
        .epoch;
    let prefix = format!("result/{sequence:020}/");
    for (i, row) in result.rows.iter().enumerate() {
        let block = OutputBlock {
            key: format!("{prefix}row/{i:020}"),
            scope: row.id.scope.clone(),
            revision: row.revision,
            coverage: Coverage {
                received: Some(row.range),
                computed: Some(row.range),
                provisional: None,
                committed: Some(row.range),
            },
            rows: vec![row.clone()],
            speakers: vec![],
        };
        lock(store).commit_recognition(id, &block)?;
    }
    for (i, turn) in result.speakers.iter().enumerate() {
        let block = OutputBlock {
            key: format!("{prefix}speaker/{i:020}"),
            scope: turn.speaker.scope.clone(),
            revision: 0,
            coverage: Coverage {
                received: Some(turn.range),
                computed: Some(turn.range),
                provisional: None,
                committed: Some(turn.range),
            },
            rows: vec![],
            speakers: vec![turn.clone()],
        };
        lock(store).commit_recognition(id, &block)?;
    }
    checkpoint::put(
        store,
        id,
        Stage::Transcribe,
        super::RESULT_KEY,
        &ResultIndex {
            version: 1,
            route: result.route,
            generation,
            prefix,
            coverage: result.coverage.clone(),
            limitations: result.coverage_limitations.clone(),
            rows: result.rows.len(),
            speakers: result.speakers.len(),
        },
    )
}
pub(super) fn load_result(
    store: &SharedStore,
    id: &str,
) -> Result<Option<RouteResult>, StageError> {
    let value: Option<serde_json::Value> =
        checkpoint::get(store, id, Stage::Transcribe, super::RESULT_KEY)?;
    let Some(value) = value else {
        return Ok(None);
    };
    // Old aggregate checkpoints have no verified execution identity.
    if value.get("version").is_none() {
        return Ok(None);
    }
    let index: ResultIndex = serde_json::from_value(value)?;
    if index.version != 1 {
        return Err(failed("unsupported durable result version"));
    }
    let mut result = RouteResult {
        route: index.route,
        coverage: index.coverage,
        rows: vec![],
        speakers: vec![],
        coverage_limitations: index.limitations,
    };
    let mut after = String::new();
    loop {
        let page = lock(store).recognition_page(id, &index.generation, &index.prefix, &after)?;
        if page.is_empty() {
            break;
        }
        for block in page {
            after = block.key;
            result.rows.extend(block.rows);
            result.speakers.extend(block.speakers);
        }
    }
    if result.rows.len() != index.rows || result.speakers.len() != index.speakers {
        return Err(failed("durable result output is incomplete"));
    }
    super::validate_result(&result, &result.coverage)?;
    Ok(Some(result))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SpeakerIndex {
    version: u32,
    generation: String,
    prefix: String,
    count: usize,
}
pub fn persist_speakers(
    store: &SharedStore,
    id: &str,
    turns: &[SpeakerTurn],
) -> Result<(), StageError> {
    let generation = super::stages::generation(store, id)?;
    let sequence = lock(store)
        .restart_recognition_epoch(id, &generation, 0, 0)?
        .scope
        .epoch;
    let prefix = format!("speakers/{sequence:020}/");
    for (i, turn) in turns.iter().enumerate() {
        if turn.speaker.scope.generation != generation {
            return Err(failed("speaker analysis generation mismatch"));
        }
        let block = OutputBlock {
            key: format!("{prefix}{i:020}"),
            scope: turn.speaker.scope.clone(),
            revision: 0,
            coverage: Coverage {
                received: Some(turn.range),
                computed: Some(turn.range),
                provisional: None,
                committed: Some(turn.range),
            },
            rows: vec![],
            speakers: vec![turn.clone()],
        };
        lock(store).commit_recognition(id, &block)?;
    }
    checkpoint::put(
        store,
        id,
        Stage::Diarize,
        super::SPEAKERS_KEY,
        &SpeakerIndex {
            version: 1,
            generation,
            prefix,
            count: turns.len(),
        },
    )
}
pub(super) fn load_speakers(
    store: &SharedStore,
    id: &str,
) -> Result<Option<Vec<SpeakerTurn>>, StageError> {
    let value: Option<serde_json::Value> =
        checkpoint::get(store, id, Stage::Diarize, super::SPEAKERS_KEY)?;
    let Some(value) = value else {
        return Ok(None);
    };
    if value.get("version").is_none() {
        return Ok(None);
    }
    let index: SpeakerIndex = serde_json::from_value(value)?;
    if index.version != 1 {
        return Err(failed("unsupported speaker result version"));
    }
    let mut turns = vec![];
    let mut after = String::new();
    loop {
        let page = lock(store).recognition_page(id, &index.generation, &index.prefix, &after)?;
        if page.is_empty() {
            break;
        }
        for block in page {
            after = block.key;
            turns.extend(block.speakers);
        }
    }
    if turns.len() != index.count {
        return Err(failed("durable speaker output is incomplete"));
    }
    Ok(Some(turns))
}
