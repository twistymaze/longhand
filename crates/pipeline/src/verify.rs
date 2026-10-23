//! The verify stage (R17a, R20): it measures and logs how much voiced audio
//! the transcript misses. Low coverage never stops a note: the note is
//! saved with the text that it has. Per segment, the VAD's voiced spans are measured
//! against the spans the recognizer actually put text on (its timestamped
//! segments, not the request windows around them), so a window that decoded
//! one word does not count its whole length as covered. Live parts and
//! gap-fill windows put text on the spans of their timed words.
//!
//! A segment has low coverage below [`MIN_COVERAGE`], or when one stretch of
//! speech with nothing transcribed between its parts carries more than
//! [`MAX_UNCOVERED_MS`] of voiced time. A segment with no voiced time
//! passes: silence is never a shortfall (AE16). Voiced time that a gap fill
//! sent to the model counts as covered, with or without words
//! ([`refilled_spans`]). A failure that an older version recorded keeps its
//! windows to redo, so a retry of that note re-transcribes only those.

use std::path::Path;

use asr::{LiveWord, Span};
use serde::{Deserialize, Serialize};
use session_core::{Segment, Stage};

use crate::transcribe::{WindowPlan, WindowResult, plan_segment, results};
use crate::{SharedStore, StageError, checkpoint, lock};

/// The share of voiced time that must carry text (provisional).
pub const MIN_COVERAGE: f64 = 0.9;
/// The most voiced time one untranscribed stretch may hold (provisional).
pub const MAX_UNCOVERED_MS: u64 = 60_000;
/// How far text covers beyond its timestamps to allow for timing error.
pub const SLACK_MS: u64 = 1_000;
/// Written when verification fails; a retry reads it to know what to redo.
/// Only an older version wrote it: verification no longer fails.
pub const FAILED: &str = "failed";
/// Written when the text of a note covers too little of its voiced time.
/// The note page shows a short warning (R17a).
pub const LOW_COVERAGE: &str = "low-coverage";

/// Save or remove the low-coverage mark of a note.
pub(crate) fn mark_low_coverage(
    store: &SharedStore,
    session_id: &str,
    low: bool,
) -> Result<(), StageError> {
    if low {
        checkpoint::put(store, session_id, Stage::Verify, LOW_COVERAGE, &true)
    } else {
        lock(store).delete_checkpoint(session_id, Stage::Verify, LOW_COVERAGE)?;
        Ok(())
    }
}

/// The last verification found too little text for the voiced time.
pub fn low_coverage(store: &SharedStore, session_id: &str) -> Result<bool, StageError> {
    Ok(checkpoint::get::<bool>(store, session_id, Stage::Verify, LOW_COVERAGE)?.unwrap_or(false))
}

/// What verification found in one segment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    pub segment: u32,
    pub voiced_ms: u64,
    pub covered_ms: u64,
    /// Untranscribed stretches, within the segment: each runs from the first
    /// to the last uncovered voiced moment with no transcribed time between.
    pub uncovered: Vec<Span>,
    /// The voiced time inside the worst stretch.
    pub longest_uncovered_ms: u64,
}

impl Coverage {
    pub fn ratio(&self) -> f64 {
        if self.voiced_ms == 0 {
            1.0
        } else {
            self.covered_ms as f64 / self.voiced_ms as f64
        }
    }

    pub fn passes(&self) -> bool {
        self.voiced_ms == 0
            || (self.ratio() >= MIN_COVERAGE && self.longest_uncovered_ms <= MAX_UNCOVERED_MS)
    }
}

/// The record of a failed verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyFailure {
    pub segments: Vec<Coverage>,
    /// (segment, window) pairs whose span overlaps an uncovered stretch.
    pub redo: Vec<(u32, usize)>,
}

pub fn run(
    store: &SharedStore,
    session_id: &str,
    session_dir: &Path,
    segments: &[Segment],
) -> Result<(), StageError> {
    let windows = results(store, session_id)?;
    let mut failing = Vec::new();
    for segment in segments {
        let plan: WindowPlan = checkpoint::get(
            store,
            session_id,
            Stage::Transcribe,
            &checkpoint::plan_key(segment.index),
        )?
        .ok_or_else(|| {
            StageError::Failed(format!("segment {} has no window plan", segment.index))
        })?;
        // Old non-silent plans contain only padded spans. Read the audio
        // again. Zero-voiced plans also preserve adopted transcript text.
        let speech = match &plan.speech {
            Some(speech) => speech.clone(),
            None if plan.voiced_ms > 0 => plan_segment(session_dir, segment, &[])?
                .speech
                .expect("a new plan includes speech spans"),
            None => Vec::new(),
        };
        let own: Vec<&WindowResult> = windows
            .iter()
            .filter(|w| w.segment == segment.index)
            .collect();
        let text = text_spans(&own);
        let coverage = measure(segment.index, &speech, &text);
        let judged = judge(segment.index, &speech, &text, &refilled_spans(&own));
        log::info!(
            "coverage of {session_id} segment {}: {:.1}% of {} s voiced, worst gap {} s, \
             {} s of sound without words after a gap fill",
            segment.index,
            coverage.ratio() * 100.0,
            coverage.voiced_ms / 1000,
            coverage.longest_uncovered_ms / 1000,
            wordless_ms(&coverage, &judged) / 1000
        );
        if !judged.passes() {
            failing.push(judged);
        }
        checkpoint::put(
            store,
            session_id,
            Stage::Verify,
            &checkpoint::segment_key(segment.index),
            &coverage,
        )?;
    }

    // Low coverage does not stop the note: a note with a part that has no
    // words is better than a note that does not finish.
    if !failing.is_empty() {
        log::warn!(
            "{session_id}: {}; the note is saved with the text that it has",
            describe(&failing, segments)
        );
    }
    mark_low_coverage(store, session_id, !failing.is_empty())?;
    checkpoint::mark_complete(store, session_id, Stage::Verify, segments.len())
}

/// The spans carrying decoded text, within the segment: only timestamped
/// segments count (U20). Text a window decoded without timestamps covers
/// nothing, since one word would otherwise vouch for the whole window; the
/// 90% and long-stretch rules tolerate an occasional such window. A live
/// part and a gap-fill window put text on the spans of their words (KTD7).
pub fn text_spans(windows: &[&WindowResult]) -> Vec<Span> {
    let mut spans = Vec::new();
    for w in windows {
        if let Some(words) = &w.words {
            spans.extend(word_spans(words).into_iter().filter_map(|s| {
                let (start, end) = (s.start_ms.max(w.start_ms), s.end_ms.min(w.end_ms));
                (end > start).then_some(Span {
                    start_ms: start,
                    end_ms: end,
                })
            }));
        }
        for s in w.segments.iter().filter(|s| !s.text.trim().is_empty()) {
            // A short window is padded before decoding, so a timestamp can
            // fall past the audio it was given.
            let start = (w.start_ms + s.start_ms).min(w.end_ms);
            let end = (w.start_ms + s.end_ms).min(w.end_ms);
            if end > start {
                spans.push(Span {
                    start_ms: start,
                    end_ms: end,
                });
            }
        }
    }
    union(spans)
}

/// The spans that a gap fill sent to the model, within the segment. A gap
/// fill runs only in a segment with live text, on the voiced time that the
/// live text missed (KTD6), so the model heard this voiced time two times.
/// Sound that still has no words there, for example background music, is
/// not missing text: a retry runs the same model on the same audio and
/// cannot add text.
pub fn refilled_spans(windows: &[&WindowResult]) -> Vec<Span> {
    if !windows.iter().any(|w| w.live) {
        return Vec::new();
    }
    union(
        windows
            .iter()
            .filter(|w| !w.live)
            .map(|w| Span {
                start_ms: w.start_ms,
                end_ms: w.end_ms,
            })
            .collect(),
    )
}

/// The coverage that decides whether a segment passes: [`measure`], with
/// the [`refilled_spans`] also counted as covered. The slack applies to
/// text only: voiced time outside a gap-fill window was not sent to the
/// model.
pub fn judge(segment: u32, voiced: &[Span], text: &[Span], refilled: &[Span]) -> Coverage {
    measure_with_exact(segment, voiced, text, refilled)
}

/// The voiced time that has no text but that a gap fill heard, from the
/// coverage against the text and the [`judge`] coverage.
pub fn wordless_ms(coverage: &Coverage, judged: &Coverage) -> u64 {
    (coverage.voiced_ms - coverage.covered_ms).saturating_sub(judged.voiced_ms - judged.covered_ms)
}

/// The spans of the words that have text, in milliseconds from the start of
/// the segment. A word of no length covers one millisecond, so that
/// [`measure`] counts it.
pub fn word_spans(words: &[LiveWord]) -> Vec<Span> {
    union(
        words
            .iter()
            .filter(|w| !w.text.trim().is_empty())
            .map(|w| Span {
                start_ms: w.start_ms,
                end_ms: w.end_ms.max(w.start_ms + 1),
            })
            .collect(),
    )
}

fn union(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by_key(|s| s.start_ms);
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        match out.last_mut() {
            Some(last) if s.start_ms <= last.end_ms => last.end_ms = last.end_ms.max(s.end_ms),
            _ => out.push(s),
        }
    }
    out
}

/// Voiced spans against the spans carrying text, each widened by
/// [`SLACK_MS`].
pub fn measure(segment: u32, voiced: &[Span], covered: &[Span]) -> Coverage {
    measure_with_exact(segment, voiced, covered, &[])
}

/// [`measure`], with `exact` spans also counted as covered, without the
/// slack.
fn measure_with_exact(segment: u32, voiced: &[Span], covered: &[Span], exact: &[Span]) -> Coverage {
    let covered = union(
        covered
            .iter()
            .map(|c| Span {
                start_ms: c.start_ms.saturating_sub(SLACK_MS),
                end_ms: c.end_ms + SLACK_MS,
            })
            .chain(exact.iter().copied())
            .collect(),
    );
    let voiced_ms: u64 = voiced.iter().map(Span::duration_ms).sum();
    let mut covered_ms = 0;
    // The uncovered pieces of voiced time, in order.
    let mut pieces: Vec<Span> = Vec::new();
    let mut c = 0;
    for v in voiced {
        let mut cursor = v.start_ms;
        while c < covered.len() && covered[c].end_ms <= v.start_ms {
            c += 1;
        }
        let mut k = c;
        while k < covered.len() && covered[k].start_ms < v.end_ms {
            let (start, end) = (
                covered[k].start_ms.max(v.start_ms),
                covered[k].end_ms.min(v.end_ms),
            );
            if start > cursor {
                pieces.push(Span {
                    start_ms: cursor,
                    end_ms: start,
                });
            }
            covered_ms += end - start;
            cursor = cursor.max(end);
            k += 1;
        }
        if cursor < v.end_ms {
            pieces.push(Span {
                start_ms: cursor,
                end_ms: v.end_ms,
            });
        }
    }

    // Pieces with no covered time between them form one stretch: speech
    // with pauses that the recognizer missed as a whole.
    let mut stretches: Vec<(Span, u64)> = Vec::new();
    for p in pieces {
        let joined = stretches.last_mut().filter(|(s, _)| {
            !covered
                .iter()
                .any(|c| c.start_ms < p.start_ms && s.end_ms < c.end_ms)
        });
        match joined {
            Some((s, voiced)) => {
                s.end_ms = p.end_ms;
                *voiced += p.duration_ms();
            }
            None => stretches.push((p, p.duration_ms())),
        }
    }
    Coverage {
        segment,
        voiced_ms,
        covered_ms,
        longest_uncovered_ms: stretches.iter().map(|(_, v)| *v).max().unwrap_or(0),
        uncovered: stretches.into_iter().map(|(s, _)| s).collect(),
    }
}

fn describe(failing: &[Coverage], segments: &[Segment]) -> String {
    let parts: Vec<String> = failing
        .iter()
        .map(|c| {
            let offset = segments
                .iter()
                .find(|s| s.index == c.segment)
                .map_or(0, |s| s.offset_ms);
            let worst: Vec<String> = c
                .uncovered
                .iter()
                .take(5)
                .map(|u| {
                    format!(
                        "{}–{}",
                        clock(offset + u.start_ms),
                        clock(offset + u.end_ms)
                    )
                })
                .collect();
            let more = c.uncovered.len().saturating_sub(worst.len());
            format!(
                "segment {} has text on {:.0}% of its speech; untranscribed {}{}",
                c.segment + 1,
                c.ratio() * 100.0,
                worst.join(", "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            )
        })
        .collect();
    format!("the transcript misses speech: {}", parts.join("; "))
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Before a retry of a failed verification: forget the windows that missed
/// speech, and the completion of every stage after them, so the runner
/// re-transcribes only those windows, joins again and verifies again. The
/// live rows stay (KTD7): only window rows are deleted.
pub fn prepare_retry(store: &SharedStore, session_id: &str) -> Result<bool, StageError> {
    let Some(failure) = checkpoint::get::<VerifyFailure>(store, session_id, Stage::Verify, FAILED)?
    else {
        return Ok(false);
    };
    let guard = lock(store);
    for (segment, window) in &failure.redo {
        guard.delete_checkpoint(
            session_id,
            Stage::Transcribe,
            &checkpoint::window_key(*segment, *window),
        )?;
        guard.delete_checkpoint(
            session_id,
            Stage::Transcribe,
            &checkpoint::segment_done_key(*segment),
        )?;
    }
    guard.delete_checkpoint(session_id, Stage::Transcribe, checkpoint::DONE)?;
    guard.delete_checkpoint(session_id, Stage::Join, checkpoint::DONE)?;
    guard.clear_checkpoints(session_id, Stage::Verify)?;
    Ok(true)
}
