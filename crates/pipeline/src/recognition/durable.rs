//! Exact compatibility checks precede checkpoint reuse and stage shortcuts.
use super::{RouteResult, failed};
use crate::{SharedStore, StageError, checkpoint, lock};
use models::recognition::ExecutionSpec;
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
    Ok(Fingerprints {
        execution: hash(spec)?,
        // The literal `false` keeps the fingerprint of every saved note. Do not
        // remove it.
        asr: hash(&(
            spec.artifacts.first(),
            runtime,
            &spec.config.language,
            false,
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
    configured_fingerprints(
        store,
        id,
        crate::assemble::recognition_audio_identity(store, id, dir, segments)?,
        spec,
    )
}

pub(super) fn configured_fingerprints(
    store: &SharedStore,
    id: &str,
    audio: String,
    spec: &ExecutionSpec,
) -> Result<Fingerprints, StageError> {
    let mut value = spec_fingerprints(audio, spec)?;
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
    for segment in segments {
        let source: Option<crate::live::archive::SourceProof> = checkpoint::get(
            store,
            id,
            Stage::Transcribe,
            &crate::live::archive::key(segment.index),
        )?;
        has_source |= source.is_some();
        if source.is_some() && live_source_matches(store, id, dir, segment, spec)? {
            continue;
        }
        // Unverified live output never blocks processing. Recognition of the
        // recorded audio replaces it, so a lost proof cannot stop a note.
        if source.is_none() {
            let rows = crate::live::segment_rows(store, id, segment.index)?;
            let features = crate::live::features::key(segment.index);
            let has_features =
                checkpoint::get::<serde_json::Value>(store, id, Stage::Diarize, &features)?
                    .is_some();
            if rows.is_empty() && !has_features {
                continue;
            }
        }
        {
            let mut reasons: Vec<String> =
                checkpoint::get(store, id, Stage::Transcribe, "live-invalidation-reasons")?
                    .unwrap_or_default();
            let reason = if source.is_none() {
                format!(
                    "Live text and speaker output for segment {} had no saved source proof. Recognition used the recorded audio.",
                    segment.index + 1
                )
            } else if segment.channels == 2 {
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
        // Each live row that remains has a verified source proof.
        checkpoint::put(store, id, Stage::Transcribe, LIVE_IDENTITY_KEY, next)?;
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
    let prefix = super::saved_prefix::read(store, id, dir, segments, spec)?;
    let active = prefix
        .as_ref()
        .map_or(segments, |prefix| &segments[prefix.count..]);
    bind_live(store, id, dir, active, spec, &next)?;
    let old: Option<Fingerprints> = checkpoint::get(store, id, Stage::Transcribe, IDENTITY_KEY)?;
    if checkpoint::is_complete(store, id, Stage::Join, segments.len())? {
        let reference: Option<ArtifactReference> =
            checkpoint::get(store, id, Stage::Join, "recognition-artifact-reference-v1")?;
        if let Some(reference) = reference {
            let header = read_artifact(&dir.join(ARTIFACT_FILE), &reference, |_| Ok(()))
                .map_err(|e| failed(e.to_string()))?;
            if file_digest(&dir.join(crate::join::TRANSCRIPT_FILE))? != header.transcript {
                return Err(failed(
                    "The transcript changed outside Longhand. Saved work is preserved.",
                ));
            }
        }
    }
    let live: Vec<_> = crate::live::rows(store, id)?
        .into_iter()
        .filter(|row| active.iter().any(|segment| segment.index == row.segment))
        .collect();
    let live_identity: Option<Fingerprints> =
        checkpoint::get(store, id, Stage::Transcribe, LIVE_IDENTITY_KEY)?;
    let live_matches = live_identity.as_ref().is_some_and(|f| {
        f.asr == next.asr && f.audio == next.audio && f.preprocessing == next.preprocessing
    });
    if !live.is_empty() && !live_matches {
        return Err(failed("The live text does not match the recorded audio."));
    }
    if old.as_ref() == Some(&next) {
        if let Some(prefix) = &prefix {
            super::saved_prefix::retain(store, id, prefix)?;
        }
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
    let keep_live_epoch = |key: &str| {
        prefix.is_some()
            && live_matches
            && key
                .strip_prefix("recognition-epoch-")
                .and_then(|s| s.parse::<u64>().ok())
                .is_some_and(|start| start >= active[0].offset_ms * 16)
    };
    let mut guard = lock(store);
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
                if !key.starts_with("live-feature-") && !keep_live_epoch(&key) {
                    guard.delete_checkpoint(id, stage, &key)?;
                }
            }
        } else {
            guard.clear_checkpoints(id, stage)?;
        }
    }
    let generation: Option<String> = guard
        .checkpoint_value(id, Stage::Transcribe, GENERATION_KEY)?
        .map(|value| serde_json::from_str(&value))
        .transpose()?;
    for (key, _) in guard.checkpoints(id, Stage::Transcribe)? {
        // A source proof describes recorded audio, not a result. A later
        // Resume must check the proofs of earlier segments again.
        let keep_proof = key.starts_with("live-source-") || key.starts_with("live-snapshot-");
        let keep_live =
            keep_proof || live_matches && (key.contains("-live-") || key == LIVE_IDENTITY_KEY);
        let keep_asr = timed && (key.starts_with("seg-") || key == GENERATION_KEY);
        let keep_prefix = prefix.is_some()
            && (key == GENERATION_KEY || key == super::saved_prefix::KEY || keep_live_epoch(&key));
        if !keep_live && !keep_asr && !keep_prefix && key != "live-invalidation-reasons" {
            guard.delete_checkpoint(id, Stage::Transcribe, &key)?;
            // The next attempt makes a new generation. Nothing reads the
            // rows of the replaced one.
            if key == GENERATION_KEY
                && let Some(generation) = &generation
            {
                guard.delete_recognition_generation(id, generation)?;
            }
        }
    }
    drop(guard);
    checkpoint::put(store, id, Stage::Transcribe, IDENTITY_KEY, &next)?;
    if let Some(prefix) = &prefix {
        super::saved_prefix::retain(store, id, prefix)?;
    }
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

/// The native operation can produce one or both output modalities, even when
/// a completed interval contains no words or speaker turns.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct EpochOutput {
    pub text: bool,
    pub speakers: bool,
}

#[derive(Default)]
struct CatchupCoverage {
    text: Vec<SampleRange>,
    speakers: Vec<SampleRange>,
}

/// Visit one authoritative copy of completed history and the remaining live output.
/// The caller holds the store lock through selection and publication.
pub fn visit_catchup_blocks(
    store: &session_core::Store,
    id: &str,
    generation: &str,
    mut visitor: impl FnMut(OutputBlock) -> session_core::jobs::Result<()>,
) -> session_core::jobs::Result<()> {
    let index = store
        .checkpoint_value(id, Stage::Transcribe, super::RESULT_KEY)?
        .map(|value| serde_json::from_str::<serde_json::Value>(&value))
        .transpose()?
        .filter(|value| value.get("version").is_some())
        .map(serde_json::from_value::<ResultIndex>)
        .transpose()?
        .filter(|index| index.version == 1 && index.generation == generation);
    let result_coverage = index
        .as_ref()
        .map(|index| index.coverage.clone())
        .unwrap_or_default();
    #[derive(serde::Deserialize)]
    struct CompletedEpoch {
        scope: SpeakerScope,
        range: SampleRange,
        #[serde(default)]
        output: Option<EpochOutput>,
    }
    let mut completed = std::collections::BTreeMap::new();
    for stage in [Stage::Transcribe, Stage::Diarize] {
        for (key, value) in store.checkpoints(id, stage)? {
            if key.starts_with("recognition-epoch-") {
                let record: CompletedEpoch = serde_json::from_str(&value)?;
                if record.scope.generation == generation {
                    let output = record.output.unwrap_or(EpochOutput {
                        text: stage == Stage::Transcribe,
                        speakers: stage == Stage::Diarize,
                    });
                    completed.insert(record.scope.epoch, (record.range, output));
                }
            }
        }
    }
    let coverage_after = |after: Option<u64>| {
        let mut coverage = CatchupCoverage {
            text: result_coverage.clone(),
            speakers: result_coverage.clone(),
        };
        for (&epoch, &(range, output)) in &completed {
            if after.is_some_and(|after| epoch <= after) {
                continue;
            }
            if output.text {
                coverage.text.push(range);
            }
            if output.speakers {
                coverage.speakers.push(range);
            }
        }
        coverage.text = merged_ranges(coverage.text);
        coverage.speakers = merged_ranges(coverage.speakers);
        coverage
    };
    let covered = coverage_after(None);
    let completed_coverage: std::collections::BTreeMap<_, _> = completed
        .keys()
        .map(|&epoch| (epoch, coverage_after(Some(epoch))))
        .collect();
    // A resumed native stream can retain its original blocks beside completed
    // epoch copies. Failed replay attempts can also remain in this generation.
    let mut live_ranges = std::collections::BTreeMap::<u64, Vec<SampleRange>>::new();
    let mut live_inputs = std::collections::BTreeMap::<u64, SampleRange>::new();
    store.visit_recognition(id, generation, |block| {
        if block.key.starts_with(LIVE_NEMO_PREFIX) && !completed.contains_key(&block.scope.epoch) {
            if let Some(range) = block.coverage.received {
                live_inputs
                    .entry(block.scope.epoch)
                    .and_modify(|input| {
                        input.start = input.start.min(range.start);
                        input.end = input.end.max(range.end);
                    })
                    .or_insert(range);
            }
            for row in &block.rows {
                if row.text_finality == Finality::Final {
                    live_ranges
                        .entry(block.scope.epoch)
                        .or_default()
                        .push(row.range);
                }
            }
        }
        Ok(())
    })?;
    for ranges in live_ranges.values_mut() {
        *ranges = merged_ranges(std::mem::take(ranges));
    }
    store.visit_recognition(id, generation, |mut block| {
        if block.key.starts_with("result/") {
            return if index
                .as_ref()
                .is_some_and(|index| block.key.starts_with(&index.prefix))
            {
                visitor(block)
            } else {
                Ok(())
            };
        }
        let native_final = block.key.starts_with(NATIVE_FINAL_PREFIX);
        let native_live = block.key.starts_with(LIVE_NEMO_PREFIX);
        if native_final && !completed.contains_key(&block.scope.epoch) {
            return Ok(());
        }
        if native_live && completed.contains_key(&block.scope.epoch) {
            return Ok(());
        }
        let covered = if native_final {
            &completed_coverage[&block.scope.epoch]
        } else {
            &covered
        };
        if native_live
            && live_inputs.get(&block.scope.epoch).is_some_and(|range| {
                range_is_covered(&covered.text, *range)
                    && range_is_covered(&covered.speakers, *range)
            })
        {
            return Ok(());
        }
        let had_output = !block.rows.is_empty() || !block.speakers.is_empty();
        // A word that crosses a boundary remains whole, with its native timing.
        // There is no reliable way to split its text at an arbitrary sample.
        block.rows.retain(|row| {
            !(range_is_covered(&covered.text, row.range)
                || (native_live
                    && live_ranges
                        .range((
                            std::ops::Bound::Excluded(block.scope.epoch),
                            std::ops::Bound::Unbounded,
                        ))
                        .any(|(_, ranges)| range_is_covered(ranges, row.range)))
                || (native_live
                    && row.text_finality == Finality::Provisional
                    && live_ranges
                        .range(..block.scope.epoch)
                        .any(|(_, ranges)| range_is_covered(ranges, row.range))))
        });
        block.speakers = block
            .speakers
            .into_iter()
            .flat_map(|turn| {
                uncovered_ranges(turn.range, &covered.speakers)
                    .into_iter()
                    .map(move |range| {
                        let mut part = turn.clone();
                        if part.range != range {
                            part.timing = TimingPrecision::NativeSegmentClipped;
                        }
                        part.range = range;
                        part
                    })
            })
            .collect();
        if had_output && block.rows.is_empty() && block.speakers.is_empty() {
            return Ok(());
        }
        // Empty markers are part of the snapshot protocol. Keep them even when
        // all rows were superseded; they can complete an empty speaker update.
        visitor(block)
    })
}

fn merged_ranges(mut ranges: Vec<SampleRange>) -> Vec<SampleRange> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<SampleRange> = vec![];
    for range in ranges {
        if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn range_is_covered(ranges: &[SampleRange], range: SampleRange) -> bool {
    let before = ranges.partition_point(|covered| covered.start <= range.start);
    before > 0
        && ranges[before - 1].end >= range.end
        && (range.start < range.end || range.start < ranges[before - 1].end)
}

fn uncovered_ranges(range: SampleRange, covered: &[SampleRange]) -> Vec<SampleRange> {
    let mut start = range.start;
    let mut remaining = vec![];
    for cut in covered {
        if cut.end <= start || cut.start >= range.end {
            continue;
        }
        if cut.start > start {
            remaining.push(SampleRange {
                start,
                end: cut.start,
            });
        }
        start = start.max(cut.end);
    }
    if start < range.end {
        remaining.push(SampleRange {
            start,
            end: range.end,
        });
    }
    remaining
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
    put_index(
        store,
        id,
        Stage::Transcribe,
        super::RESULT_KEY,
        &generation,
        &prefix,
        &ResultIndex {
            version: 1,
            route: result.route,
            generation: generation.clone(),
            prefix: prefix.clone(),
            coverage: result.coverage.clone(),
            limitations: result.coverage_limitations.clone(),
            rows: result.rows.len(),
            speakers: result.speakers.len(),
        },
    )
}
/// Put a result or speaker index after its blocks are written. The put
/// also deletes the blocks of the index that it replaces in the same
/// generation, in one transaction. The `previous_*` copies of a
/// Transcribe Again run are not read here, so their blocks stay.
fn put_index(
    store: &SharedStore,
    id: &str,
    stage: Stage,
    key: &str,
    generation: &str,
    prefix: &str,
    index: &impl serde::Serialize,
) -> Result<(), StageError> {
    let mut guard = lock(store);
    let old: Option<serde_json::Value> = guard
        .checkpoint_value(id, stage, key)?
        .map(|value| serde_json::from_str(&value))
        .transpose()?;
    let replaced = old.as_ref().and_then(|old| {
        let same = old.get("generation")?.as_str()? == generation;
        let old = old.get("prefix")?.as_str()?;
        (same && old != prefix).then_some(old)
    });
    guard.replace_recognition_index(
        id,
        stage,
        key,
        &serde_json::to_string(index)?,
        replaced.map(|old| (generation, old)),
    )?;
    Ok(())
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
    put_index(
        store,
        id,
        Stage::Diarize,
        super::SPEAKERS_KEY,
        &generation,
        &prefix,
        &SpeakerIndex {
            version: 1,
            generation: generation.clone(),
            prefix: prefix.clone(),
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
    use models::recognition::SpeakerSource;

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
            ends_in_pause: false,
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

    /// Saved notes compare this value. A change stops text reuse for every note.
    #[test]
    fn asr_fingerprint_of_a_fixed_spec_is_stable() {
        use models::recognition::*;
        let spec = ExecutionSpec {
            schema_version: 1,
            catalog_revision: "catalog".into(),
            config: RecognitionConfig {
                asr_model: "whisper-large-v3-turbo-q5km".into(),
                speaker_source: SpeakerSource::Pyannote,
                processing_mode: ProcessingMode::AfterStop,
                speaker_processing_mode: ProcessingMode::AfterStop,
                speaker_count: None,
                language: Some("en".into()),
            },
            artifacts: vec![ArtifactSpec {
                id: "whisper-large-v3-turbo-q5km".into(),
                filename: "whisper.gguf".into(),
                sha256: "0".repeat(64),
                revision: "revision".into(),
                url: "https://example.invalid/whisper.gguf".into(),
                size_bytes: 1,
                format: "transcribe-gguf".into(),
            }],
            runtimes: vec![RuntimeSpec {
                name: "transcribe.cpp".into(),
                revision: "runtime-revision".into(),
            }],
            capabilities: Capabilities {
                live_transcription: false,
                live_speaker_analysis: false,
                live_attributed_text: false,
                maximum_speakers: None,
                locally_verified: false,
            },
            enabled_live_transcription: false,
            enabled_live_speaker_analysis: false,
            preprocessing_revision: "mono-16khz-v1".into(),
        };
        let asr = spec_fingerprints("audio".into(), &spec).unwrap().asr;
        assert_eq!(
            asr,
            "85f58a898b07e6d23265ebeaf49a846cbc271a2860464941aa857cb357aad195"
        );
    }

    #[test]
    fn catchup_selects_saved_result_once_and_retains_resumed_words() {
        let (_temp, store, _, _) = audio_fixture();
        let generation = super::super::stages::generation(&store, "test").unwrap();
        let make = |epoch, start, end, text: &str| {
            let scope = SpeakerScope {
                generation: generation.clone(),
                epoch,
            };
            RecognitionSegment {
                id: RecognitionId {
                    scope,
                    input_start_sample: start,
                    ordinal: 0,
                },
                range: SampleRange { start, end },
                timing: TimingPrecision::NativeSegment,
                text: text.into(),
                speaker: None,
                revision: 1,
                text_finality: Finality::Final,
                speaker_finality: Finality::Final,
            }
        };
        let put_native = |row: RecognitionSegment| {
            let block = OutputBlock {
                key: format!("nemo/{:020}/word/0", row.id.scope.epoch),
                scope: row.id.scope.clone(),
                revision: 1,
                coverage: Coverage {
                    received: Some(row.range),
                    computed: Some(row.range),
                    provisional: None,
                    committed: Some(row.range),
                },
                rows: vec![row],
                speakers: vec![],
            };
            lock(&store).commit_recognition("test", &block).unwrap();
        };
        put_native(make(1, 0, 16000, "obsolete"));
        let mut result = RouteResult {
            route: super::super::Route::Off,
            rows: vec![make(2, 0, 16000, "old result")],
            speakers: vec![],
            coverage: vec![SampleRange {
                start: 0,
                end: 16000,
            }],
            coverage_limitations: vec![],
        };
        persist_result(&store, "test", &result).unwrap();
        result.rows = vec![make(3, 0, 16000, "saved")];
        put_native(result.rows[0].clone());
        persist_result(&store, "test", &result).unwrap();
        put_native(make(4, 16000, 32000, "resumed"));
        let mut rows = vec![];
        visit_catchup_blocks(&lock(&store), "test", &generation, |block| {
            rows.extend(block.rows);
            Ok(())
        })
        .unwrap();
        rows.sort_by_key(|row| row.range.start);
        assert_eq!(
            rows.iter().map(|row| row.text.as_str()).collect::<Vec<_>>(),
            vec!["saved", "resumed"]
        );
    }

    #[test]
    fn catchup_preserves_uncovered_words_and_completed_epoch_speakers() {
        let (_temp, store, _, _) = audio_fixture();
        let generation = super::super::stages::generation(&store, "test").unwrap();
        let scope = |epoch| SpeakerScope {
            generation: generation.clone(),
            epoch,
        };
        let put = |key: &str, epoch, start, end, text: &str, speaker_only| {
            let range = SampleRange { start, end };
            let speaker = SpeakerKey {
                scope: scope(epoch),
                input_start_sample: 0,
                local_id: 1,
            };
            lock(&store)
                .commit_recognition(
                    "test",
                    &OutputBlock {
                        key: key.into(),
                        scope: scope(epoch),
                        revision: 1,
                        coverage: Coverage {
                            received: Some(range),
                            computed: Some(range),
                            provisional: None,
                            committed: Some(range),
                        },
                        rows: if speaker_only {
                            vec![]
                        } else {
                            vec![RecognitionSegment {
                                id: RecognitionId {
                                    scope: scope(epoch),
                                    input_start_sample: 0,
                                    ordinal: start,
                                },
                                range,
                                timing: TimingPrecision::NativeSegment,
                                text: text.into(),
                                speaker: None,
                                revision: 1,
                                text_finality: Finality::Final,
                                speaker_finality: Finality::Final,
                            }]
                        },
                        speakers: if speaker_only {
                            vec![SpeakerTurn {
                                range,
                                timing: TimingPrecision::NativeSegment,
                                speaker,
                                finality: Finality::Final,
                            }]
                        } else {
                            vec![]
                        },
                    },
                )
                .unwrap();
        };
        // Epoch 2 replaces only the first second of epoch 1. Its completed
        // rows have both final copies and original live blocks.
        put("nemo/1/word/0", 1, 0, 16000, "old", false);
        put("nemo/1/word/1", 1, 16000, 32000, "tail", false);
        put("nemo/1/word/2", 1, 15000, 17000, "boundary", false);
        put("nemo/1/turn/0", 1, 0, 32000, "", true);
        put("nemo/2/word/0", 2, 0, 16000, "replacement", false);
        put("nemo/2/turn/0", 2, 0, 16000, "", true);
        put("native-final/2/row/0", 2, 0, 16000, "replacement", false);
        put("native-final/2/speaker/0", 2, 0, 16000, "", true);
        checkpoint::put(
            &store,
            "test",
            Stage::Transcribe,
            "recognition-epoch-00000000000000000000",
            &serde_json::json!({ "scope": scope(2), "range": { "start": 0, "end": 16000 },
                "output": { "text": true, "speakers": true } }),
        )
        .unwrap();
        let mut blocks = vec![];
        visit_catchup_blocks(&lock(&store), "test", &generation, |block| {
            blocks.push(block);
            Ok(())
        })
        .unwrap();
        let mut words: Vec<_> = blocks.iter().flat_map(|block| block.rows.iter()).collect();
        words.sort_by_key(|row| row.range.start);
        assert_eq!(
            words
                .iter()
                .map(|row| row.text.as_str())
                .collect::<Vec<_>>(),
            vec!["replacement", "boundary", "tail"]
        );
        assert_eq!(
            words[1].range,
            SampleRange {
                start: 15000,
                end: 17000
            }
        );
        assert_eq!(words[1].timing, TimingPrecision::NativeSegment);
        let turns: Vec<_> = blocks
            .iter()
            .flat_map(|block| block.speakers.iter())
            .collect();
        assert_eq!(turns.len(), 2);
        assert!(turns.iter().any(|turn| turn.range
            == SampleRange {
                start: 16000,
                end: 32000
            }
            && turn.timing == TimingPrecision::NativeSegmentClipped));
        // Older combined epochs have no modality metadata. Their persisted
        // speaker copy remains visible without its original live duplicate.
        checkpoint::put(
            &store,
            "test",
            Stage::Transcribe,
            "recognition-epoch-00000000000000000000",
            &serde_json::json!({
                "scope": scope(2), "range": { "start": 0, "end": 16000 },
                "rows": 1, "speakers": 1,
            }),
        )
        .unwrap();
        let mut restored = vec![];
        visit_catchup_blocks(&lock(&store), "test", &generation, |block| {
            restored.push(block);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            restored
                .iter()
                .filter(|block| block.scope.epoch == 2)
                .flat_map(|block| block.speakers.iter())
                .count(),
            1
        );
        assert!(
            restored
                .iter()
                .all(|block| !block.key.starts_with("nemo/2/"))
        );
    }

    #[test]
    fn catchup_replay_keeps_old_tail_until_new_words_commit() {
        let (_temp, store, _, _) = audio_fixture();
        let generation = super::super::stages::generation(&store, "test").unwrap();
        for (epoch, start, end, finality, text) in [
            (1, 0, 16000, Finality::Final, "old"),
            (1, 16000, 32000, Finality::Final, "tail"),
            (2, 0, 16000, Finality::Final, "replayed"),
            (2, 16000, 32000, Finality::Provisional, "interim"),
            (2, 32000, 48000, Finality::Provisional, "new interim"),
        ] {
            let scope = SpeakerScope {
                generation: generation.clone(),
                epoch,
            };
            let range = SampleRange { start, end };
            lock(&store)
                .commit_recognition(
                    "test",
                    &OutputBlock {
                        key: format!(
                            "nemo/{epoch}/{}/{start}",
                            if finality == Finality::Provisional {
                                "interim"
                            } else {
                                "word"
                            }
                        ),
                        scope: scope.clone(),
                        revision: 1,
                        coverage: Coverage {
                            received: Some(range),
                            computed: Some(range),
                            provisional: Some(range),
                            committed: None,
                        },
                        speakers: vec![],
                        rows: vec![RecognitionSegment {
                            id: RecognitionId {
                                scope,
                                input_start_sample: 0,
                                ordinal: start,
                            },
                            range,
                            timing: TimingPrecision::NativeSegment,
                            text: text.into(),
                            speaker: None,
                            revision: 1,
                            text_finality: finality,
                            speaker_finality: Finality::Provisional,
                        }],
                    },
                )
                .unwrap();
        }
        let mut words = vec![];
        visit_catchup_blocks(&lock(&store), "test", &generation, |block| {
            words.extend(block.rows.into_iter().map(|row| row.text));
            Ok(())
        })
        .unwrap();
        assert_eq!(words, vec!["tail", "new interim", "replayed"]);
    }

    #[test]
    fn catchup_completed_empty_epoch_replaces_only_its_output_modality() {
        for speaker_only in [false, true] {
            let (_temp, store, _, _) = audio_fixture();
            let generation = super::super::stages::generation(&store, "test").unwrap();
            let scope = SpeakerScope {
                generation: generation.clone(),
                epoch: 1,
            };
            let range = SampleRange {
                start: 0,
                end: 16000,
            };
            lock(&store)
                .commit_recognition(
                    "test",
                    &OutputBlock {
                        key: "nemo/1/word/0".into(),
                        scope: scope.clone(),
                        revision: 1,
                        coverage: Coverage {
                            received: Some(range),
                            computed: Some(range),
                            provisional: Some(range),
                            committed: None,
                        },
                        rows: vec![RecognitionSegment {
                            id: RecognitionId {
                                scope: scope.clone(),
                                input_start_sample: 0,
                                ordinal: 0,
                            },
                            range,
                            timing: TimingPrecision::NativeSegment,
                            text: "retained text".into(),
                            speaker: None,
                            revision: 1,
                            text_finality: Finality::Final,
                            speaker_finality: Finality::Provisional,
                        }],
                        speakers: vec![SpeakerTurn {
                            range,
                            timing: TimingPrecision::NativeSegment,
                            speaker: SpeakerKey {
                                scope,
                                input_start_sample: 0,
                                local_id: 1,
                            },
                            finality: Finality::Provisional,
                        }],
                    },
                )
                .unwrap();
            checkpoint::put(&store, "test", if speaker_only { Stage::Diarize } else { Stage::Transcribe },
                "recognition-epoch-00000000000000000000", &serde_json::json!({
                    "scope": {"generation": generation, "epoch": 2}, "range": range,
                    "rows": 0, "speakers": 0, "output": { "text": !speaker_only, "speakers": speaker_only },
                })).unwrap();
            let mut words = vec![];
            let mut turns = vec![];
            visit_catchup_blocks(&lock(&store), "test", &generation, |block| {
                words.extend(block.rows);
                turns.extend(block.speakers);
                Ok(())
            })
            .unwrap();
            assert_eq!(
                words.len(),
                usize::from(speaker_only),
                "text; speaker-only={speaker_only}"
            );
            assert_eq!(
                turns.len(),
                usize::from(!speaker_only),
                "speakers; speaker-only={speaker_only}"
            );
        }
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

    /// The distinct `{kind}/{sequence}/` prefixes that have blocks.
    fn stored_prefixes(store: &SharedStore, generation: &str, kind: &str) -> Vec<String> {
        let mut prefixes = std::collections::BTreeSet::new();
        lock(store)
            .visit_recognition("test", generation, |block| {
                if let Some(rest) = block.key.strip_prefix(&format!("{kind}/")) {
                    let sequence = rest.split('/').next().unwrap_or_default();
                    prefixes.insert(format!("{kind}/{sequence}/"));
                }
                Ok(())
            })
            .unwrap();
        prefixes.into_iter().collect()
    }

    #[test]
    fn a_new_result_or_speaker_index_removes_the_blocks_of_the_index_it_replaces() {
        let (_temp, store, _, _) = audio_fixture();
        let generation = super::super::stages::generation(&store, "test").unwrap();
        let scope = SpeakerScope {
            generation: generation.clone(),
            epoch: 1,
        };
        let range = SampleRange {
            start: 0,
            end: 16000,
        };
        let turn = SpeakerTurn {
            range,
            timing: TimingPrecision::NativeSegment,
            speaker: SpeakerKey {
                scope: scope.clone(),
                input_start_sample: 0,
                local_id: 1,
            },
            finality: Finality::Final,
        };
        let result = RouteResult {
            route: super::super::Route::Off,
            rows: vec![RecognitionSegment {
                id: RecognitionId {
                    scope,
                    input_start_sample: 0,
                    ordinal: 0,
                },
                range,
                timing: TimingPrecision::NativeSegment,
                text: "words".into(),
                speaker: None,
                revision: 1,
                text_finality: Finality::Final,
                speaker_finality: Finality::Final,
            }],
            speakers: vec![turn.clone()],
            coverage: vec![range],
            coverage_limitations: vec![],
        };
        for _ in 0..2 {
            persist_result(&store, "test", &result).unwrap();
            persist_speakers(&store, "test", std::slice::from_ref(&turn)).unwrap();
        }

        let results: ResultIndex =
            checkpoint::get(&store, "test", Stage::Transcribe, super::super::RESULT_KEY)
                .unwrap()
                .unwrap();
        assert_eq!(
            stored_prefixes(&store, &generation, "result"),
            [results.prefix]
        );
        let speakers: SpeakerIndex =
            checkpoint::get(&store, "test", Stage::Diarize, super::super::SPEAKERS_KEY)
                .unwrap()
                .unwrap();
        assert_eq!(
            stored_prefixes(&store, &generation, "speakers"),
            [speakers.prefix]
        );
        assert_eq!(
            load_result(&store, "test").unwrap().unwrap().rows,
            result.rows
        );
        assert_eq!(load_speakers(&store, "test").unwrap().unwrap(), vec![turn]);
    }

    #[test]
    fn a_retry_after_a_verify_failure_keeps_the_live_text_of_a_finished_epoch() {
        // The Retry deletes the epoch checkpoint. Catch-up then reads the
        // live blocks, so they stay until the note is saved.
        let (_temp, store, _, _) = audio_fixture();
        let generation = super::super::stages::generation(&store, "test").unwrap();
        let scope = SpeakerScope {
            generation: generation.clone(),
            epoch: 1,
        };
        let range = SampleRange {
            start: 0,
            end: 16000,
        };
        for key in [
            format!("nemo/{:020}/word/0", 1),
            format!("native-final/{:020}/row/0", 1),
        ] {
            let block = OutputBlock {
                key,
                scope: scope.clone(),
                revision: 1,
                coverage: Coverage {
                    received: Some(range),
                    computed: Some(range),
                    provisional: None,
                    committed: Some(range),
                },
                rows: vec![RecognitionSegment {
                    id: RecognitionId {
                        scope: scope.clone(),
                        input_start_sample: 0,
                        ordinal: 0,
                    },
                    range,
                    timing: TimingPrecision::NativeSegment,
                    text: "live".into(),
                    speaker: None,
                    revision: 1,
                    text_finality: Finality::Final,
                    speaker_finality: Finality::Final,
                }],
                speakers: vec![],
            };
            lock(&store).commit_recognition("test", &block).unwrap();
        }
        checkpoint::put(
            &store,
            "test",
            Stage::Transcribe,
            &format!("recognition-epoch-{:020}", 0),
            &serde_json::json!({ "scope": scope, "range": range,
                "output": { "text": true, "speakers": false } }),
        )
        .unwrap();

        super::super::stages::prepare_retry(&store, "test").unwrap();

        let mut words = vec![];
        visit_catchup_blocks(&lock(&store), "test", &generation, |block| {
            words.extend(block.rows.into_iter().map(|row| row.text));
            Ok(())
        })
        .unwrap();
        assert_eq!(words, ["live"]);
    }
}
