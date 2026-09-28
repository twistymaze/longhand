//! Preview and validate an exact replacement request before work starts.
use crate::{StageError, recognition::durable};
use models::recognition::{ExecutionSpec, RecognitionConfig};
use session_core::{
    folder::SessionDoc,
    recognition::{self, Reuse},
};
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Preview {
    pub execution: ExecutionSpec,
    pub requires_transcription: bool,
    pub explanation: String,
    pub manifest: String,
}

pub(crate) fn speaker_count(spec: &ExecutionSpec) -> Option<session_core::SpeakerCount> {
    spec.config.speaker_count.map(|count| match count {
        models::recognition::SpeakerCount::Exact { count } => {
            session_core::SpeakerCount::Exact(count)
        }
        models::recognition::SpeakerCount::Range { min, max } => {
            session_core::SpeakerCount::Range { min, max }
        }
    })
}

pub fn preview(home: &Path, config: &RecognitionConfig) -> Result<Preview, StageError> {
    let execution =
        models::recognition::resolve(config).map_err(|e| StageError::Failed(e.to_string()))?;
    crate::recognition::TaskGraph::resolve(&execution)?;
    let doc = session_core::generation::manifest(home)?;
    let reusable = reusable(home, &doc, &execution)?;
    Ok(Preview {
        execution,
        requires_transcription: !reusable,
        explanation: if reusable { "Saved timed text can be reused. Speaker labels and the summary will be made again." }
            else { "This change requires transcription again. The note has no compatible verified timed text, or its transcript or audio changed." }.into(),
        manifest: session_core::folder::fingerprint(&std::fs::read(home.join("session.md"))?),
    })
}

pub(crate) fn reusable(
    home: &Path,
    doc: &SessionDoc,
    spec: &ExecutionSpec,
) -> Result<bool, StageError> {
    Ok(read_reusable(home, doc, spec, |_| {})?.is_some())
}

/// Callers retain visited rows only when the returned header confirms reuse.
pub(crate) fn read_reusable(
    home: &Path,
    doc: &SessionDoc,
    spec: &ExecutionSpec,
    mut visit: impl FnMut(recognition::OutputBlock),
) -> Result<Option<recognition::ArtifactHeader>, StageError> {
    if matches!(
        crate::recognition::TaskGraph::resolve(spec)?.route,
        crate::recognition::Route::Integrated | crate::recognition::Route::CombinedNemo
    ) {
        return Ok(None);
    }
    let (Some(reference), Some(audio)) = (&doc.recognition_artifact, &doc.recognition_audio) else {
        return Ok(None);
    };
    if audio.len() != doc.segments.len() {
        return Ok(None);
    }
    for segment in &doc.segments {
        if audio.get(&segment.audio) != Some(&recognition::file_digest(&home.join(&segment.audio))?)
        {
            return Ok(None);
        }
    }
    let dir = doc
        .result_generation
        .as_ref()
        .map(|g| g.directory(home))
        .transpose()?
        .unwrap_or_else(|| home.to_path_buf());
    let mut timed = true;
    let header = recognition::read_artifact(&dir.join(&reference.filename), reference, |b| {
        timed &= b.timed_asr();
        visit(b);
        Ok(())
    })
    .map_err(|e| StageError::Failed(e.to_string()))?;
    let transcript = session_core::generation::read(home, doc, "transcript.md")?;
    let next = durable::spec_fingerprints(header.fingerprints.audio.clone(), spec)?;
    Ok((timed && header.reuse(&next, &transcript) != Reuse::TranscribeAgain).then_some(header))
}

#[cfg(test)]
mod tests {
    use super::*;
    use models::recognition::{ProcessingMode, SpeakerSource};
    use session_core::recognition::*;

    fn saved(config: &RecognitionConfig) -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let spec = models::recognition::resolve(config).unwrap();
        let transcript = b"Saved timed text";
        let mut doc: SessionDoc = serde_json::from_value(serde_json::json!({
            "session_id": "s", "started": "2026-09-27T10:00:00-05:00", "segments": []
        }))
        .unwrap();
        let header = ArtifactHeader {
            version: ARTIFACT_VERSION,
            generation: "g".into(),
            snapshot: serde_json::to_value(&spec).unwrap(),
            fingerprints: durable::spec_fingerprints(digest(b"audio"), &spec).unwrap(),
            transcript: digest(transcript),
            limitations: vec![],
            coverage: vec![SampleRange {
                start: 0,
                end: 16000,
            }],
        };
        let range = SampleRange {
            start: 0,
            end: 16000,
        };
        let scope = SpeakerScope {
            generation: "g".into(),
            epoch: 0,
        };
        let block = OutputBlock {
            key: "row".into(),
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
                    scope,
                    input_start_sample: 0,
                    ordinal: 0,
                },
                range,
                timing: TimingPrecision::NativeSegment,
                text: "Saved timed text".into(),
                speaker: None,
                revision: 1,
                text_finality: Finality::Final,
                speaker_finality: Finality::Final,
            }],
            speakers: vec![],
        };
        doc.recognition_artifact =
            Some(write_artifact(&home.join("recognition.jsonl"), &header, [block]).unwrap());
        doc.recognition_audio = Some(Default::default());
        std::fs::write(home.join("transcript.md"), transcript).unwrap();
        std::fs::write(home.join("summary.md"), b"Summary").unwrap();
        doc.result_generation = Some(session_save::result::stage(home, home).unwrap());
        std::fs::write(home.join("session.md"), doc.render()).unwrap();
        temp
    }

    #[test]
    fn integrated_and_combined_routes_require_transcription_consent() {
        for config in [
            RecognitionConfig {
                asr_model: "moss-transcribe-diarize-q80".into(),
                speaker_source: SpeakerSource::Integrated,
                ..Default::default()
            },
            RecognitionConfig {
                asr_model: "nemotron-speech-streaming-en-0.6b-q80".into(),
                speaker_source: SpeakerSource::Nemotron,
                processing_mode: ProcessingMode::DuringRecording,
                speaker_processing_mode: ProcessingMode::DuringRecording,
                ..Default::default()
            },
        ] {
            let temp = saved(&config);
            assert!(
                preview(temp.path(), &config)
                    .unwrap()
                    .requires_transcription
            );
        }
    }

    #[test]
    fn reusable_text_comes_from_the_verified_generation() {
        let config = RecognitionConfig {
            speaker_source: SpeakerSource::Off,
            ..Default::default()
        };
        let temp = saved(&config);
        assert!(
            !preview(temp.path(), &config)
                .unwrap()
                .requires_transcription
        );
        std::fs::remove_file(temp.path().join("transcript.md")).unwrap();
        assert!(
            !preview(temp.path(), &config)
                .unwrap()
                .requires_transcription
        );
        std::fs::write(temp.path().join("transcript.md"), b"External edit").unwrap();
        assert!(preview(temp.path(), &config).is_err());
    }
}
