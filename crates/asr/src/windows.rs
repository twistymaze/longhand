//! What each recognition call hears (KTD6, KTD8).
//!
//! VAD over a segment's 16 kHz processing audio yields voiced spans; the
//! spans are packed into windows of at most 30 s. A window never spans a
//! silence, so no decoded text is carried across one, and never crosses a
//! speaker-turn boundary when diarization has supplied them. A span longer
//! than the maximum is cut at its quietest frame near the limit.

use serde::{Deserialize, Serialize};
use vad::{
    EarshotVad, VAD_OFFLINE_HANGOVER_MS, VAD_ONSET_MS, VAD_PREFILL_MS, VoiceActivityDetector,
};

use crate::SAMPLES_PER_MS;

/// Earshot predicts on exactly 256 samples (16 ms at 16 kHz).
pub const FRAME_SAMPLES: usize = 256;
pub const FRAME_MS: u64 = FRAME_SAMPLES as u64 / SAMPLES_PER_MS;
pub const MAX_WINDOW_MS: u64 = 30_000;
/// A pause up to this long stays inside one window; anything longer is a
/// silence and ends it.
pub const JOIN_GAP_MS: u64 = 2_000;
/// How far back from the limit an over-long span looks for its quietest frame.
const SPLIT_SEARCH_MS: u64 = 5_000;
pub(crate) const THRESHOLD: f32 = 0.5;

/// A half-open interval `[start_ms, end_ms)` within a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl Span {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms - self.start_ms
    }
}

/// Per-frame voice decisions and energies for one segment, built by
/// streaming its audio through [`Voicing::push`].
pub struct Voicing {
    vad: EarshotVad,
    carry: Vec<f32>,
    voiced: Vec<bool>,
    energy: Vec<f32>,
    samples: u64,
}

impl Voicing {
    pub fn new() -> Self {
        Self {
            vad: EarshotVad::new(THRESHOLD).expect("threshold is in range"),
            carry: Vec::with_capacity(FRAME_SAMPLES),
            voiced: Vec::new(),
            energy: Vec::new(),
            samples: 0,
        }
    }

    pub fn push(&mut self, mut pcm: &[f32]) {
        self.samples += pcm.len() as u64;
        while !pcm.is_empty() {
            let take = (FRAME_SAMPLES - self.carry.len()).min(pcm.len());
            self.carry.extend_from_slice(&pcm[..take]);
            pcm = &pcm[take..];
            if self.carry.len() == FRAME_SAMPLES {
                let frame = std::mem::take(&mut self.carry);
                self.classify(&frame);
                self.carry = frame;
                self.carry.clear();
            }
        }
    }

    fn classify(&mut self, frame: &[f32]) {
        // A non-finite sample is a corrupt frame, not speech.
        let voiced = self.vad.is_voice(frame).unwrap_or(false);
        let energy = frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32;
        self.voiced.push(voiced);
        self.energy.push(energy);
    }

    pub fn duration_ms(&self) -> u64 {
        self.samples / SAMPLES_PER_MS
    }

    pub fn frames(&self) -> &[bool] {
        &self.voiced
    }

    /// Voiced spans: runs of at least the onset length, widened by the
    /// pre-roll before and the offline hangover after, merged where they
    /// touch, and clamped to the audio.
    pub fn spans(&self) -> Vec<Span> {
        spans_from_frames(&self.voiced, self.duration_ms())
    }

    /// Speech runs that meet the onset length, without request padding.
    pub fn speech_spans(&self) -> Vec<Span> {
        speech_spans_from_frames(&self.voiced, self.duration_ms())
    }

    /// Windows for this segment; see [`plan_windows`].
    pub fn windows(&self, turn_cuts_ms: &[u64]) -> Vec<Span> {
        plan_windows(&self.spans(), &self.energy, turn_cuts_ms)
    }
}

impl Default for Voicing {
    fn default() -> Self {
        Self::new()
    }
}

pub fn spans_from_frames(voiced: &[bool], total_ms: u64) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    for speech in speech_spans_from_frames(voiced, total_ms) {
        let span = Span {
            start_ms: speech.start_ms.saturating_sub(VAD_PREFILL_MS),
            end_ms: (speech.end_ms + VAD_OFFLINE_HANGOVER_MS).min(total_ms),
        };
        match spans.last_mut() {
            Some(last) if span.start_ms <= last.end_ms => last.end_ms = span.end_ms,
            _ => spans.push(span),
        }
    }
    spans
}

/// Speech runs that meet the onset length, clamped to the audio.
fn speech_spans_from_frames(voiced: &[bool], total_ms: u64) -> Vec<Span> {
    let onset = VAD_ONSET_MS.div_ceil(FRAME_MS) as usize;
    let mut spans: Vec<Span> = Vec::new();
    let mut i = 0;
    while i < voiced.len() {
        if !voiced[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < voiced.len() && voiced[i] {
            i += 1;
        }
        if i - start < onset {
            continue;
        }
        let start_ms = start as u64 * FRAME_MS;
        let end_ms = (i as u64 * FRAME_MS).min(total_ms);
        if start_ms < end_ms {
            spans.push(Span { start_ms, end_ms });
        }
    }
    spans
}

/// Total duration of the supplied spans.
pub fn voiced_ms(spans: &[Span]) -> u64 {
    spans.iter().map(Span::duration_ms).sum()
}

/// Pack voiced spans into recognition windows.
///
/// - `energy` holds one mean-square value per [`FRAME_MS`] frame and chooses
///   where an over-long span is cut.
/// - `turn_cuts_ms` are speaker-change positions; no window contains one.
pub fn plan_windows(spans: &[Span], energy: &[f32], turn_cuts_ms: &[u64]) -> Vec<Span> {
    let mut cuts: Vec<u64> = turn_cuts_ms.to_vec();
    cuts.sort_unstable();

    // Split at turn changes, then at the length limit.
    let mut pieces = Vec::new();
    for span in spans {
        let mut start = span.start_ms;
        for &cut in cuts
            .iter()
            .filter(|&&c| c > span.start_ms && c < span.end_ms)
        {
            split_long(
                Span {
                    start_ms: start,
                    end_ms: cut,
                },
                energy,
                &mut pieces,
            );
            start = cut;
        }
        split_long(
            Span {
                start_ms: start,
                end_ms: span.end_ms,
            },
            energy,
            &mut pieces,
        );
    }

    let mut windows: Vec<Span> = Vec::new();
    for piece in pieces {
        if let Some(last) = windows.last_mut() {
            let gap = piece.start_ms.saturating_sub(last.end_ms);
            let crosses_turn = cuts
                .iter()
                .any(|&c| c >= last.end_ms && c <= piece.start_ms);
            if gap <= JOIN_GAP_MS && piece.end_ms - last.start_ms <= MAX_WINDOW_MS && !crosses_turn
            {
                last.end_ms = piece.end_ms;
                continue;
            }
        }
        windows.push(piece);
    }
    windows
}

fn split_long(mut span: Span, energy: &[f32], out: &mut Vec<Span>) {
    while span.duration_ms() > MAX_WINDOW_MS {
        let limit = span.start_ms + MAX_WINDOW_MS;
        let from = (limit - SPLIT_SEARCH_MS) / FRAME_MS;
        let to = limit / FRAME_MS;
        // The quietest frame boundary in the search range; with no energy
        // data the limit itself.
        let cut_frame = (from..to)
            .filter(|&f| (f as usize) < energy.len())
            .min_by(|&a, &b| energy[a as usize].total_cmp(&energy[b as usize]))
            .unwrap_or(to);
        let cut = (cut_frame * FRAME_MS).clamp(span.start_ms + 1, limit);
        out.push(Span {
            start_ms: span.start_ms,
            end_ms: cut,
        });
        span.start_ms = cut;
    }
    if span.duration_ms() > 0 {
        out.push(span);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(pattern: &[(bool, u64)]) -> Vec<bool> {
        pattern
            .iter()
            .flat_map(|&(v, ms)| std::iter::repeat_n(v, (ms / FRAME_MS) as usize))
            .collect()
    }

    #[test]
    fn a_blip_shorter_than_the_onset_is_not_speech() {
        let v = frames(&[(false, 1_600), (true, 32), (false, 1_600)]);
        assert!(spans_from_frames(&v, 3_232).is_empty());
    }

    #[test]
    fn speech_spans_keep_onset_runs_without_changing_padded_windows() {
        let mut voicing = Voicing::new();
        voicing.voiced = frames(&[
            (false, 736),
            (true, 4208),
            (false, 1120),
            (true, 2544),
            (false, 9184),
            (true, 96),
            (false, 128),
            (true, 80),
            (false, 304),
            (true, 48),
            (false, 1552),
        ]);
        voicing.samples = 20_000 * SAMPLES_PER_MS;
        assert_eq!(
            voicing.speech_spans(),
            vec![
                Span {
                    start_ms: 736,
                    end_ms: 4944
                },
                Span {
                    start_ms: 6064,
                    end_ms: 8608
                },
                Span {
                    start_ms: 17792,
                    end_ms: 17888
                },
                Span {
                    start_ms: 18016,
                    end_ms: 18096
                },
            ]
        );
        assert_eq!(
            voicing.spans(),
            vec![
                Span {
                    start_ms: 286,
                    end_ms: 5394
                },
                Span {
                    start_ms: 5614,
                    end_ms: 9058
                },
                Span {
                    start_ms: 17342,
                    end_ms: 18546
                },
            ]
        );
        assert_eq!(
            voicing.windows(&[]),
            vec![
                Span {
                    start_ms: 286,
                    end_ms: 9058
                },
                Span {
                    start_ms: 17342,
                    end_ms: 18546
                },
            ]
        );
    }

    #[test]
    fn speech_spans_require_the_full_onset_and_clamp_to_audio() {
        assert!(speech_spans_from_frames(&[true; 3], 48).is_empty());
        assert_eq!(
            speech_spans_from_frames(&[true; 4], 64),
            vec![Span {
                start_ms: 0,
                end_ms: 64
            },]
        );
        assert_eq!(
            speech_spans_from_frames(&[true; 4], 60),
            vec![Span {
                start_ms: 0,
                end_ms: 60
            },]
        );
        assert!(speech_spans_from_frames(&[true; 4], 0).is_empty());
        assert!(speech_spans_from_frames(&[], 1000).is_empty());
    }

    #[test]
    fn a_voiced_run_is_widened_by_pre_roll_and_hangover() {
        let v = frames(&[(false, 1_600), (true, 800), (false, 1_600)]);
        let spans = spans_from_frames(&v, 4_000);
        assert_eq!(
            spans,
            vec![Span {
                start_ms: 1_600 - VAD_PREFILL_MS,
                end_ms: 2_400 + VAD_OFFLINE_HANGOVER_MS
            }]
        );
    }

    #[test]
    fn a_short_pause_stays_inside_a_window_and_a_silence_ends_it() {
        let spans = [
            Span {
                start_ms: 0,
                end_ms: 5_000,
            },
            Span {
                start_ms: 6_000,
                end_ms: 9_000,
            },
            Span {
                start_ms: 20_000,
                end_ms: 24_000,
            },
        ];
        let windows = plan_windows(&spans, &[], &[]);
        assert_eq!(
            windows,
            vec![
                Span {
                    start_ms: 0,
                    end_ms: 9_000
                },
                Span {
                    start_ms: 20_000,
                    end_ms: 24_000
                }
            ]
        );
    }

    #[test]
    fn packing_stops_at_the_window_limit() {
        let spans: Vec<Span> = (0..8)
            .map(|i| Span {
                start_ms: i * 6_000,
                end_ms: i * 6_000 + 5_000,
            })
            .collect();
        let windows = plan_windows(&spans, &[], &[]);
        assert!(windows.iter().all(|w| w.duration_ms() <= MAX_WINDOW_MS));
        assert_eq!(windows.len(), 2);
    }

    #[test]
    fn an_over_long_span_is_cut_at_its_quietest_frame_before_the_limit() {
        let span = Span {
            start_ms: 0,
            end_ms: 70_000,
        };
        let mut energy = vec![1.0f32; (70_000 / FRAME_MS) as usize];
        let quiet = (27_000 / FRAME_MS) as usize;
        energy[quiet] = 0.0;
        let windows = plan_windows(&[span], &energy, &[]);
        assert_eq!(windows[0].end_ms, quiet as u64 * FRAME_MS);
        assert!(windows.iter().all(|w| w.duration_ms() <= MAX_WINDOW_MS));
        assert_eq!(windows.last().unwrap().end_ms, 70_000);
        let covered: u64 = windows.iter().map(Span::duration_ms).sum();
        assert_eq!(covered, 70_000);
    }

    #[test]
    fn no_window_crosses_a_speaker_turn() {
        let spans = [
            Span {
                start_ms: 0,
                end_ms: 10_000,
            },
            Span {
                start_ms: 10_500,
                end_ms: 14_000,
            },
        ];
        let windows = plan_windows(&spans, &[], &[4_000, 10_200]);
        assert_eq!(
            windows,
            vec![
                Span {
                    start_ms: 0,
                    end_ms: 4_000
                },
                Span {
                    start_ms: 4_000,
                    end_ms: 10_000
                },
                Span {
                    start_ms: 10_500,
                    end_ms: 14_000
                }
            ]
        );
    }
}
