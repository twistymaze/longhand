//! Session diarization (KTD7) on the borrowed `pyannote-local` stack.
//!
//! Each segment is analysed on its own (segmentation and embeddings, the
//! expensive part), so the work checkpoints per segment. The analyses are
//! then clustered together across the whole session, so a speaker label
//! means the same voice on both sides of a gap. Clustering runs in
//! consecutive windows of at most [`CLUSTER_WINDOW_LIMIT`] analysis windows
//! (120 minutes at the native 2 s hop); numbering continues across windows,
//! so one person may carry two labels but a label never covers two people.

pub mod turns;
pub mod wav_source;

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use pyannote_local::segmentation::WindowActivity;
pub use pyannote_local::{
    Analysis, AudioSource, DiarizationConfig, DiarizeRequest, Diarizer, SpeakerBounds,
    SpeakerSegment, WindowEmbedding,
};

pub use turns::{Turn, smooth, turn_cuts};
pub use wav_source::WavAudioSource;

/// Analysis windows per clustering window: the U1 cap, which holds the
/// native 2 s hop to 120 minutes (KTD7).
pub const CLUSTER_WINDOW_LIMIT: usize = 3612;

#[derive(Debug, thiserror::Error)]
pub enum DiarizeError {
    #[error("the diarization weights are not downloaded yet ({0})")]
    MissingWeights(String),
    #[error("{0}")]
    Format(String),
    #[error(transparent)]
    Engine(#[from] pyannote_local::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What processing needs from a diarizer, so the stage is testable without
/// weights on disk.
pub trait SpeakerAnalyzer: Send {
    fn analyze(
        &mut self,
        audio: &mut dyn AudioSource,
        request: &DiarizeRequest<'_>,
    ) -> Result<Analysis, DiarizeError>;
}

/// The real analyzer: both ONNX models, loaded from the models directory.
pub struct OnnxAnalyzer {
    diarizer: Diarizer,
}

impl OnnxAnalyzer {
    pub fn load(segmentation: &Path, embedding: &Path) -> Result<Self, DiarizeError> {
        for path in [segmentation, embedding] {
            if !path.exists() {
                return Err(DiarizeError::MissingWeights(path.display().to_string()));
            }
        }
        let config = DiarizationConfig {
            // Analysis always runs at the native hop; the cap bounds each
            // clustering window instead (KTD7).
            max_windows: usize::MAX,
            ..DiarizationConfig::default()
        };
        Ok(Self {
            diarizer: Diarizer::new(config, segmentation, embedding)?,
        })
    }
}

impl SpeakerAnalyzer for OnnxAnalyzer {
    fn analyze(
        &mut self,
        audio: &mut dyn AudioSource,
        request: &DiarizeRequest<'_>,
    ) -> Result<Analysis, DiarizeError> {
        Ok(self.diarizer.analyze(audio, request)?)
    }
}

/// Cluster a session's analyses and return each one's speaker segments, in
/// the analysis's own seconds, labelled with session-wide speaker indices.
/// `bounds` applies within each clustering window, never to the session as
/// one (U1: a global exact count merges speakers inside a window).
pub fn cluster_session(analyses: &[&Analysis], bounds: SpeakerBounds) -> Vec<Vec<SpeakerSegment>> {
    cluster_session_in_windows(analyses, bounds, CLUSTER_WINDOW_LIMIT)
}

/// [`cluster_session`] with the clustering window's size in analysis
/// windows as a parameter.
pub fn cluster_session_in_windows(
    analyses: &[&Analysis],
    bounds: SpeakerBounds,
    window_limit: usize,
) -> Vec<Vec<SpeakerSegment>> {
    let config = DiarizationConfig::default();
    // (analysis, embedding index) in timeline order, grouped into windows.
    let mut groups: Vec<Vec<(usize, usize)>> = vec![Vec::new()];
    let mut windows_in_group = 0usize;
    for (a, analysis) in analyses.iter().enumerate() {
        let mut next = 0;
        for window in 0..analysis.activities.len() {
            if windows_in_group == window_limit.max(1) {
                groups.push(Vec::new());
                windows_in_group = 0;
            }
            windows_in_group += 1;
            while next < analysis.embeddings.len() && analysis.embeddings[next].window == window {
                groups.last_mut().expect("never empty").push((a, next));
                next += 1;
            }
        }
    }

    let mut labels: Vec<Vec<usize>> = analyses
        .iter()
        .map(|a| vec![0; a.embeddings.len()])
        .collect();
    let mut speakers = 0usize;
    for group in groups.iter().filter(|g| !g.is_empty()) {
        let step = analyses[group[0].0].step;
        let items: Vec<&WindowEmbedding> = group
            .iter()
            .map(|&(a, e)| &analyses[a].embeddings[e])
            .collect();
        let (local, centroids) = pyannote_local::cluster(&config, &items, step, bounds);
        for (&(a, e), label) in group.iter().zip(local) {
            labels[a][e] = speakers + label;
        }
        speakers += centroids.len();
    }

    analyses
        .iter()
        .zip(&labels)
        .map(|(analysis, labels)| pyannote_local::reconstruct(&config, analysis, labels, speakers))
        .collect()
}

// ------------------------------------------------------------ analysis file

const MAGIC: &[u8; 8] = b"LHDIAR01";

/// Where a segment's analysis is kept, beside its processing audio.
pub fn analysis_path(proc_wav: &Path) -> PathBuf {
    proc_wav.with_extension("diar")
}

/// Write an analysis durably: a compact binary file rather than a JSON
/// checkpoint, since a four-hour segment holds millions of frame flags.
pub fn write_analysis(path: &Path, analysis: &Analysis) -> std::io::Result<()> {
    let temp = path.with_extension("diar.tmp");
    {
        let mut out = BufWriter::new(std::fs::File::create(&temp)?);
        out.write_all(MAGIC)?;
        out.write_all(&(analysis.total as u64).to_le_bytes())?;
        out.write_all(&(analysis.step as u64).to_le_bytes())?;
        out.write_all(&(analysis.activities.len() as u64).to_le_bytes())?;
        for activity in &analysis.activities {
            out.write_all(&(activity.frames.len() as u32).to_le_bytes())?;
            for frame in &activity.frames {
                let bits = frame
                    .iter()
                    .enumerate()
                    .fold(0u8, |acc, (i, &on)| acc | (u8::from(on) << i));
                out.write_all(&[bits])?;
            }
        }
        out.write_all(&(analysis.embeddings.len() as u64).to_le_bytes())?;
        for item in &analysis.embeddings {
            out.write_all(&(item.window as u32).to_le_bytes())?;
            out.write_all(&[item.local_speaker as u8, u8::from(item.anchor)])?;
            out.write_all(&(item.embedding.len() as u32).to_le_bytes())?;
            for v in &item.embedding {
                out.write_all(&v.to_le_bytes())?;
            }
        }
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    }
    std::fs::rename(temp, path)
}

pub fn read_analysis(path: &Path) -> std::io::Result<Analysis> {
    let mut input = BufReader::new(std::fs::File::open(path)?);
    let mut magic = [0u8; 8];
    input.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a Longhand diarization analysis",
        ));
    }
    let u64_ = |r: &mut BufReader<std::fs::File>| -> std::io::Result<u64> {
        let mut b = [0u8; 8];
        r.read_exact(&mut b)?;
        Ok(u64::from_le_bytes(b))
    };
    let u32_ = |r: &mut BufReader<std::fs::File>| -> std::io::Result<u32> {
        let mut b = [0u8; 4];
        r.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    };
    let total = u64_(&mut input)? as usize;
    let step = u64_(&mut input)? as usize;
    let windows = u64_(&mut input)? as usize;
    let mut activities = Vec::with_capacity(windows);
    for _ in 0..windows {
        let n = u32_(&mut input)? as usize;
        let mut bytes = vec![0u8; n];
        input.read_exact(&mut bytes)?;
        activities.push(WindowActivity {
            frames: bytes
                .iter()
                .map(|b| [b & 1 != 0, b & 2 != 0, b & 4 != 0])
                .collect(),
        });
    }
    let count = u64_(&mut input)? as usize;
    let mut embeddings = Vec::with_capacity(count);
    for _ in 0..count {
        let window = u32_(&mut input)? as usize;
        let mut flags = [0u8; 2];
        input.read_exact(&mut flags)?;
        let dim = u32_(&mut input)? as usize;
        let mut raw = vec![0u8; dim * 4];
        input.read_exact(&mut raw)?;
        embeddings.push(WindowEmbedding {
            window,
            local_speaker: flags[0] as usize,
            anchor: flags[1] != 0,
            embedding: raw
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().expect("four bytes")))
                .collect(),
        });
    }
    Ok(Analysis {
        total,
        step,
        activities,
        embeddings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analysis() -> Analysis {
        Analysis {
            total: 480_000,
            step: 31_860,
            activities: vec![
                WindowActivity {
                    frames: vec![[true, false, true], [false, false, false]],
                },
                WindowActivity {
                    frames: vec![[false, true, false]],
                },
            ],
            embeddings: vec![WindowEmbedding {
                window: 1,
                local_speaker: 2,
                embedding: vec![0.25, -1.5, 3.0],
                anchor: true,
            }],
        }
    }

    #[test]
    fn an_analysis_round_trips_through_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("segment-01.diar");
        write_analysis(&path, &analysis()).unwrap();
        assert_eq!(read_analysis(&path).unwrap(), analysis());
        assert!(!dir.path().join("segment-01.diar.tmp").exists());
    }

    #[test]
    fn a_foreign_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.diar");
        std::fs::write(&path, b"not an analysis at all").unwrap();
        assert!(read_analysis(&path).is_err());
    }

    #[test]
    fn missing_weights_are_a_typed_error() {
        let err = OnnxAnalyzer::load(Path::new("/nope/seg.onnx"), Path::new("/nope/emb.onnx"))
            .err()
            .unwrap();
        assert!(matches!(err, DiarizeError::MissingWeights(_)));
    }
}
