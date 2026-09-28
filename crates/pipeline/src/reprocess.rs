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
    let (Some(reference), Some(audio)) = (&doc.recognition_artifact, &doc.recognition_audio) else {
        return Ok(false);
    };
    if audio.len() != doc.segments.len() {
        return Ok(false);
    }
    for segment in &doc.segments {
        if audio.get(&segment.audio) != Some(&recognition::file_digest(&home.join(&segment.audio))?)
        {
            return Ok(false);
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
        Ok(())
    })
    .map_err(|e| StageError::Failed(e.to_string()))?;
    let transcript = std::fs::read(home.join("transcript.md"))?;
    let next = durable::spec_fingerprints(header.fingerprints.audio.clone(), spec)?;
    Ok(timed && header.reuse(&next, &transcript) != Reuse::TranscribeAgain)
}
