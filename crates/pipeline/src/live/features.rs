//! Persist segmentation and embedding windows during capture. Clustering runs
//! after Stop. The native analyzer handles accelerated inference fallback.
use super::{CaptureEnd, LiveConfig, LiveEnd, Step, archive, next_step};
use crate::{SharedStore, StageError, checkpoint};
use diarize::{Analysis, SpeakerAnalyzer};
use session_core::Stage;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
const WINDOW: usize = 160_000;
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Window {
    digest: String,
    pub(crate) file: PathBuf,
    step: usize,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Index {
    total: usize,
    step: usize,
    windows: usize,
    /// Where window 0 starts in the segment (KTD18 of the
    /// forgotten-recording plan). See [`grid_key`].
    #[serde(default)]
    grid_start: i64,
}
pub fn key(segment: u32) -> String {
    format!("live-features-{segment:08}")
}
pub(crate) fn window_key(segment: u32, window: usize) -> String {
    format!("live-feature-{segment:08}-{window:012}")
}
/// The grid start of a part of a cut segment: the sample of the part where
/// window 0 starts. Window `w` starts at `grid_start + w * step`. A part
/// keeps the windows of the original segment that lie fully inside it, so
/// its grid is the grid of the original segment (KTD18 of the
/// forgotten-recording plan). It is 0 or negative: a window that starts
/// before the part reads zeros there. A segment with no grid start has the
/// grid start 0. The key starts with `live-feature-`, so it stays when the
/// recognition identity changes, as the windows stay.
pub fn grid_key(segment: u32) -> String {
    format!("live-feature-{segment:08}-grid")
}

pub fn extract(
    store: &SharedStore,
    config: &LiveConfig,
    analyzer: &mut dyn SpeakerAnalyzer,
    cancel: &CancellationToken,
    end: &CaptureEnd,
) -> Result<(), StageError> {
    let archive = config
        .archive
        .clone()
        .ok_or_else(|| StageError::Failed("live feature source is absent".into()))?;
    let dir = archive
        .dir
        .parent()
        .ok_or_else(|| StageError::Failed("live recording folder is absent".into()))?
        .join("live-features");
    std::fs::create_dir_all(&dir)?;
    let mut source = archive::ArchiveReader::new(archive.clone());
    let mut resample = crate::resample::StreamResampler::new(archive.rate, 16000);
    let mut pending = Vec::new();
    let mut total = 0;
    let mut start = 0;
    let mut last_end = 0;
    let mut windows = 0;
    let mut step = 0;
    let mut flushed = false;
    loop {
        if cancel.is_cancelled() {
            return Err(StageError::Cancelled);
        }
        if *end.lock().unwrap_or_else(|e| e.into_inner()) == Some(LiveEnd::Discard) {
            return Ok(());
        }
        if !flushed {
            let mut append = |pcm: &[f32]| {
                pending.extend_from_slice(pcm);
                total += pcm.len();
            };
            match next_step(end, || source.next_frame())? {
                Step::Frame(frame) => resample.push(&frame.pcm, &mut append),
                Step::Finish => {
                    resample.finish(&mut append);
                    flushed = true;
                }
                Step::Wait => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            }
        }
        if pending.len() >= WINDOW || (flushed && total > last_end && !pending.is_empty()) {
            let count = pending.len().min(WINDOW);
            let is_cancelled = || cancel.is_cancelled();
            let saved = window(
                store,
                &config.note_id,
                &dir,
                config.segment,
                windows,
                &config.snapshot,
                &pending[..count],
                analyzer,
                &is_cancelled,
            )?;
            if step != 0 && step != saved.step {
                return Err(StageError::Failed("live feature hop changed".into()));
            }
            step = saved.step;
            windows += 1;
            last_end = start + count;
            if pending.len() < WINDOW {
                break;
            }
            pending.drain(..step);
            start += step;
        } else if flushed {
            break;
        }
    }
    checkpoint::put(
        store,
        &config.note_id,
        Stage::Diarize,
        &key(config.segment),
        &Index {
            total,
            step,
            windows,
            grid_start: 0,
        },
    )?;
    archive::save_proof(store, config, &source)
}

#[allow(clippy::too_many_arguments)]
fn window(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segment: u32,
    index: usize,
    snapshot: &Option<models::recognition::ExecutionSpec>,
    pcm: &[f32],
    analyzer: &mut dyn SpeakerAnalyzer,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<Window, StageError> {
    let mut digest = Sha256::new();
    digest.update(serde_json::to_vec(snapshot)?);
    for sample in pcm {
        digest.update(sample.to_le_bytes());
    }
    let digest = format!("{:x}", digest.finalize());
    let key = window_key(segment, index);
    let cached: Option<Window> = checkpoint::get(store, id, Stage::Diarize, &key)?;
    if let Some(cached) = cached.filter(|c| {
        c.digest == digest && c.step > 0 && c.step <= WINDOW && dir.join(&c.file).is_file()
    }) {
        return Ok(cached);
    }
    let mut source = pcm;
    let analysis = analyzer
        .analyze(
            &mut source,
            &diarize::DiarizeRequest {
                is_cancelled: Some(is_cancelled),
                ..Default::default()
            },
        )
        .map_err(|e| match e {
            diarize::DiarizeError::MissingWeights(path) => StageError::Waiting(path),
            e => StageError::Failed(e.to_string()),
        })?;
    if analysis.step == 0 || analysis.step > WINDOW || analysis.total != pcm.len() {
        return Err(StageError::Failed("invalid live feature layout".into()));
    }
    // The digest is in the name: after a cut, a part keeps the files of
    // the original segment, and a window of another segment can get the
    // same index (KTD18 of the forgotten-recording plan).
    let file = PathBuf::from(format!("{segment:08}-{index:012}-{}.diar", &digest[..16]));
    diarize::write_analysis(&dir.join(&file), &analysis)?;
    let saved = Window {
        digest,
        file,
        step: analysis.step,
    };
    checkpoint::put(store, id, Stage::Diarize, &key, &saved)?;
    Ok(saved)
}

/// Verify partial windows against the processing PCM and compute missing windows.
/// Initial mono Assemble writes this PCM from capture, before Opus encoding.
/// An archive decode or changed preprocessing must pass each PCM digest again.
/// `progress` receives the fraction of the segment's samples that is done.
/// A part of a cut segment uses the grid of its [`grid_key`]. Its windows
/// at the cut edges are computed again; the samples before its sample 0
/// are zeros.
pub fn complete(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segment: &session_core::Segment,
    analyzer: &mut dyn SpeakerAnalyzer,
    control: &dyn crate::Control,
    progress: &mut dyn FnMut(f32),
) -> Result<Option<Analysis>, StageError> {
    use diarize::AudioSource;
    let grid: Option<i64> = checkpoint::get(store, id, Stage::Diarize, &grid_key(segment.index))?;
    if grid.is_none()
        && checkpoint::get::<Window>(store, id, Stage::Diarize, &window_key(segment.index, 0))?
            .is_none()
    {
        return Ok(None);
    }
    let grid_start = grid.unwrap_or(0).min(0);
    let snapshot = crate::recognition::snapshot(store, id)?;
    let mut source =
        diarize::WavAudioSource::open(&dir.join(crate::proc_file(&segment.audio_file)))
            .map_err(|e| StageError::Failed(e.to_string()))?;
    let total = source
        .sample_count()
        .map_err(|e| StageError::Failed(e.to_string()))?;
    let stop = std::cell::RefCell::new(None);
    let is_cancelled = || match control.proceed() {
        Ok(()) => false,
        Err(error) => {
            *stop.borrow_mut() = Some(error);
            true
        }
    };
    let total_i = total as i64;
    let mut start = grid_start;
    let mut windows = 0;
    let mut step = 0;
    while start < total_i {
        control.proceed()?;
        let mut pcm = vec![0.0; WINDOW.min((total_i - start) as usize)];
        // The samples before sample 0 stay zeros.
        let lead = (-start).max(0) as usize;
        if lead < pcm.len() {
            source
                .read(start.max(0) as usize, &mut pcm[lead..])
                .map_err(|e| StageError::Failed(e.to_string()))?;
        }
        let saved = window(
            store,
            id,
            &dir.join("live-features"),
            segment.index,
            windows,
            &snapshot,
            &pcm,
            analyzer,
            &is_cancelled,
        );
        if let Some(reason) = stop.borrow_mut().take() {
            return Err(reason);
        }
        let saved = saved?;
        if step != 0 && step != saved.step {
            return Err(StageError::Failed("live feature hop changed".into()));
        }
        step = saved.step;
        windows += 1;
        let end = start + pcm.len() as i64;
        progress(end as f32 / total as f32);
        if end == total_i {
            break;
        }
        start += step as i64;
    }
    checkpoint::put(
        store,
        id,
        Stage::Diarize,
        &key(segment.index),
        &Index {
            total,
            step,
            windows,
            grid_start,
        },
    )?;
    restore(store, id, dir, segment.index)
}

/// True when the features of `segment` are complete and [`restore`] reads
/// them with no new analysis.
pub fn saved(store: &SharedStore, id: &str, segment: u32) -> Result<bool, StageError> {
    Ok(checkpoint::get::<Index>(store, id, Stage::Diarize, &key(segment))?.is_some())
}

/// Read the persisted features for final clustering; no native inference.
/// `None` when the features are not complete, or when a window file does
/// not read: the caller then analyses the segment again.
pub fn restore(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segment: u32,
) -> Result<Option<Analysis>, StageError> {
    let Some(index) = checkpoint::get::<Index>(store, id, Stage::Diarize, &key(segment))? else {
        return Ok(None);
    };
    let mut result = Analysis {
        total: index.total,
        step: index.step,
        grid_start: index.grid_start,
        ..Default::default()
    };
    for window in 0..index.windows {
        let saved: Window =
            checkpoint::get(store, id, Stage::Diarize, &window_key(segment, window))?
                .ok_or_else(|| StageError::Failed("live feature checkpoint is absent".into()))?;
        let path = dir.join("live-features").join(&saved.file);
        // Each file holds the analysis of one live window.
        let mut analysis = match diarize::read_analysis(&path) {
            Ok(analysis) if analysis.step == index.step && analysis.activities.len() == 1 => {
                analysis
            }
            Ok(_) => {
                log::warn!(
                    "live speaker features {} of {id} do not match their index",
                    path.display()
                );
                return Ok(None);
            }
            Err(e) => {
                log::warn!(
                    "live speaker features {} of {id} do not read: {e}",
                    path.display()
                );
                return Ok(None);
            }
        };
        for embedding in &mut analysis.embeddings {
            embedding.window = window;
        }
        result.activities.extend(analysis.activities);
        result.embeddings.extend(analysis.embeddings);
    }
    Ok(Some(result))
}
