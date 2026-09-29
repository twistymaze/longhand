//! The diarize stage (KTD7, R16, R36). It runs before transcription so the
//! windowing can cut at speaker changes. Each segment's analysis is written
//! beside its processing audio and checkpointed; then every analysis is
//! clustered together, the turns smoothed, and the result checkpointed.

use std::path::Path;

use diarize::{
    DiarizeError, DiarizeRequest, SpeakerAnalyzer, SpeakerBounds, Turn, WINDOW_SAMPLES,
    WavAudioSource, analysis_path, cluster_session, read_analysis, smooth, write_analysis,
};
use serde::{Deserialize, Serialize};
use session_core::{Segment, SpeakerCount, Stage};

use crate::progress::SpeakerMeter;
use crate::{Control, SharedStore, StageError, checkpoint, proc_file};

pub const TURNS: &str = "turns";
/// Written instead of [`TURNS`] when diarization failed: the transcript is
/// delivered without speaker marks and says so (R20a).
pub const FAILED: &str = "failed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyzedSegment {
    pub segment: u32,
    pub windows: usize,
    pub embeddings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiarizeFailure {
    pub message: String,
}

/// What the join has to work with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Speakers {
    Turns(Vec<Turn>),
    /// Diarization failed; the transcript carries no speaker marks.
    Unavailable,
}

pub fn bounds(count: Option<SpeakerCount>) -> SpeakerBounds {
    match count {
        Some(SpeakerCount::Exact(n)) => SpeakerBounds::exact(n as usize),
        Some(SpeakerCount::Range { min, max }) => SpeakerBounds {
            min_speakers: Some(min as usize),
            max_speakers: Some(max as usize),
            ..SpeakerBounds::default()
        },
        None => SpeakerBounds::default(),
    }
}

/// The analysis work of one segment, in samples. A segment shorter than one
/// analysis window costs one window (KTD2).
pub fn segment_units(segment: &Segment) -> u64 {
    (segment.duration_ms * 16).max(WINDOW_SAMPLES as u64)
}

/// The analysis work of `segments`, in the units of [`segment_units`].
pub fn units(segments: &[Segment]) -> u64 {
    segments.iter().map(segment_units).sum()
}

/// Analyse each segment, then cluster them together. `speakers` is planned
/// for the whole note; each segment adds its [`segment_units`].
#[allow(clippy::too_many_arguments)]
pub fn run(
    store: &SharedStore,
    session_id: &str,
    session_dir: &Path,
    segments: &[Segment],
    speaker_count: Option<SpeakerCount>,
    analyzer: &mut dyn SpeakerAnalyzer,
    control: &dyn Control,
    speakers: &mut SpeakerMeter,
) -> Result<(), StageError> {
    // Count the saved work first, so the first value covers it.
    let mut pending = Vec::with_capacity(segments.len());
    for segment in segments {
        let key = checkpoint::segment_key(segment.index);
        let file = analysis_path(&session_dir.join(proc_file(&segment.audio_file)));
        let done = checkpoint::get::<AnalyzedSegment>(store, session_id, Stage::Diarize, &key)?
            .is_some()
            && file.exists();
        // Features saved during recording need no new analysis.
        let counted = done || crate::live::features::saved(store, session_id, segment.index)?;
        if counted {
            speakers.saved(segment_units(segment));
        }
        if !done {
            pending.push((segment, key, file, counted));
        }
    }
    speakers.at(0);
    for (segment, key, file, counted) in pending {
        let units = segment_units(segment);
        control.proceed()?;
        let live_analysis =
            match crate::live::features::restore(store, session_id, session_dir, segment.index)? {
                Some(analysis) => Some(analysis),
                None => crate::live::features::complete(
                    store,
                    session_id,
                    session_dir,
                    segment,
                    analyzer,
                    control,
                    &mut |fraction| speakers.at_fraction(units, fraction),
                )?,
            };
        if let Some(analysis) = live_analysis {
            write_analysis(&file, &analysis)?;
            checkpoint::put(
                store,
                session_id,
                Stage::Diarize,
                &key,
                &AnalyzedSegment {
                    segment: segment.index,
                    windows: analysis.activities.len(),
                    embeddings: analysis.embeddings.len(),
                },
            )?;
            if !counted {
                speakers.finished(units);
            }
            continue;
        }
        let mut source = WavAudioSource::open(&session_dir.join(proc_file(&segment.audio_file)))
            .map_err(|e| StageError::Failed(e.to_string()))?;
        // The stack polls once per 2 s window; that poll is where a resume
        // cancels and where capture suspends it (KTD20).
        let stop = std::cell::Cell::new(None::<StageError>);
        let is_cancelled = || match control.proceed() {
            Ok(()) => false,
            Err(e) => {
                stop.set(Some(e));
                true
            }
        };
        let meter = std::cell::RefCell::new(&mut *speakers);
        let report = |fraction: f32| meter.borrow_mut().at_fraction(units, fraction);
        let analysis = analyzer.analyze(
            &mut source,
            &DiarizeRequest {
                bounds: SpeakerBounds::default(),
                is_cancelled: Some(&is_cancelled),
                on_progress: Some(&report),
            },
        );
        let analysis = match analysis {
            Ok(analysis) => analysis,
            Err(e) => {
                return Err(match (stop.take(), e) {
                    // The poll above said stop; pass on why.
                    (Some(reason), _) => reason,
                    (None, DiarizeError::MissingWeights(path)) => StageError::Waiting(path),
                    (None, e) => StageError::Failed(e.to_string()),
                });
            }
        };
        write_analysis(&file, &analysis)?;
        checkpoint::put(
            store,
            session_id,
            Stage::Diarize,
            &key,
            &AnalyzedSegment {
                segment: segment.index,
                windows: analysis.activities.len(),
                embeddings: analysis.embeddings.len(),
            },
        )?;
        speakers.finished(units);
        log::info!(
            "analysed segment {} of {session_id}: {} windows, {} embeddings",
            segment.index,
            analysis.activities.len(),
            analysis.embeddings.len()
        );
    }

    // Clustering, smoothing, and the writes give no fraction (KTD3).
    speakers.unmeasured();
    control.proceed()?;
    let mut analyses = Vec::with_capacity(segments.len());
    for segment in segments {
        analyses.push(read_analysis(&analysis_path(
            &session_dir.join(proc_file(&segment.audio_file)),
        ))?);
    }
    let refs: Vec<_> = analyses.iter().collect();
    let raw = cluster_session(&refs, bounds(speaker_count));
    let per_segment: Vec<_> = segments.iter().map(|s| s.index).zip(raw).collect();
    let turns = smooth(&per_segment);
    let found: std::collections::BTreeSet<u32> = turns.iter().map(|t| t.speaker).collect();
    log::info!(
        "diarized {session_id}: {} turns, {} speakers",
        turns.len(),
        found.len()
    );
    // A retry after a failure must not keep the failure's marker.
    lock_clear_failure(store, session_id)?;
    checkpoint::put(store, session_id, Stage::Diarize, TURNS, &turns)?;
    checkpoint::mark_complete(store, session_id, Stage::Diarize, segments.len())?;
    Ok(())
}

fn lock_clear_failure(store: &SharedStore, session_id: &str) -> Result<(), StageError> {
    let rows = checkpoint::all(store, session_id, Stage::Diarize)?;
    if rows.iter().any(|(k, _)| k == FAILED) {
        let kept: Vec<_> = rows.into_iter().filter(|(k, _)| k != FAILED).collect();
        let guard = crate::lock(store);
        guard.clear_checkpoints(session_id, Stage::Diarize)?;
        for (k, v) in kept {
            guard.put_checkpoint(session_id, Stage::Diarize, &k, &v)?;
        }
    }
    Ok(())
}

/// Record that diarization failed, so the stages after it proceed without
/// speakers instead of failing the session (R20a).
pub fn record_failure(
    store: &SharedStore,
    session_id: &str,
    segments: usize,
    message: &str,
) -> Result<(), StageError> {
    checkpoint::put(
        store,
        session_id,
        Stage::Diarize,
        FAILED,
        &DiarizeFailure {
            message: message.to_string(),
        },
    )?;
    checkpoint::mark_complete(store, session_id, Stage::Diarize, segments)
}

pub fn speakers(store: &SharedStore, session_id: &str) -> Result<Speakers, StageError> {
    if checkpoint::get::<DiarizeFailure>(store, session_id, Stage::Diarize, FAILED)?.is_some() {
        return Ok(Speakers::Unavailable);
    }
    Ok(
        match checkpoint::get::<Vec<Turn>>(store, session_id, Stage::Diarize, TURNS)? {
            Some(turns) => Speakers::Turns(turns),
            None => Speakers::Unavailable,
        },
    )
}
