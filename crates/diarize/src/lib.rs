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

pub use pyannote_local::segmentation::WINDOW_SAMPLES;
use pyannote_local::segmentation::{LOCAL_SPEAKERS, WindowActivity};
pub use pyannote_local::{
    Analysis, AnalysisChunk, AnalysisLayout, AudioSource, Backend, DiarizationConfig,
    DiarizeRequest, Diarizer, SpeakerBounds, SpeakerSegment, WindowEmbedding,
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
        let diarizer = Diarizer::new(config, segmentation, embedding)?;
        log::info!(
            "speaker embedding provider: {:?}; fallback: {:?}; segmentation provider: CPU",
            diarizer.embedding_backend(),
            diarizer.embedding_fallback_reason()
        );
        Ok(Self { diarizer })
    }

    pub fn embedding_backend(&self) -> Backend {
        self.diarizer.embedding_backend()
    }

    pub fn embedding_fallback_reason(&self) -> Option<&str> {
        self.diarizer.embedding_fallback_reason()
    }

    /// Deliver overlapping extraction windows without clustering or output retention.
    pub fn analyze_chunks(
        &mut self,
        audio: &mut dyn AudioSource,
        request: &DiarizeRequest<'_>,
        on_chunk: &mut dyn FnMut(AnalysisChunk) -> Result<(), pyannote_local::Error>,
    ) -> Result<AnalysisLayout, DiarizeError> {
        Ok(self.diarizer.analyze_chunks(audio, request, on_chunk)?)
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

/// Read an analysis that [`write_analysis`] wrote. A file that `analyze()`
/// cannot have made is `InvalidData`: each count must fit in the bytes that
/// are left, and each index must point into the analysis.
pub fn read_analysis(path: &Path) -> std::io::Result<Analysis> {
    let file = std::fs::File::open(path)?;
    let left = file.metadata()?.len();
    let mut input = AnalysisInput {
        reader: BufReader::new(file),
        left,
    };
    if &input.bytes::<8>()? != MAGIC {
        return Err(invalid("not a Longhand diarization analysis"));
    }
    let total = input.u64()?;
    let step = input.u64()?;
    // Each window holds at least its frame count.
    let windows = input.u64()?;
    let windows = input.count(windows, 4)?;
    let step = usize::try_from(step).map_err(|_| invalid("the analysis hop is not valid"))?;
    if windows > 0 && (step == 0 || step > WINDOW_SAMPLES) {
        return Err(invalid("the analysis hop is not valid"));
    }
    // `analyze()` makes no window for no samples, and else one window more
    // than the hops that the samples after the first window need.
    let cover = match windows {
        0 => Some(0),
        _ => (windows - 1)
            .checked_mul(step)
            .and_then(|hops| hops.checked_add(WINDOW_SAMPLES)),
    };
    let total = usize::try_from(total)
        .ok()
        .filter(|&total| cover.is_some_and(|cover| total <= cover))
        .ok_or_else(|| invalid("the analysis is longer than its windows"))?;
    let mut activities = Vec::with_capacity(windows);
    for _ in 0..windows {
        let n = input.u32()?;
        let n = input.count(u64::from(n), 1)?;
        let bytes = input.vec(n)?;
        activities.push(WindowActivity {
            frames: bytes
                .iter()
                .map(|b| [b & 1 != 0, b & 2 != 0, b & 4 != 0])
                .collect(),
        });
    }
    // Each embedding holds at least its window, its flags and its dimension.
    let count = input.u64()?;
    let count = input.count(count, 10)?;
    let mut embeddings = Vec::with_capacity(count);
    for _ in 0..count {
        let window = input.u32()? as usize;
        let [local_speaker, anchor] = input.bytes::<2>()?;
        if window >= windows {
            return Err(invalid("an embedding is past the analysis windows"));
        }
        if usize::from(local_speaker) >= LOCAL_SPEAKERS {
            return Err(invalid("an embedding has an unknown local speaker"));
        }
        let dim = input.u32()?;
        let dim = input.count(u64::from(dim), 4)?;
        let raw = input.vec(dim * 4)?;
        embeddings.push(WindowEmbedding {
            window,
            local_speaker: usize::from(local_speaker),
            anchor: anchor != 0,
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

fn invalid(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.to_owned())
}

/// The reader of an analysis file, with the count of bytes that are left.
struct AnalysisInput {
    reader: BufReader<std::fs::File>,
    left: u64,
}

impl AnalysisInput {
    fn bytes<const N: usize>(&mut self) -> std::io::Result<[u8; N]> {
        let mut bytes = [0u8; N];
        self.reader.read_exact(&mut bytes)?;
        self.left = self.left.saturating_sub(N as u64);
        Ok(bytes)
    }

    fn u64(&mut self) -> std::io::Result<u64> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }

    fn u32(&mut self) -> std::io::Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }

    /// `count` items of at least `size` bytes each, when they fit in the
    /// bytes that are left.
    fn count(&self, count: u64, size: u64) -> std::io::Result<usize> {
        count
            .checked_mul(size)
            .filter(|&bytes| bytes <= self.left)
            .and_then(|_| usize::try_from(count).ok())
            .ok_or_else(|| invalid("a count in the analysis is larger than the file"))
    }

    /// The next `n` bytes. [`Self::count`] checks `n` first.
    fn vec(&mut self, n: usize) -> std::io::Result<Vec<u8>> {
        let mut bytes = vec![0u8; n];
        self.reader.read_exact(&mut bytes)?;
        self.left = self.left.saturating_sub(n as u64);
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analysis() -> Analysis {
        Analysis {
            total: 180_000,
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

    // Byte offsets in the file of `analysis()`.
    const TOTAL: usize = 8;
    const STEP: usize = 16;
    const WINDOWS: usize = 24;
    const FIRST_FRAMES: usize = 32;
    const COUNT: usize = 43;
    const EMBEDDING_WINDOW: usize = 51;
    const LOCAL_SPEAKER: usize = 55;
    const DIM: usize = 57;

    /// Write `analysis()`, put `bytes` at `at`, and read the file again.
    fn read_damaged(at: usize, bytes: &[u8]) -> std::io::Result<Analysis> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("segment-01.diar");
        write_analysis(&path, &analysis()).unwrap();
        let mut raw = std::fs::read(&path).unwrap();
        raw[at..at + bytes.len()].copy_from_slice(bytes);
        std::fs::write(&path, raw).unwrap();
        read_analysis(&path)
    }

    fn assert_invalid(result: std::io::Result<Analysis>) {
        let error = result.expect_err("a damaged file must not read");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{error}");
    }

    #[test]
    fn a_count_larger_than_the_file_is_refused() {
        assert_invalid(read_damaged(WINDOWS, &u64::MAX.to_le_bytes()));
        assert_invalid(read_damaged(FIRST_FRAMES, &u32::MAX.to_le_bytes()));
        assert_invalid(read_damaged(COUNT, &u64::MAX.to_le_bytes()));
        assert_invalid(read_damaged(DIM, &u32::MAX.to_le_bytes()));
    }

    #[test]
    fn a_truncated_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("segment-01.diar");
        write_analysis(&path, &analysis()).unwrap();
        let raw = std::fs::read(&path).unwrap();
        for len in [12, WINDOWS + 4, FIRST_FRAMES + 3, COUNT + 2, raw.len() - 1] {
            std::fs::write(&path, &raw[..len]).unwrap();
            assert!(read_analysis(&path).is_err(), "{len} bytes");
        }
    }

    #[test]
    fn a_local_speaker_past_the_window_speakers_is_refused() {
        assert_invalid(read_damaged(LOCAL_SPEAKER, &[LOCAL_SPEAKERS as u8]));
    }

    #[test]
    fn an_embedding_past_the_windows_is_refused() {
        assert_invalid(read_damaged(EMBEDDING_WINDOW, &2u32.to_le_bytes()));
    }

    #[test]
    fn a_total_past_the_windows_is_refused() {
        assert_invalid(read_damaged(TOTAL, &u64::MAX.to_le_bytes()));
        let cover = 31_860 + WINDOW_SAMPLES as u64;
        assert_invalid(read_damaged(TOTAL, &(cover + 1).to_le_bytes()));
        assert!(read_damaged(TOTAL, &cover.to_le_bytes()).is_ok());
    }

    #[test]
    fn a_step_that_analyze_never_writes_is_refused() {
        assert_invalid(read_damaged(STEP, &0u64.to_le_bytes()));
        assert_invalid(read_damaged(STEP, &u64::MAX.to_le_bytes()));
    }

    #[test]
    fn missing_weights_are_a_typed_error() {
        let err = OnnxAnalyzer::load(Path::new("/nope/seg.onnx"), Path::new("/nope/emb.onnx"))
            .err()
            .unwrap();
        assert!(matches!(err, DiarizeError::MissingWeights(_)));
    }
}
