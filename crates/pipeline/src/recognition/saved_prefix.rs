//! Reuse completed output only for an unchanged saved prefix of a resumed note.
use super::{RouteResult, TaskGraph, failed};
use crate::{SharedStore, StageError, checkpoint, lock};
use models::recognition::ExecutionSpec;
use session_core::{Segment, Stage, recognition::*};
use std::path::Path;

pub(super) const KEY: &str = "recognition-saved-prefix-v1";

pub(super) struct Prefix {
    pub count: usize,
    pub result: RouteResult,
    proof: Proof,
    reference: ArtifactReference,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Proof {
    manifest: String,
    segments: usize,
}

/// Once an attempt uses a parent artifact, a changed parent is an error, not a
/// reason to publish already computed output with a different parent.
pub(super) fn read(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
    spec: &ExecutionSpec,
) -> Result<Option<Prefix>, StageError> {
    if lock(store)
        .session(id)?
        .is_some_and(|record| record.state == session_core::SessionState::Saved)
    {
        return Ok(None);
    }
    let mut previous: Option<Proof> = checkpoint::get(store, id, Stage::Transcribe, KEY)?;
    // A new appended segment starts a new Resume attempt. Do not carry a
    // completed attempt's parent-edit guard into its ordinary fallback path.
    if previous
        .as_ref()
        .is_some_and(|proof| proof.segments < segments.len())
    {
        lock(store).delete_checkpoint(id, Stage::Transcribe, KEY)?;
        previous = None;
    }
    let candidate = candidate(store, id, dir, segments, spec);
    // Save can commit the replacement manifest before the process records Done.
    // Accept only our exact joined artifact as that completed publication.
    if let Ok(Some(prefix)) = &candidate
        && prefix.count == segments.len()
    {
        let joined: Option<ArtifactReference> =
            checkpoint::get(store, id, Stage::Join, "recognition-artifact-reference-v1")?;
        if previous.is_none() || joined.as_ref() == Some(&prefix.reference) {
            return Ok(None);
        }
    }
    match (previous, candidate) {
        (Some(previous), Ok(Some(prefix))) if previous.manifest == prefix.proof.manifest => {
            Ok(Some(prefix))
        }
        (Some(_), _) => Err(failed(
            "The saved recording, transcript, or recognition settings changed during Resume processing. Saved work is preserved.",
        )),
        (None, Ok(value)) => Ok(value),
        // Old notes without a usable artifact retain the ordinary recognition path.
        (None, Err(_)) => Ok(None),
    }
}

pub(super) fn retain(store: &SharedStore, id: &str, prefix: &Prefix) -> Result<(), StageError> {
    checkpoint::put(store, id, Stage::Transcribe, KEY, &prefix.proof)
}

fn candidate(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
    spec: &ExecutionSpec,
) -> Result<Option<Prefix>, StageError> {
    let record = lock(store)
        .session(id)?
        .ok_or_else(|| failed("session is absent"))?;
    if lock(store).result_state(id, "pending-snapshot")?.is_some() {
        return Ok(None);
    }
    let Some(home) = record.home_path.as_deref().map(Path::new) else {
        return Ok(None);
    };
    let route = TaskGraph::resolve(spec)?.route;
    // An explicit speaker count constrains Pyannote's whole-session clustering.
    // Keep that path intact instead of forcing the count onto only the new audio.
    if route == super::Route::SeparatePyannote && record.speaker_count.is_some() {
        return Ok(None);
    }
    let manifest = file_digest(&home.join("session.md"))?;
    let doc = session_core::generation::manifest(home)?;
    let old = doc.timeline_segments();
    if old.is_empty()
        || old.len() > segments.len()
        || old != segments[..old.len()]
        || segments
            .windows(2)
            .any(|pair| pair[0].end_ms() > pair[1].offset_ms || pair[0].index >= pair[1].index)
        || doc.session_id != id
        || doc.recognition_snapshot.as_ref() != Some(&serde_json::to_value(spec)?)
        || doc.speaker_count != record.speaker_count
    {
        return Ok(None);
    }
    let (Some(reference), Some(audio)) = (&doc.recognition_artifact, &doc.recognition_audio) else {
        return Ok(None);
    };
    if audio.len() != old.len() {
        return Ok(None);
    }
    for segment in &old {
        let expected = audio.get(&segment.audio_file);
        if expected != Some(&file_digest(&home.join(&segment.audio_file))?) {
            return Ok(None);
        }
        // Assemble can prefer a local archive. It must represent the same source.
        let local = dir.join(&segment.audio_file);
        if local.exists() && expected != Some(&file_digest(&local)?) {
            return Ok(None);
        }
    }
    let artifact_dir = doc
        .result_generation
        .as_ref()
        .map(|g| g.directory(home))
        .transpose()?
        .unwrap_or_else(|| home.to_path_buf());
    let mut result = RouteResult {
        route,
        coverage: vec![],
        rows: vec![],
        speakers: vec![],
        coverage_limitations: vec![],
    };
    let header = read_artifact(
        &artifact_dir.join(&reference.filename),
        reference,
        |block| {
            result.rows.extend(block.rows);
            result.speakers.extend(block.speakers);
            Ok(())
        },
    )
    .map_err(|e| failed(e.to_string()))?;
    let generation: Option<String> =
        checkpoint::get(store, id, Stage::Transcribe, "recognition-generation-v1")?;
    let expected = super::durable::configured_fingerprints(
        store,
        id,
        header.fingerprints.audio.clone(),
        spec,
    )?;
    let expected_coverage = super::stages::ranges(dir, &old)?;
    let same_coverage = expected_coverage
        .iter()
        .all(|range| super::covers(&header.coverage, *range))
        && header
            .coverage
            .iter()
            .all(|range| super::covers(&expected_coverage, *range));
    if generation.as_ref() != Some(&header.generation)
        || header.snapshot != serde_json::to_value(spec)?
        || header.fingerprints != expected
        || digest(&session_core::generation::read(
            home,
            &doc,
            "transcript.md",
        )?) != header.transcript
        || !same_coverage
    {
        return Ok(None);
    }
    result.coverage = header.coverage;
    result.coverage_limitations = header.limitations;
    super::validate_result(&result, &result.coverage)?;
    if manifest != file_digest(&home.join("session.md"))? {
        return Ok(None);
    }
    // Check the source again after reading output; publication repeats this proof.
    for segment in &old {
        if audio.get(&segment.audio_file) != Some(&file_digest(&home.join(&segment.audio_file))?) {
            return Ok(None);
        }
    }
    Ok(Some(Prefix {
        count: old.len(),
        result,
        reference: reference.clone(),
        proof: Proof {
            manifest,
            segments: segments.len(),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use models::recognition::{RecognitionConfig, SpeakerSource};
    use std::sync::{Arc, Mutex};

    struct Case {
        temp: tempfile::TempDir,
        store: SharedStore,
        spec: ExecutionSpec,
        segments: Vec<Segment>,
    }
    impl Case {
        fn new() -> Self {
            Self::with_segments(2)
        }
        fn with_segments(count: u32) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let store = Arc::new(Mutex::new(session_core::Store::open_in_memory().unwrap()));
            let at = chrono::DateTime::parse_from_rfc3339("2026-09-28T00:00:00Z").unwrap();
            lock(&store).create_session("s", at, None).unwrap();
            lock(&store)
                .set_home_path("s", temp.path().to_str().unwrap())
                .unwrap();
            let spec = models::recognition::resolve(&RecognitionConfig {
                asr_model: "whisper-large-v3-turbo-q5km".into(),
                speaker_source: SpeakerSource::Nemotron,
                ..Default::default()
            })
            .unwrap();
            let segments = (0..count)
                .map(|index| Segment {
                    index,
                    offset_ms: u64::from(index) * 2000,
                    duration_ms: 1000,
                    wall_start: at,
                    wall_end: at + chrono::Duration::seconds(1),
                    audio_file: format!("audio/{index}.opus"),
                    channels: 1,
                })
                .collect::<Vec<_>>();
            for segment in &segments {
                lock(&store).put_segment("s", segment).unwrap();
                std::fs::create_dir_all(temp.path().join("audio")).unwrap();
                std::fs::write(
                    temp.path().join(&segment.audio_file),
                    [segment.index as u8; 8],
                )
                .unwrap();
                let path = temp.path().join(crate::proc_file(&segment.audio_file));
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let mut wav = hound::WavWriter::create(
                    path,
                    hound::WavSpec {
                        channels: 1,
                        sample_rate: 16000,
                        bits_per_sample: 32,
                        sample_format: hound::SampleFormat::Float,
                    },
                )
                .unwrap();
                for _ in 0..16000 {
                    wav.write_sample(0.0_f32).unwrap();
                }
                wav.finalize().unwrap();
            }
            checkpoint::put(
                &store,
                "s",
                Stage::Transcribe,
                "recognition-generation-v1",
                &"g",
            )
            .unwrap();
            let case = Self {
                temp,
                store,
                spec,
                segments,
            };
            case.publish(1);
            case
        }
        fn publish(&self, count: usize) {
            let coverage =
                super::super::stages::ranges(self.temp.path(), &self.segments[..count]).unwrap();
            self.publish_ranges(count, coverage);
        }
        fn publish_ranges(&self, count: usize, coverage: Vec<SampleRange>) {
            let speakers = lock(&self.store)
                .session("s")
                .unwrap()
                .unwrap()
                .speaker_count;
            let mut doc =
                session_core::folder::SessionDoc::new("s", speakers, &self.segments[..count])
                    .unwrap();
            doc.recognition_snapshot = Some(serde_json::to_value(&self.spec).unwrap());
            let text = b"Saved transcript";
            std::fs::write(self.temp.path().join("transcript.md"), text).unwrap();
            let header = ArtifactHeader {
                version: ARTIFACT_VERSION,
                generation: "g".into(),
                snapshot: serde_json::to_value(&self.spec).unwrap(),
                fingerprints: super::super::durable::configured_fingerprints(
                    &self.store,
                    "s",
                    "verified source".into(),
                    &self.spec,
                )
                .unwrap(),
                transcript: digest(text),
                limitations: vec![],
                coverage: coverage.clone(),
            };
            let blocks = coverage.iter().enumerate().map(|(index, range)| {
                let scope = SpeakerScope {
                    generation: "g".into(),
                    epoch: index as u64 + 10,
                };
                let speaker = SpeakerKey {
                    scope: scope.clone(),
                    input_start_sample: range.start,
                    local_id: 1,
                };
                OutputBlock {
                    key: format!("saved/{index}"),
                    scope: scope.clone(),
                    revision: 1,
                    coverage: Coverage {
                        received: Some(*range),
                        computed: Some(*range),
                        provisional: None,
                        committed: Some(*range),
                    },
                    rows: vec![RecognitionSegment {
                        id: RecognitionId {
                            scope,
                            input_start_sample: range.start,
                            ordinal: 0,
                        },
                        range: *range,
                        timing: TimingPrecision::NativeSegment,
                        text: "saved".into(),
                        speaker: Some(speaker.clone()),
                        revision: 1,
                        text_finality: Finality::Final,
                        speaker_finality: Finality::Final,
                    }],
                    speakers: vec![SpeakerTurn {
                        range: *range,
                        timing: TimingPrecision::NativeSegment,
                        speaker,
                        finality: Finality::Final,
                    }],
                }
            });
            doc.recognition_artifact = Some(
                write_artifact(&self.temp.path().join(ARTIFACT_FILE), &header, blocks).unwrap(),
            );
            doc.recognition_audio = Some(
                self.segments[..count]
                    .iter()
                    .map(|segment| {
                        (
                            segment.audio_file.clone(),
                            file_digest(&self.temp.path().join(&segment.audio_file)).unwrap(),
                        )
                    })
                    .collect(),
            );
            std::fs::write(self.temp.path().join("session.md"), doc.render()).unwrap();
        }
        fn read(&self) -> Result<Option<Prefix>, StageError> {
            read(
                &self.store,
                "s",
                self.temp.path(),
                &self.segments,
                &self.spec,
            )
        }
    }

    #[test]
    fn saved_prefix_rejects_changed_inputs_before_reuse() {
        for change in [
            "archive",
            "transcript",
            "geometry",
            "model",
            "runtime",
            "preprocessing",
            "generation",
            "artifact",
            "speaker-count",
        ] {
            let mut case = Case::new();
            assert!(case.read().unwrap().is_some());
            match change {
                "archive" => {
                    std::fs::write(case.temp.path().join("audio/0.opus"), b"changed").unwrap()
                }
                "transcript" => {
                    std::fs::write(case.temp.path().join("transcript.md"), b"edited").unwrap()
                }
                "geometry" => case.segments[0].offset_ms += 1,
                "model" => case.spec.artifacts[0].sha256.push('x'),
                "runtime" => case.spec.runtimes[0].revision.push('x'),
                "preprocessing" => case.spec.preprocessing_revision.push('x'),
                "generation" => checkpoint::put(
                    &case.store,
                    "s",
                    Stage::Transcribe,
                    "recognition-generation-v1",
                    &"other",
                )
                .unwrap(),
                "artifact" => {
                    std::fs::write(case.temp.path().join(ARTIFACT_FILE), b"incomplete").unwrap()
                }
                "speaker-count" => lock(&case.store)
                    .set_speaker_count("s", Some(session_core::SpeakerCount::Exact(3)))
                    .unwrap(),
                _ => unreachable!(),
            }
            assert!(case.read().unwrap().is_none(), "accepted changed {change}");
        }
    }

    #[test]
    fn saved_prefix_rechecks_parent_after_reuse_and_allows_retry() {
        for filename in ["session.md", "transcript.md", "audio/0.opus", ARTIFACT_FILE] {
            let case = Case::new();
            let prefix = case.read().unwrap().unwrap();
            retain(&case.store, "s", &prefix).unwrap();
            assert!(case.read().unwrap().is_some(), "unchanged retry must work");
            std::fs::write(
                case.temp.path().join(filename),
                b"changed during processing",
            )
            .unwrap();
            assert!(
                case.read().is_err(),
                "publication accepted changed {filename}"
            );
        }
    }

    #[test]
    fn saved_prefix_accepts_next_completed_parent_on_repeated_resume() {
        let mut case = Case::new();
        let prefix = case.read().unwrap().unwrap();
        retain(&case.store, "s", &prefix).unwrap();
        case.publish(2);
        let mut next = case.segments[1].clone();
        next.index = 2;
        next.offset_ms = 4000;
        next.audio_file = "audio/2.opus".into();
        case.segments.push(next);
        let prefix = case.read().unwrap().unwrap();
        assert_eq!(prefix.count, 2);
        assert_eq!(prefix.result.rows.len(), 2);
        assert_eq!(prefix.result.speakers[0].speaker.scope.epoch, 10);
    }
    #[test]
    fn saved_prefix_accepts_only_its_committed_publication_on_save_retry() {
        let case = Case::new();
        let prefix = case.read().unwrap().unwrap();
        retain(&case.store, "s", &prefix).unwrap();
        case.publish(2);
        assert!(
            case.read().is_err(),
            "an unrelated full parent is not our publication"
        );
        let doc = session_core::generation::manifest(case.temp.path()).unwrap();
        checkpoint::put(
            &case.store,
            "s",
            Stage::Join,
            "recognition-artifact-reference-v1",
            &doc.recognition_artifact.unwrap(),
        )
        .unwrap();
        assert!(
            case.read().unwrap().is_none(),
            "committed Save retry must not reuse its own new result as an old prefix"
        );
    }

    #[test]
    fn saved_prefix_parent_edit_fails_publishing_attempt() {
        let case = Case::new();
        for segment in &case.segments {
            checkpoint::put(&case.store, "s", Stage::Assemble, &format!("audio-provenance-{:08}", segment.index), &serde_json::json!({
                "archive": file_digest(&case.temp.path().join(&segment.audio_file)).unwrap(),
                "processing": file_digest(&case.temp.path().join(crate::proc_file(&segment.audio_file))).unwrap(),
                "echo": digest(b"null"),
            })).unwrap();
        }
        let mut attempt = super::super::durable::Attempt::default();
        attempt
            .prepare(
                &case.store,
                "s",
                case.temp.path(),
                &case.segments,
                &case.spec,
                false,
            )
            .unwrap();
        std::fs::write(
            case.temp.path().join("transcript.md"),
            b"external edit after recognition",
        )
        .unwrap();
        let error = attempt
            .prepare(
                &case.store,
                "s",
                case.temp.path(),
                &case.segments,
                &case.spec,
                true,
            )
            .unwrap_err();
        assert!(error.to_string().contains("changed during Resume"));
    }
    #[test]
    fn review_split_epoch_coverage_reuses_three_segment_prefix() {
        let case = Case::with_segments(4);
        let spans = vec![
            SampleRange {
                start: 0,
                end: 4000,
            },
            SampleRange {
                start: 4000,
                end: 8000,
            },
            SampleRange {
                start: 8000,
                end: 12000,
            },
            SampleRange {
                start: 12000,
                end: 16000,
            },
            SampleRange {
                start: 32000,
                end: 36000,
            },
            SampleRange {
                start: 36000,
                end: 40000,
            },
            SampleRange {
                start: 40000,
                end: 44000,
            },
            SampleRange {
                start: 44000,
                end: 48000,
            },
            SampleRange {
                start: 64000,
                end: 68000,
            },
            SampleRange {
                start: 68000,
                end: 72000,
            },
            SampleRange {
                start: 72000,
                end: 80000,
            },
        ];
        case.publish_ranges(3, spans.clone());
        let prefix = case
            .read()
            .unwrap()
            .expect("equivalent eleven-epoch coverage must reuse three saved recording segments");
        assert_eq!(prefix.count, 3);
        assert_eq!(prefix.result.coverage, spans);
    }

    #[test]
    fn review_split_epoch_coverage_rejects_gaps_and_extra_audio() {
        for spans in [
            vec![
                SampleRange {
                    start: 0,
                    end: 7999,
                },
                SampleRange {
                    start: 8000,
                    end: 16000,
                },
            ],
            vec![
                SampleRange {
                    start: 0,
                    end: 8000,
                },
                SampleRange {
                    start: 8000,
                    end: 16001,
                },
            ],
        ] {
            let case = Case::new();
            case.publish_ranges(1, spans);
            assert!(case.read().unwrap().is_none());
        }
    }

    #[test]
    fn review_finished_resume_proof_does_not_block_later_edit_fallback() {
        for edit in ["transcript", "speaker-count"] {
            let mut case = Case::new();
            retain(&case.store, "s", &case.read().unwrap().unwrap()).unwrap();
            case.publish(2);
            let mut appended = case.segments[1].clone();
            appended.index = 2;
            appended.offset_ms = 4000;
            appended.audio_file = "audio/2.opus".into();
            case.segments.push(appended);
            if edit == "transcript" {
                std::fs::write(
                    case.temp.path().join("transcript.md"),
                    b"corrected between completed Resumes",
                )
                .unwrap();
            } else {
                lock(&case.store)
                    .set_speaker_count("s", Some(session_core::SpeakerCount::Exact(3)))
                    .unwrap();
            }
            assert!(
                case.read()
                    .expect(
                        "a proof from the previous completed Resume must permit ordinary fallback"
                    )
                    .is_none()
            );
            assert!(
                checkpoint::get::<Proof>(&case.store, "s", Stage::Transcribe, KEY)
                    .unwrap()
                    .is_none(),
                "expired proof must not reappear on retry"
            );
            assert!(case.read().unwrap().is_none());
        }
    }

    #[test]
    fn review_pyannote_global_speaker_count_declines_tail_only_clustering() {
        for count in [
            models::recognition::SpeakerCount::Exact { count: 3 },
            models::recognition::SpeakerCount::Range { min: 2, max: 4 },
        ] {
            let mut case = Case::new();
            case.spec = models::recognition::resolve(&RecognitionConfig {
                asr_model: "parakeet-tdt-0.6b-v3-q80".into(),
                speaker_source: SpeakerSource::Pyannote,
                speaker_count: Some(count),
                ..Default::default()
            })
            .unwrap();
            let effective = match count {
                models::recognition::SpeakerCount::Exact { count } => {
                    session_core::SpeakerCount::Exact(count)
                }
                models::recognition::SpeakerCount::Range { min, max } => {
                    session_core::SpeakerCount::Range { min, max }
                }
            };
            lock(&case.store)
                .set_speaker_count("s", Some(effective))
                .unwrap();
            case.publish(1);
            assert!(
                case.read().unwrap().is_none(),
                "a whole-meeting count must keep whole-meeting clustering"
            );
        }
    }
}
