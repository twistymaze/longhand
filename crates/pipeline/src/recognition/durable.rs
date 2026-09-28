//! Exact compatibility checks precede checkpoint reuse and stage shortcuts.
use super::{RouteResult, failed};
use crate::{SharedStore, StageError, checkpoint, lock};
use models::recognition::{ExecutionSpec, SpeakerSource};
use session_core::{Segment, Stage, recognition::*};
use std::path::Path;

pub const IDENTITY_KEY: &str = "recognition-identity-v1";
pub const LIVE_IDENTITY_KEY: &str = "recognition-liveidentity-v1";

/// An attempt never persists its cached identity. File metadata detects ordinary
/// changes between stages; publication always checks the exact file contents.
#[derive(Default)]
pub(crate) struct Attempt {
    verified: Option<(String, Fingerprints)>,
}

impl Attempt {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        store: &SharedStore,
        id: &str,
        dir: &Path,
        segments: &[Segment],
        spec: &ExecutionSpec,
        publishing: bool,
    ) -> Result<(), StageError> {
        let result = self.check(store, id, dir, segments, spec, publishing);
        if result.is_err() {
            self.verified = None;
            // Retry must reconsider dependent work even when Assemble restores
            // the previous processing bytes from the unchanged archive.
            lock(store).delete_checkpoint(id, Stage::Transcribe, IDENTITY_KEY)?;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn check(
        &mut self,
        store: &SharedStore,
        id: &str,
        dir: &Path,
        segments: &[Segment],
        spec: &ExecutionSpec,
        publishing: bool,
    ) -> Result<(), StageError> {
        let stamp = || -> Result<String, StageError> {
            let inputs = crate::assemble::recognition_input_stamp(store, id, dir, segments)?;
            let count = lock(store)
                .session(id)?
                .ok_or_else(|| failed("session is absent"))?
                .speaker_count;
            hash(&(inputs, spec, count))
        };
        let before = stamp()?;
        let previous = if let Some((_, value)) = &self.verified {
            Some(value.clone())
        } else {
            checkpoint::get::<Fingerprints>(store, id, Stage::Transcribe, IDENTITY_KEY)?
        };
        let next = match &self.verified {
            Some((key, value)) if !publishing && *key == before => value.clone(),
            _ => fingerprints(store, id, dir, segments, spec)?,
        };
        if stamp()? != before {
            return Err(failed(
                "Recognition input changed during verification. Retry processing.",
            ));
        }
        let changed = (self.verified.is_some() || publishing) && previous.as_ref() != Some(&next);
        prepare_fingerprints(store, id, dir, segments, spec, next.clone())?;
        if changed {
            return Err(failed(
                "Recognition input changed during processing. Retry before saving.",
            ));
        }
        self.verified = Some((before, next));
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    static AUDIO_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
    #[cfg(test)]
    AUDIO_SCANS.with(|count| count.set(count.get() + 1));
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
    let covered_end = segment.offset_ms * 16
        + crate::resample::output_sample_count(proof.samples, u64::from(proof.rate), 16000);
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
    prepare_fingerprints(store, id, dir, segments, spec, next)
}

fn prepare_fingerprints(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
    spec: &ExecutionSpec,
    next: Fingerprints,
) -> Result<Fingerprints, StageError> {
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

/// Keep durable records bounded and publish indexes only after all batches commit.
pub(super) fn commit_blocks(
    store: &SharedStore,
    id: &str,
    blocks: impl IntoIterator<Item = OutputBlock>,
) -> Result<(), StageError> {
    let mut batch = Vec::with_capacity(MAX_COMMIT_BLOCKS);
    let mut bytes = 0;
    for block in blocks {
        let size = serde_json::to_vec(&block)?.len();
        if !batch.is_empty()
            && (batch.len() == MAX_COMMIT_BLOCKS || bytes + size > MAX_COMMIT_BYTES)
        {
            lock(store).commit_recognition_batch(id, &batch)?;
            batch.clear();
            bytes = 0;
        }
        bytes += size;
        batch.push(block);
    }
    if !batch.is_empty() {
        lock(store).commit_recognition_batch(id, &batch)?;
    }
    Ok(())
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
    let rows = result.rows.iter().enumerate().map(|(i, row)| OutputBlock {
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
    });
    let speakers = result
        .speakers
        .iter()
        .enumerate()
        .map(|(i, turn)| OutputBlock {
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
        });
    commit_blocks(store, id, rows.chain(speakers))?;
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
    if turns
        .iter()
        .any(|turn| turn.speaker.scope.generation != generation)
    {
        return Err(failed("speaker analysis generation mismatch"));
    }
    let blocks = turns.iter().enumerate().map(|(i, turn)| OutputBlock {
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
    });
    commit_blocks(store, id, blocks)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn audio_fixture() -> (tempfile::TempDir, SharedStore, Segment, ExecutionSpec) {
        let temp = tempfile::tempdir().unwrap();
        let mut database = session_core::Store::open_in_memory().unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-27T10:00:00-05:00").unwrap();
        database.create_session("test", now, None).unwrap();
        let store = std::sync::Arc::new(std::sync::Mutex::new(database));
        let segment = Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 1000,
            wall_start: now,
            wall_end: now + chrono::Duration::seconds(1),
            audio_file: "audio/segment-01.opus".into(),
            channels: 1,
        };
        let archive = temp.path().join(&segment.audio_file);
        let processing = temp.path().join(crate::proc_file(&segment.audio_file));
        std::fs::create_dir_all(archive.parent().unwrap()).unwrap();
        std::fs::create_dir_all(processing.parent().unwrap()).unwrap();
        std::fs::write(&archive, vec![7_u8; 128 * 1024]).unwrap();
        std::fs::write(&processing, vec![9_u8; 128 * 1024]).unwrap();
        checkpoint::put(
            &store,
            "test",
            Stage::Assemble,
            "audio-provenance-00000000",
            &serde_json::json!({
                "archive": file_digest(&archive).unwrap(),
                "processing": file_digest(&processing).unwrap(),
                "echo": digest(b"null"),
            }),
        )
        .unwrap();
        let spec = models::recognition::resolve(&models::recognition::RecognitionConfig {
            speaker_source: SpeakerSource::Off,
            ..Default::default()
        })
        .unwrap();
        (temp, store, segment, spec)
    }

    #[test]
    fn recognition_attempt_scans_once_and_again_before_save() {
        let (temp, store, segment, spec) = audio_fixture();
        let mut attempt = Attempt::default();
        AUDIO_SCANS.with(|count| count.set(0));
        for stage in [
            Stage::Diarize,
            Stage::Transcribe,
            Stage::Join,
            Stage::Verify,
            Stage::Summarize,
            Stage::Save,
        ] {
            attempt
                .prepare(
                    &store,
                    "test",
                    temp.path(),
                    std::slice::from_ref(&segment),
                    &spec,
                    stage == Stage::Save,
                )
                .unwrap();
        }
        assert_eq!(AUDIO_SCANS.with(|count| count.get()), 2);
        Attempt::default()
            .prepare(&store, "test", temp.path(), &[segment], &spec, false)
            .unwrap();
        assert_eq!(
            AUDIO_SCANS.with(|count| count.get()),
            3,
            "new attempts check exact content again"
        );
    }

    #[test]
    fn recognition_attempt_checks_restored_timestamps_before_save_and_on_restart() {
        for publishing in [false, true] {
            let (temp, store, segment, spec) = audio_fixture();
            let mut attempt = Attempt::default();
            let segments = [segment];
            attempt
                .prepare(&store, "test", temp.path(), &segments, &spec, false)
                .unwrap();
            let path = temp.path().join(&segments[0].audio_file);
            let before = std::fs::metadata(&path).unwrap();
            std::fs::write(&path, vec![8_u8; before.len() as usize]).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(before.modified().unwrap()))
                .unwrap();
            if !publishing {
                attempt = Attempt::default();
            }
            assert!(
                attempt
                    .prepare(&store, "test", temp.path(), &segments, &spec, publishing)
                    .is_err()
            );
            assert!(
                checkpoint::get::<Fingerprints>(&store, "test", Stage::Transcribe, IDENTITY_KEY)
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn recognition_attempt_rejects_changed_config_and_count_before_saving() {
        for change_count in [false, true] {
            let (temp, store, segment, mut spec) = audio_fixture();
            let mut attempt = Attempt::default();
            let segments = [segment];
            attempt
                .prepare(&store, "test", temp.path(), &segments, &spec, false)
                .unwrap();
            checkpoint::put(&store, "test", Stage::Summarize, "old-summary", &"old").unwrap();
            if change_count {
                lock(&store)
                    .set_speaker_count("test", Some(session_core::SpeakerCount::Exact(2)))
                    .unwrap();
            } else {
                spec.config.language = Some("fr".into());
            }
            assert!(
                attempt
                    .prepare(&store, "test", temp.path(), &segments, &spec, true)
                    .is_err()
            );
            assert!(
                checkpoint::get::<String>(&store, "test", Stage::Summarize, "old-summary")
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn recognition_attempt_detects_processing_and_preprocessing_changes_between_stages() {
        for echo_change in [false, true] {
            let (temp, store, segment, spec) = audio_fixture();
            let mut attempt = Attempt::default();
            let segments = [segment];
            attempt
                .prepare(&store, "test", temp.path(), &segments, &spec, false)
                .unwrap();
            if echo_change {
                checkpoint::put(
                    &store,
                    "test",
                    Stage::Assemble,
                    &checkpoint::echo_key(0),
                    &serde_json::json!({"changed": true}),
                )
                .unwrap();
            } else {
                std::fs::write(
                    temp.path().join(crate::proc_file(&segments[0].audio_file)),
                    b"changed",
                )
                .unwrap();
            }
            assert!(
                attempt
                    .prepare(&store, "test", temp.path(), &segments, &spec, false)
                    .is_err()
            );
        }
    }

    #[test]
    fn partial_speaker_batches_preserve_previous_index_and_retry_completely() {
        let mut database = session_core::Store::open_in_memory().unwrap();
        database
            .create_session(
                "test",
                chrono::DateTime::parse_from_rfc3339("2026-09-27T10:00:00-05:00").unwrap(),
                None,
            )
            .unwrap();
        let store = std::sync::Arc::new(std::sync::Mutex::new(database));
        let generation = super::super::stages::generation(&store, "test").unwrap();
        let turn = SpeakerTurn {
            range: SampleRange {
                start: 0,
                end: 16000,
            },
            timing: TimingPrecision::NativeSegment,
            speaker: SpeakerKey {
                scope: SpeakerScope {
                    generation,
                    epoch: 1,
                },
                input_start_sample: 0,
                local_id: 1,
            },
            finality: Finality::Final,
        };
        persist_speakers(&store, "test", std::slice::from_ref(&turn)).unwrap();
        let mut replacement = vec![turn.clone(); MAX_COMMIT_BLOCKS * 2 + 1];
        replacement.last_mut().unwrap().finality = Finality::Provisional;
        assert!(persist_speakers(&store, "test", &replacement).is_err());
        assert_eq!(load_speakers(&store, "test").unwrap().unwrap(), vec![turn]);
        replacement.last_mut().unwrap().finality = Finality::Final;
        persist_speakers(&store, "test", &replacement).unwrap();
        assert_eq!(load_speakers(&store, "test").unwrap().unwrap(), replacement);
    }
}
