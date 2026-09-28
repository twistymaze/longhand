//! Offline speaker diarization: sliding-window segmentation, masked speaker
//! embeddings, agglomerative clustering, then reconstruction of who spoke
//! when. Mirrors the structure of the pyannote.audio 3.1 pipeline, tuned for
//! CPU batch use after a recording ends.
//!
//! Longhand's copy (see PROVENANCE.md) splits the pipeline at its seam:
//! [`Diarizer::analyze`] runs segmentation and embedding over one audio
//! source, and [`cluster`] plus [`reconstruct`] turn any set of analyses
//! into speaker turns, so a session's segments can be analysed one at a time
//! (and checkpointed) and then clustered together. The known-voiceprint
//! steering path is removed.

use std::path::Path;

use crate::clustering::{self, SpeakerBounds};
use crate::segmentation::{
    FRAME_SIZE, LOCAL_SPEAKERS, SAMPLE_RATE, Segmenter, WINDOW_SAMPLES, WindowActivity,
    frame_start_sample, frames_for_samples,
};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DiarizationConfig {
    /// Hop between consecutive 10 s segmentation windows.
    pub step_seconds: f32,
    /// Clean (non-overlapped) speech a local speaker needs inside a window
    /// before its embedding is trusted as a clustering anchor.
    pub min_embedding_seconds: f32,
    /// Speech a local speaker needs inside a window to be embedded at all;
    /// shorter blips are attached to the nearest cluster instead of dropped.
    pub min_activity_seconds: f32,
    /// Centroid-linkage cut on unit-normalised embeddings (euclidean).
    pub clustering_threshold: f32,
    /// Clusters backed by fewer seconds of anchor windows are absorbed by
    /// their neighbours. Expressed in seconds so it stays meaningful when the
    /// step changes (pyannote's 12 anchors at a 1 s step).
    pub min_cluster_seconds: f32,
    /// Drop speaker turns shorter than this after reconstruction.
    pub min_duration_on: f32,
    /// Bridge same-speaker gaps shorter than this after reconstruction.
    pub min_duration_off: f32,
    /// Grow the step on long recordings so compute stays bounded.
    pub max_windows: usize,
}

impl Default for DiarizationConfig {
    fn default() -> Self {
        Self {
            // 2 s keeps every frame under ~5 overlapping windows so majority
            // voting suppresses per-window false speech; larger steps trade
            // accuracy for speed roughly linearly (see eval/PROGRAM.md).
            step_seconds: 2.0,
            min_embedding_seconds: 1.0,
            min_activity_seconds: 0.3,
            clustering_threshold: 0.7045,
            min_cluster_seconds: 12.0,
            min_duration_on: 0.0,
            min_duration_off: 0.0,
            // Upstream is 1,800, which stretches the hop past 60 minutes.
            // 3,612 holds the native 2 s hop to 120 minutes (KTD7, U1).
            max_windows: 3612,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeakerSegment {
    pub start: f64,
    pub end: f64,
    pub speaker: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiarizedSpeaker {
    pub index: usize,
    pub centroid: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diarization {
    pub segments: Vec<SpeakerSegment>,
    pub speakers: Vec<DiarizedSpeaker>,
}

impl Diarization {
    pub fn speaker_count(&self) -> usize {
        self.speakers.len()
    }
}

/// Random-access mono 16 kHz audio. Implemented for in-memory buffers; the
/// batch pipeline implements it over a WAV file so long recordings do not
/// need to be resident.
pub trait AudioSource {
    fn sample_count(&mut self) -> Result<usize, crate::Error>;
    /// Fills `out` with samples starting at `start`, zero-padding past the end.
    fn read(&mut self, start: usize, out: &mut [f32]) -> Result<(), crate::Error>;
}

impl AudioSource for &[f32] {
    fn sample_count(&mut self) -> Result<usize, crate::Error> {
        Ok(<[f32]>::len(self))
    }

    fn read(&mut self, start: usize, out: &mut [f32]) -> Result<(), crate::Error> {
        let end = (start + out.len()).min(<[f32]>::len(self));
        let copied = end.saturating_sub(start);
        out[..copied].copy_from_slice(&self[start..end]);
        out[copied..].fill(0.0);
        Ok(())
    }
}

impl AudioSource for Vec<f32> {
    fn sample_count(&mut self) -> Result<usize, crate::Error> {
        Ok(Vec::len(self))
    }

    fn read(&mut self, start: usize, out: &mut [f32]) -> Result<(), crate::Error> {
        self.as_slice().read(start, out)
    }
}

#[derive(Default)]
pub struct DiarizeRequest<'a> {
    pub bounds: SpeakerBounds,
    pub is_cancelled: Option<&'a dyn Fn() -> bool>,
    /// Called after each window with the fraction of windows processed, so
    /// callers can keep liveness signals flowing during long recordings.
    pub on_progress: Option<&'a dyn Fn(f32)>,
}

/// One local speaker's embedding inside one segmentation window.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowEmbedding {
    pub window: usize,
    pub local_speaker: usize,
    pub embedding: Vec<f32>,
    /// Enough clean speech to be trusted as a clustering anchor.
    pub anchor: bool,
}

/// Segmentation and embeddings of one audio source: everything clustering
/// needs, and the expensive part of diarization.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Analysis {
    /// Samples in the source.
    pub total: usize,
    /// Hop between windows, in samples.
    pub step: usize,
    pub activities: Vec<WindowActivity>,
    pub embeddings: Vec<WindowEmbedding>,
}

/// One completed extraction window. Adjacent windows overlap by
/// `WINDOW_SAMPLES - layout.step` samples. No speaker labels are assigned here.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisChunk {
    pub index: usize,
    pub start_sample: usize,
    /// Source samples in this window, excluding zero padding.
    pub valid_samples: usize,
    pub activity: WindowActivity,
    pub embeddings: Vec<WindowEmbedding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisLayout {
    pub total: usize,
    pub step: usize,
    pub windows: usize,
}

fn analysis_layout(config: &DiarizationConfig, total: usize) -> AnalysisLayout {
    let step = step_samples(config, total);
    let windows = if total == 0 {
        0
    } else if total <= WINDOW_SAMPLES {
        1
    } else {
        (total - WINDOW_SAMPLES).div_ceil(step) + 1
    };
    AnalysisLayout {
        total,
        step,
        windows,
    }
}

pub struct Diarizer {
    segmenter: Segmenter,
    embedder: anlg_embedding::EmbeddingExtractor,
    config: DiarizationConfig,
}

impl Diarizer {
    /// Load both models from the models directory (R28).
    pub fn new(
        config: DiarizationConfig,
        segmentation_weights: &Path,
        embedding_weights: &Path,
    ) -> Result<Self, crate::Error> {
        Ok(Self {
            segmenter: Segmenter::from_path(segmentation_weights)?,
            embedder: anlg_embedding::EmbeddingExtractor::from_path(embedding_weights)?,
            config,
        })
    }

    pub fn config(&self) -> &DiarizationConfig {
        &self.config
    }

    /// Segment and embed one source.
    pub fn analyze(
        &mut self,
        audio: &mut dyn AudioSource,
        request: &DiarizeRequest<'_>,
    ) -> Result<Analysis, crate::Error> {
        let mut analysis = Analysis::default();
        let layout = self.analyze_chunks(audio, request, &mut |chunk| {
            analysis.activities.push(chunk.activity);
            analysis.embeddings.extend(chunk.embeddings);
            Ok(())
        })?;
        analysis.total = layout.total;
        analysis.step = if layout.windows == 0 { 0 } else { layout.step };
        Ok(analysis)
    }

    pub fn embedding_backend(&self) -> anlg_onnx::Backend {
        self.embedder.backend()
    }

    pub fn embedding_fallback_reason(&self) -> Option<&str> {
        self.embedder.fallback_reason()
    }

    /// Extract and deliver each complete window without retaining earlier outputs.
    /// The callback can persist a chunk or stop extraction with an error.
    /// Clustering is a separate operation. Input and output window indices are
    /// local to this source; a caller must retain its canonical source offset.
    pub fn analyze_chunks(
        &mut self,
        audio: &mut dyn AudioSource,
        request: &DiarizeRequest<'_>,
        on_chunk: &mut dyn FnMut(AnalysisChunk) -> Result<(), crate::Error>,
    ) -> Result<AnalysisLayout, crate::Error> {
        let layout = analysis_layout(&self.config, audio.sample_count()?);
        if layout.windows == 0 {
            return Ok(layout);
        }
        let AnalysisLayout {
            total,
            step,
            windows: window_count,
        } = layout;
        let min_anchor_frames = seconds_to_frames(self.config.min_embedding_seconds).max(1);
        let min_activity_frames = seconds_to_frames(self.config.min_activity_seconds).max(1);

        let mut window = vec![0.0f32; WINDOW_SAMPLES];
        let mut mask = vec![0.0f32; WINDOW_SAMPLES];

        for index in 0..window_count {
            if request.is_cancelled.is_some_and(|cancelled| cancelled()) {
                return Err(crate::Error::Cancelled);
            }
            let start = index * step;
            audio.read(start, &mut window)?;
            let valid = WINDOW_SAMPLES.min(total - start);
            let mut activity = self.segmenter.run_window(&window)?;
            activity
                .frames
                .truncate(frames_for_samples(valid).min(activity.frames.len()));

            let mut embeddings = Vec::with_capacity(LOCAL_SPEAKERS);
            for local_speaker in 0..LOCAL_SPEAKERS {
                let active = activity.active_frames(local_speaker);
                if active < min_activity_frames {
                    continue;
                }
                let clean = activity.clean_frames(local_speaker);
                let anchor = clean >= min_anchor_frames;
                fill_sample_mask(&activity, local_speaker, anchor, &mut mask);
                match self.embedder.compute_with_mask_optional(&window, &mask)? {
                    Some(embedding) => embeddings.push(WindowEmbedding {
                        window: index,
                        local_speaker,
                        embedding,
                        anchor,
                    }),
                    None => continue,
                }
            }
            on_chunk(AnalysisChunk {
                index,
                start_sample: start,
                valid_samples: valid,
                activity,
                embeddings,
            })?;
            if let Some(on_progress) = request.on_progress {
                on_progress((index + 1) as f32 / window_count as f32);
            }
        }

        Ok(layout)
    }

    /// Diarize one source end to end, as upstream does.
    pub fn diarize(
        &mut self,
        audio: &mut dyn AudioSource,
        request: &DiarizeRequest<'_>,
    ) -> Result<Diarization, crate::Error> {
        let analysis = self.analyze(audio, request)?;
        if analysis.embeddings.is_empty() {
            return Ok(Diarization::default());
        }
        let items: Vec<&WindowEmbedding> = analysis.embeddings.iter().collect();
        let (labels, centroids) = cluster(&self.config, &items, analysis.step, request.bounds);
        let speakers: Vec<DiarizedSpeaker> = centroids
            .into_iter()
            .enumerate()
            .map(|(index, centroid)| DiarizedSpeaker { index, centroid })
            .collect();
        let segments = reconstruct(&self.config, &analysis, &labels, speakers.len());
        let speakers = speakers
            .into_iter()
            .filter(|speaker| {
                segments
                    .iter()
                    .any(|segment| segment.speaker == speaker.index)
            })
            .collect();
        let mut diarization = Diarization { segments, speakers };
        renumber_by_first_appearance(&mut diarization);
        Ok(diarization)
    }
}

fn step_samples(config: &DiarizationConfig, total: usize) -> usize {
    let requested = seconds_to_frames(config.step_seconds).max(1);
    let max_windows = config.max_windows.max(2);
    let needed = if total > WINDOW_SAMPLES {
        (total - WINDOW_SAMPLES).div_ceil((max_windows - 1).saturating_mul(FRAME_SIZE))
    } else {
        0
    };
    let frames = requested
        .max(needed)
        .min(frames_for_samples(WINDOW_SAMPLES).max(1));
    frames * FRAME_SIZE
}

fn min_cluster_size(config: &DiarizationConfig, step: usize) -> usize {
    let step_seconds = step as f32 / SAMPLE_RATE as f32;
    (config.min_cluster_seconds / step_seconds.max(f32::EPSILON)).ceil() as usize
}

/// Cluster embeddings drawn from one or more analyses at hop `step`. Returns
/// one label per item and the cluster centroids. Items that are not anchors
/// take the nearest anchor centroid.
pub fn cluster(
    config: &DiarizationConfig,
    embeddings: &[&WindowEmbedding],
    step: usize,
    bounds: SpeakerBounds,
) -> (Vec<usize>, Vec<Vec<f32>>) {
    if embeddings.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let anchors: Vec<usize> = embeddings
        .iter()
        .enumerate()
        .filter(|(_, item)| item.anchor)
        .map(|(index, _)| index)
        .collect();
    let anchors = if anchors.is_empty() {
        (0..embeddings.len()).collect()
    } else {
        anchors
    };
    let anchor_vectors: Vec<Vec<f32>> = anchors
        .iter()
        .map(|index| embeddings[*index].embedding.clone())
        .collect();
    let anchor_labels = clustering::cluster(
        &anchor_vectors,
        config.clustering_threshold,
        min_cluster_size(config, step),
        bounds,
    );
    let centroids = clustering::centroids(&anchor_vectors, &anchor_labels);

    let mut labels = vec![usize::MAX; embeddings.len()];
    for (anchor, label) in anchors.iter().zip(&anchor_labels) {
        labels[*anchor] = *label;
    }
    for (index, item) in embeddings.iter().enumerate() {
        if labels[index] == usize::MAX {
            labels[index] = clustering::nearest_centroid(&item.embedding, &centroids).unwrap_or(0);
        }
    }
    (labels, centroids)
}

/// Votes every window's local-speaker activity onto the global frame grid
/// under its cluster label and turns the majority into speaker turns.
/// `labels` holds one label per `analysis.embeddings` entry, each below
/// `speaker_count`. Times are seconds from the start of the source.
pub fn reconstruct(
    config: &DiarizationConfig,
    analysis: &Analysis,
    labels: &[usize],
    speaker_count: usize,
) -> Vec<SpeakerSegment> {
    let (total, step) = (analysis.total, analysis.step);
    if total == 0 || speaker_count == 0 {
        return Vec::new();
    }
    let frame_count = frames_for_samples(total.max(WINDOW_SAMPLES));
    let mut votes = vec![vec![0u16; frame_count]; speaker_count];
    let mut coverage = vec![0u16; frame_count];

    for (window, activity) in analysis.activities.iter().enumerate() {
        let offset = window * step / FRAME_SIZE;
        for frame in 0..activity.frames.len() {
            if let Some(slot) = coverage.get_mut(offset + frame) {
                *slot += 1;
            }
        }
    }
    for (item, &label) in analysis.embeddings.iter().zip(labels) {
        let activity = &analysis.activities[item.window];
        let offset = item.window * step / FRAME_SIZE;
        for (frame, active) in activity.frames.iter().enumerate() {
            if active[item.local_speaker]
                && let Some(slot) = votes[label].get_mut(offset + frame)
            {
                *slot += 1;
            }
        }
    }

    let mut segments = Vec::new();
    for (speaker, track) in votes.iter().enumerate() {
        let active: Vec<bool> = track
            .iter()
            .zip(&coverage)
            .map(|(votes, coverage)| *coverage > 0 && u32::from(*votes) * 2 >= u32::from(*coverage))
            .collect();
        segments.extend(
            frames_to_segments(
                &active,
                speaker,
                config.min_duration_on,
                config.min_duration_off,
            )
            .into_iter()
            .map(|mut segment| {
                segment.end = segment.end.min(total as f64 / SAMPLE_RATE as f64);
                segment
            })
            .filter(|segment| segment.end > segment.start),
        );
    }
    segments.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.speaker.cmp(&b.speaker)));
    segments
}

fn seconds_to_frames(seconds: f32) -> usize {
    ((seconds.max(0.0) * SAMPLE_RATE as f32) / FRAME_SIZE as f32).round() as usize
}

fn frame_time(frame: usize) -> f64 {
    frame_start_sample(frame) as f64 / SAMPLE_RATE as f64
}

fn fill_sample_mask(activity: &WindowActivity, speaker: usize, clean_only: bool, mask: &mut [f32]) {
    mask.fill(0.0);
    for (frame, active) in activity.frames.iter().enumerate() {
        let keep =
            active[speaker] && (!clean_only || active.iter().filter(|value| **value).count() == 1);
        if !keep {
            continue;
        }
        let start = if frame == 0 {
            0
        } else {
            frame_start_sample(frame)
        };
        let end = frame_start_sample(frame + 1).min(mask.len());
        if start < end {
            mask[start..end].fill(1.0);
        }
    }
}

fn frames_to_segments(
    active: &[bool],
    speaker: usize,
    min_duration_on: f32,
    min_duration_off: f32,
) -> Vec<SpeakerSegment> {
    let mut segments: Vec<SpeakerSegment> = Vec::new();
    let mut run_start: Option<usize> = None;
    for (frame, is_active) in active
        .iter()
        .copied()
        .chain(std::iter::once(false))
        .enumerate()
    {
        match (run_start, is_active) {
            (None, true) => run_start = Some(frame),
            (Some(start), false) => {
                let segment = SpeakerSegment {
                    start: frame_time(start),
                    end: frame_time(frame),
                    speaker,
                };
                match segments.last_mut() {
                    Some(previous)
                        if segment.start - previous.end < f64::from(min_duration_off) =>
                    {
                        previous.end = segment.end;
                    }
                    _ => segments.push(segment),
                }
                run_start = None;
            }
            _ => {}
        }
    }
    segments.retain(|segment| segment.end - segment.start >= f64::from(min_duration_on));
    segments
}

fn renumber_by_first_appearance(diarization: &mut Diarization) {
    let mut order: Vec<usize> = Vec::new();
    for segment in &diarization.segments {
        if !order.contains(&segment.speaker) {
            order.push(segment.speaker);
        }
    }
    let position = |speaker: usize| order.iter().position(|s| *s == speaker).unwrap_or(0);
    for segment in &mut diarization.segments {
        segment.speaker = position(segment.speaker);
    }
    for speaker in &mut diarization.speakers {
        speaker.index = position(speaker.index);
    }
    diarization.speakers.sort_by_key(|speaker| speaker.index);
}

/// Speaker for each `(start, end)` word: the speaker with the largest overlap,
/// falling back to the nearest turn when the word lies in a gap. `None` only
/// when there are no segments at all.
pub fn assign_words(words: &[(f64, f64)], segments: &[SpeakerSegment]) -> Vec<Option<usize>> {
    words
        .iter()
        .map(|&(start, end)| {
            if segments.is_empty() {
                return None;
            }
            let mut best_overlap: Option<(f64, usize)> = None;
            let mut nearest: Option<(f64, usize)> = None;
            for segment in segments {
                let overlap = segment.end.min(end) - segment.start.max(start);
                if overlap > 0.0 {
                    if best_overlap.is_none_or(|(best, _)| overlap > best) {
                        best_overlap = Some((overlap, segment.speaker));
                    }
                } else {
                    let distance = if segment.end <= start {
                        start - segment.end
                    } else {
                        segment.start - end
                    };
                    if nearest.is_none_or(|(best, _)| distance < best) {
                        nearest = Some((distance, segment.speaker));
                    }
                }
            }
            best_overlap.or(nearest).map(|(_, speaker)| speaker)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start: f64, end: f64, speaker: usize) -> SpeakerSegment {
        SpeakerSegment {
            start,
            end,
            speaker,
        }
    }

    #[test]
    fn assigns_words_by_overlap_then_proximity() {
        let segments = vec![segment(0.0, 2.0, 0), segment(2.5, 5.0, 1)];
        let words = [(0.2, 0.6), (1.8, 2.7), (2.1, 2.3), (6.0, 6.4)];
        assert_eq!(
            assign_words(&words, &segments),
            vec![Some(0), Some(1), Some(0), Some(1)]
        );
        assert_eq!(assign_words(&words, &[]), vec![None; 4]);
    }

    #[test]
    fn frames_to_segments_bridges_and_filters() {
        let mut active = vec![true; 100];
        active[40..42].fill(false);
        active[70..].fill(false);
        active[95] = true;

        let raw = frames_to_segments(&active, 0, 0.0, 0.0);
        assert_eq!(raw.len(), 3);

        let bridged = frames_to_segments(&active, 0, 0.1, 0.1);
        assert_eq!(bridged.len(), 1);
        assert!((bridged[0].start - frame_time(0)).abs() < 1e-9);
        assert!((bridged[0].end - frame_time(70)).abs() < 1e-9);
    }

    #[test]
    fn renumbering_follows_first_appearance() {
        let mut diarization = Diarization {
            segments: vec![
                segment(5.0, 6.0, 0),
                segment(0.0, 1.0, 2),
                segment(2.0, 3.0, 1),
            ],
            speakers: (0..3)
                .map(|index| DiarizedSpeaker {
                    index,
                    centroid: vec![],
                })
                .collect(),
        };
        diarization
            .segments
            .sort_by(|a, b| a.start.total_cmp(&b.start));
        renumber_by_first_appearance(&mut diarization);
        assert_eq!(
            diarization
                .segments
                .iter()
                .map(|s| s.speaker)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn sample_mask_covers_active_frames_only() {
        let frames = vec![
            [true, false, false],
            [true, true, false],
            [false, true, false],
        ];
        let activity = WindowActivity { frames };
        let mut mask = vec![0.0f32; frame_start_sample(3)];

        fill_sample_mask(&activity, 0, false, &mut mask);
        assert_eq!(mask[0], 1.0);
        assert_eq!(mask[frame_start_sample(1)], 1.0);
        assert_eq!(mask[frame_start_sample(2)], 0.0);

        fill_sample_mask(&activity, 0, true, &mut mask);
        assert_eq!(mask[0], 1.0);
        assert_eq!(mask[frame_start_sample(1)], 0.0);
    }

    #[test]
    fn the_hop_holds_at_two_seconds_to_120_minutes_and_stretches_after() {
        let config = DiarizationConfig::default();
        let at = |minutes: usize| {
            step_samples(&config, minutes * 60 * SAMPLE_RATE as usize) as f64 / SAMPLE_RATE as f64
        };
        assert!((at(120) - 2.0).abs() < 0.02, "{}", at(120));
        assert!(at(180) > 2.5);
    }

    #[test]
    fn reconstruct_votes_one_speaker_across_overlapping_windows() {
        let frames = frames_for_samples(WINDOW_SAMPLES);
        let step = seconds_to_frames(2.0) * FRAME_SIZE;
        let window = WindowActivity {
            frames: vec![[true, false, false]; frames],
        };
        let analysis = Analysis {
            total: WINDOW_SAMPLES + step,
            step,
            activities: vec![window.clone(), window],
            embeddings: (0..2)
                .map(|w| WindowEmbedding {
                    window: w,
                    local_speaker: 0,
                    embedding: vec![1.0],
                    anchor: true,
                })
                .collect(),
        };
        let segments = reconstruct(&DiarizationConfig::default(), &analysis, &[0, 0], 1);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].speaker, 0);
        assert!(segments[0].end > 11.5);
    }
}

#[cfg(test)]
mod chunk_layout_tests {
    use super::*;

    #[test]
    fn native_chunk_layout_covers_long_input_and_tail() {
        let config = DiarizationConfig {
            max_windows: usize::MAX,
            ..DiarizationConfig::default()
        };
        let total = 181 * 60 * SAMPLE_RATE as usize + 137;
        let layout = analysis_layout(&config, total);
        assert!(layout.windows > 3612);
        // The native hop rounds 32,000 samples to 119 frames of 270 samples.
        assert_eq!(layout.step, 32_130);
        let mut covered = 0;
        for index in 0..layout.windows {
            let start = index * layout.step;
            assert!(start <= covered);
            let valid = WINDOW_SAMPLES.min(total - start);
            covered = covered.max(start + valid);
        }
        assert_eq!(covered, total);
    }

    #[test]
    fn empty_and_short_input_do_not_add_extra_windows() {
        let config = DiarizationConfig::default();
        assert_eq!(analysis_layout(&config, 0).windows, 0);
        assert_eq!(analysis_layout(&config, 1).windows, 1);
        assert_eq!(analysis_layout(&config, WINDOW_SAMPLES).windows, 1);
        assert_eq!(analysis_layout(&config, WINDOW_SAMPLES + 1).windows, 2);
    }
}
