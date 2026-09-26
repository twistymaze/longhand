//! The transcribe stage (KTD6, R17a window side). Per segment: VAD over the
//! processing file plans the windows, the plan is checkpointed so a resume
//! transcribes exactly the windows it planned, and each window's result is
//! checkpointed as it completes.
//!
//! A segment with live rows (R11, R13) gets a gap fill: only the planned
//! windows that hold voiced time that no live word covers are transcribed,
//! and each of those windows keeps only its words inside the gaps. A segment
//! with no live rows gets a full pass.
//!
//! Each echo span of a stereo segment (KTD8 step 7) is a gap too (KTD10):
//! the live words inside it came from echoed audio, so they are left out,
//! and the gap fill recognizes the span on the cleaned processing file.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use asr::{AsrSegment, LiveWord, Recognizer, SAMPLES_PER_MS, Span, Voicing};
use serde::{Deserialize, Serialize};
use session_core::folder::{Transcript, TranscriptLine};
use session_core::{Segment, Stage, Store};

use crate::assemble::open_proc;
use crate::live::{self, LiveRow};
use crate::verify::{self, SLACK_MS};
use crate::{Control, SharedStore, StageError, checkpoint, echo, lock};

const READ_BLOCK: usize = 16_000;

/// A gap-fill word within this distance of a live word says that live word
/// again, and the merge drops it (Anarlog's rule).
pub const MERGE_MS: u64 = 150;

/// The planned windows of one segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlan {
    pub segment: u32,
    pub duration_ms: u64,
    /// Voiced duration by VAD.
    pub voiced_ms: u64,
    /// The voiced spans themselves, which verification measures coverage
    /// against (U20). Empty in a plan checkpointed before they were kept.
    #[serde(default)]
    pub voiced: Vec<Span>,
    pub windows: Vec<Span>,
}

/// One transcribed window, or one part of the live transcript. Window bounds
/// are within the segment; ASR segment times are within the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowResult {
    pub segment: u32,
    /// The window's number in the plan, or the part's number for a live
    /// part.
    pub window: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub segments: Vec<AsrSegment>,
    /// Timed words, in milliseconds from the start of the segment: the
    /// words of a live part, or the words that a gap fill kept (KTD6).
    /// `None` for a window of a full pass, whose text is in `segments`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<LiveWord>>,
    /// True for a part of the live transcript (KTD7).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub live: bool,
}

impl WindowResult {
    /// A live part as a result that Join and Verify read (KTD7).
    pub fn from_live(row: LiveRow) -> Self {
        Self {
            segment: row.segment,
            window: row.part as usize,
            start_ms: row.start_ms,
            end_ms: row.end_ms,
            text: row.text(),
            segments: Vec::new(),
            words: Some(row.words),
            live: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub segment: u32,
    pub windows_done: usize,
    pub windows_total: usize,
}

/// Speaker-change positions per segment, in ms within the segment. Empty
/// until diarization runs ahead of transcription (U9).
pub type TurnCuts = BTreeMap<u32, Vec<u64>>;

/// Loads a recognition model by its catalog id.
pub type Load<'a> = dyn Fn(&str) -> Result<Box<dyn Recognizer>, String> + 'a;

/// The runner's engine slot, which the live worker shares (KTD9), and the
/// model that the session recorded.
pub struct Slot<'a> {
    pub slot: &'a Mutex<Option<Box<dyn Recognizer>>>,
    pub model: &'a str,
    pub load: &'a Load<'a>,
}

impl Slot<'_> {
    /// Lock the slot for one window, after [`Control::proceed`] returns, so
    /// no wait for a suspension holds the lock. A live worker may have
    /// loaded another model in the meantime: the session's model is loaded
    /// again then.
    fn hold(
        &self,
        control: &dyn Control,
    ) -> Result<MutexGuard<'_, Option<Box<dyn Recognizer>>>, StageError> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        // Checked under the lock, so a shutdown that freed the engine is
        // never followed by loading it again.
        if control.is_cancelled() {
            return Err(StageError::Cancelled);
        }
        // A queued session keeps the model it recorded (KTD2).
        live::ensure_model(&mut slot, self.model, self.load).map_err(StageError::Failed)?;
        Ok(slot)
    }
}

/// What a segment with live rows needs from recognition (KTD6).
struct GapFill {
    words: Vec<LiveWord>,
    gaps: Vec<Span>,
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    store: &SharedStore,
    session_id: &str,
    session_dir: &Path,
    segments: &[Segment],
    turn_cuts: &TurnCuts,
    engine: &Slot<'_>,
    control: &dyn Control,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), StageError> {
    // The live worker saved its last part before processing started (KTD9),
    // so the live rows do not change during this stage.
    let live_rows = live::rows(store, session_id)?;
    let again = lock(store).transcribe_again_marks(session_id)?.model;
    for segment in segments {
        let done_key = checkpoint::segment_done_key(segment.index);
        if checkpoint::get::<bool>(store, session_id, Stage::Transcribe, &done_key)?.is_some() {
            continue;
        }
        control.proceed()?;
        let plan = match checkpoint::get::<WindowPlan>(
            store,
            session_id,
            Stage::Transcribe,
            &checkpoint::plan_key(segment.index),
        )? {
            Some(plan) => plan,
            None => {
                let cuts = turn_cuts.get(&segment.index).map_or(&[][..], Vec::as_slice);
                let plan = plan_segment(session_dir, segment, cuts)?;
                put_for_run(
                    store,
                    session_id,
                    again.as_deref(),
                    &checkpoint::plan_key(segment.index),
                    &plan,
                )?;
                plan
            }
        };

        let rows: Vec<&LiveRow> = live_rows
            .iter()
            .filter(|r| r.segment == segment.index)
            .collect();
        let echo = if rows.is_empty() {
            Vec::new()
        } else {
            echo_spans(store, session_id, segment.index)?
        };
        let fill = (!rows.is_empty()).then(|| {
            // Live words inside an echo span came from echoed audio: they
            // give way to the cleaned file (KTD10).
            let words = without_echo(rows.iter().flat_map(|r| r.words.iter().cloned()), &echo);
            // A plan checkpointed before voiced spans were kept: its
            // windows hold the voiced time.
            let voiced = if plan.voiced.is_empty() && plan.voiced_ms > 0 {
                &plan.windows
            } else {
                &plan.voiced
            };
            let parts: Vec<Span> = rows
                .iter()
                .map(|r| Span {
                    start_ms: r.start_ms,
                    end_ms: r.end_ms,
                })
                .collect();
            let mut gaps = gaps(voiced, &parts, &words);
            gaps.extend(echo.iter().copied());
            GapFill {
                gaps: union(gaps),
                words,
            }
        });
        let todo: Vec<usize> = match &fill {
            None => (0..plan.windows.len()).collect(),
            Some(fill) => windows_in(&plan.windows, &fill.gaps),
        };
        if let Some(fill) = &fill {
            log::info!(
                "{session_id} segment {}: live text covers all but {} s of speech \
                 ({} s of echo); {} of {} windows get a gap fill",
                segment.index,
                fill.gaps.iter().map(Span::duration_ms).sum::<u64>() / 1000,
                echo.iter().map(Span::duration_ms).sum::<u64>() / 1000,
                todo.len(),
                plan.windows.len()
            );
        }

        let prefix = checkpoint::window_prefix(segment.index);
        let finished: HashSet<String> = checkpoint::all(store, session_id, Stage::Transcribe)?
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| k.starts_with(&prefix))
            .collect();

        let total = todo.len();
        let mut done = todo
            .iter()
            .filter(|&&i| finished.contains(&checkpoint::window_key(segment.index, i)))
            .count();
        progress(Progress {
            segment: segment.index,
            windows_done: done,
            windows_total: total,
        });
        // Opened at the first window to transcribe: a segment that the
        // live text covers reads no audio.
        let mut reader = None;
        for i in todo {
            let window = plan.windows[i];
            let key = checkpoint::window_key(segment.index, i);
            if finished.contains(&key) {
                continue;
            }
            control.proceed()?;
            let reader = match &mut reader {
                Some(reader) => reader,
                None => reader.insert(open_proc(session_dir, segment)?),
            };
            let pcm = read_span(reader, window)?;
            let result = {
                let mut slot = engine.hold(control)?;
                let recognizer = slot.as_deref_mut().expect("hold loads the model");
                recognize(recognizer, &pcm, segment.index, i, window, fill.as_ref())
            };
            let result = result.map_err(|e| {
                StageError::Failed(format!(
                    "segment {}, window {} ({}–{} ms): {e}",
                    segment.index, i, window.start_ms, window.end_ms
                ))
            })?;
            put_for_run(store, session_id, again.as_deref(), &key, &result)?;
            done += 1;
            progress(Progress {
                segment: segment.index,
                windows_done: done,
                windows_total: total,
            });
        }
        put_for_run(store, session_id, again.as_deref(), &done_key, &true)?;
    }
    put_for_run(
        store,
        session_id,
        again.as_deref(),
        checkpoint::DONE,
        &checkpoint::StageDone {
            segments: segments.len(),
        },
    )?;
    Ok(())
}

/// Write a checkpoint of this stage. When a resume ended the Transcribe
/// Again run (`again`, its model) while a window was recognized, the
/// resume restored the previous checkpoints: the write stops the stage
/// instead of replacing them (KTD14). Checked under the store lock that
/// the resume takes.
fn put_for_run<T: Serialize>(
    store: &SharedStore,
    session_id: &str,
    again: Option<&str>,
    key: &str,
    value: &T,
) -> Result<(), StageError> {
    let payload = serde_json::to_string(value)?;
    let guard = lock(store);
    if again.is_some() && guard.transcribe_again_marks(session_id)?.model.as_deref() != again {
        return Err(StageError::Cancelled);
    }
    guard.put_checkpoint(session_id, Stage::Transcribe, key, &payload)?;
    Ok(())
}

/// Transcribe one window: in a full pass, by segments; in a gap fill, by
/// tokens, keeping only the words that the merge keeps.
fn recognize(
    recognizer: &mut dyn Recognizer,
    pcm: &[f32],
    segment: u32,
    window: usize,
    span: Span,
    fill: Option<&GapFill>,
) -> Result<WindowResult, asr::AsrError> {
    let mut result = WindowResult {
        segment,
        window,
        start_ms: span.start_ms,
        end_ms: span.end_ms,
        text: String::new(),
        segments: Vec::new(),
        words: None,
        live: false,
    };
    match fill {
        None => {
            let text = recognizer.transcribe(pcm)?;
            result.text = text.text;
            result.segments = text.segments;
        }
        Some(fill) => {
            let tokens = recognizer.transcribe_tokens(pcm)?;
            let words = asr::words_from_tokens(&tokens, span.start_ms);
            let kept = merge(words, &fill.gaps, &fill.words);
            result.text = asr::join_words(&kept);
            result.words = Some(kept);
        }
    }
    Ok(result)
}

/// The stretches of voiced time that no live word covers, by the rule and
/// the slack that Verify uses (KTD6), so Verify passes after a gap fill
/// that recognizes the speech. Voiced time that no saved part covers
/// (`parts`) is also a gap, however near the live words are: the live
/// worker did not hear it.
pub fn gaps(voiced: &[Span], parts: &[Span], live: &[LiveWord]) -> Vec<Span> {
    let mut all = verify::measure(0, voiced, &verify::word_spans(live)).uncovered;
    all.extend(unheard(voiced, parts));
    union(all)
}

/// A part span that ends this near the next part span, or the voiced time,
/// is not a gap: the part bounds round to the millisecond.
const PART_GAP_MIN_MS: u64 = 10;

/// The voiced time outside every part span.
fn unheard(voiced: &[Span], parts: &[Span]) -> Vec<Span> {
    let heard = union(parts.to_vec());
    let mut out = Vec::new();
    for v in voiced {
        let mut at = v.start_ms;
        for p in heard
            .iter()
            .filter(|p| p.end_ms > v.start_ms && p.start_ms < v.end_ms)
        {
            if p.start_ms > at {
                out.push(Span {
                    start_ms: at,
                    end_ms: p.start_ms,
                });
            }
            at = at.max(p.end_ms);
        }
        if v.end_ms > at {
            out.push(Span {
                start_ms: at,
                end_ms: v.end_ms,
            });
        }
    }
    out.retain(|s| s.duration_ms() >= PART_GAP_MIN_MS);
    out
}

/// `spans` sorted, with the spans that overlap or touch made one.
fn union(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by_key(|s| (s.start_ms, s.end_ms));
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        match out.last_mut() {
            Some(last) if s.start_ms <= last.end_ms => last.end_ms = last.end_ms.max(s.end_ms),
            _ => out.push(s),
        }
    }
    out
}

/// The echo spans of a segment, from its Assemble checkpoint. Empty for a
/// mono segment.
fn echo_spans(
    store: &SharedStore,
    session_id: &str,
    segment: u32,
) -> Result<Vec<Span>, StageError> {
    Ok(echo::saved(store, session_id, segment)?
        .map(|e| e.spans)
        .unwrap_or_default())
}

/// The live words that overlap no echo span (KTD10). The gap fill gives
/// the time of the others to the cleaned file, and [`results`] leaves them
/// out.
pub fn without_echo(words: impl IntoIterator<Item = LiveWord>, echo: &[Span]) -> Vec<LiveWord> {
    words
        .into_iter()
        .filter(|w| !echo.iter().any(|e| overlaps(word_span(w), *e)))
        .collect()
}

/// The numbers of the windows that overlap a gap.
pub fn windows_in(windows: &[Span], gaps: &[Span]) -> Vec<usize> {
    windows
        .iter()
        .enumerate()
        .filter(|(_, w)| gaps.iter().any(|g| overlaps(**w, *g)))
        .map(|(i, _)| i)
        .collect()
}

/// The gap-fill words to keep. A word is kept when it overlaps a gap, and
/// is not within [`MERGE_MS`] of a live word. Each gap is first widened by
/// [`SLACK_MS`], the slack that made it start that far from the live words
/// at its ends, so the words between the live text and the gap stay.
pub fn merge(batch: Vec<LiveWord>, gaps: &[Span], live: &[LiveWord]) -> Vec<LiveWord> {
    let keep: Vec<Span> = gaps
        .iter()
        .map(|g| Span {
            start_ms: g.start_ms.saturating_sub(SLACK_MS),
            end_ms: g.end_ms + SLACK_MS,
        })
        .collect();
    batch
        .into_iter()
        .filter(|word| {
            let span = word_span(word);
            keep.iter().any(|k| overlaps(span, *k))
                && !live
                    .iter()
                    .any(|l| distance(span, word_span(l)) <= MERGE_MS)
        })
        .collect()
}

fn word_span(word: &LiveWord) -> Span {
    Span {
        start_ms: word.start_ms,
        end_ms: word.end_ms.max(word.start_ms),
    }
}

/// True when two spans share time. A span of no length shares time with a
/// span it falls inside.
fn overlaps(a: Span, b: Span) -> bool {
    a.start_ms < b.end_ms && b.start_ms < a.end_ms
        || (a.start_ms == a.end_ms && b.start_ms <= a.start_ms && a.start_ms < b.end_ms)
}

/// The time between two spans: zero when they overlap or touch.
fn distance(a: Span, b: Span) -> u64 {
    if a.end_ms < b.start_ms {
        b.start_ms - a.end_ms
    } else {
        a.start_ms.saturating_sub(b.end_ms)
    }
}

/// VAD over a segment's processing file, streamed.
pub fn plan_segment(
    session_dir: &Path,
    segment: &Segment,
    turn_cuts_ms: &[u64],
) -> Result<WindowPlan, StageError> {
    let mut reader = open_proc(session_dir, segment)?;
    let mut voicing = Voicing::new();
    let mut block = Vec::with_capacity(READ_BLOCK);
    for sample in reader.samples::<f32>() {
        block.push(sample?);
        if block.len() == READ_BLOCK {
            voicing.push(&block);
            block.clear();
        }
    }
    voicing.push(&block);
    let spans = voicing.spans();
    Ok(WindowPlan {
        segment: segment.index,
        duration_ms: voicing.duration_ms(),
        voiced_ms: asr::voiced_ms(&spans),
        windows: voicing.windows(turn_cuts_ms),
        voiced: spans,
    })
}

fn read_span(
    reader: &mut hound::WavReader<std::io::BufReader<std::fs::File>>,
    span: Span,
) -> Result<Vec<f32>, StageError> {
    let start = span.start_ms * SAMPLES_PER_MS;
    let len = (span.duration_ms() * SAMPLES_PER_MS) as usize;
    reader.seek(start as u32)?;
    let mut pcm = Vec::with_capacity(len);
    for sample in reader.samples::<f32>().take(len) {
        pcm.push(sample?);
    }
    Ok(pcm)
}

/// Resume of a saved note that has no stored recognition results (KD3,
/// KTD12): an adopted note, or one whose results a new `transcript.md` from
/// another computer cleared. The text of each recorded segment comes from
/// the `transcript.md` in `folder`: each turn goes to the segment that it
/// falls in, by the offsets and durations of the segments. Each segment
/// gets one window with those turns, and counts as transcribed, so the
/// transcribe stage recognizes only the new segment. The segments have no
/// voiced spans, so verification measures only the new segment.
///
/// Call it under the store lock, after the resume, before the new segment
/// is added. Returns false and writes nothing when the note has stored
/// recognition results, or when `transcript.md` cannot be read: then every
/// segment is transcribed from its audio.
pub fn seed_from_transcript(
    store: &Store,
    session_id: &str,
    folder: &Path,
) -> Result<bool, StageError> {
    if !store.checkpoints(session_id, Stage::Transcribe)?.is_empty() {
        return Ok(false);
    }
    let mut segments = store.segments(session_id)?;
    if segments.is_empty() {
        return Ok(false);
    }
    let path = folder.join(crate::join::TRANSCRIPT_FILE);
    let transcript = match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|text| Transcript::parse(&text).map_err(|e| e.to_string()))
    {
        Ok(transcript) => transcript,
        Err(e) => {
            log::warn!(
                "{session_id} is transcribed again from its audio: {} cannot be used: {e}",
                path.display()
            );
            return Ok(false);
        }
    };
    segments.sort_by_key(|s| s.offset_ms);
    let mut turns: BTreeMap<u32, Vec<(u64, String)>> = BTreeMap::new();
    for line in transcript.lines {
        let TranscriptLine::Turn {
            position_ms, text, ..
        } = line
        else {
            continue;
        };
        // The last segment that starts at or before the turn, or the first.
        let segment = segments
            .iter()
            .rev()
            .find(|s| s.offset_ms <= position_ms)
            .unwrap_or(&segments[0]);
        let at = position_ms
            .saturating_sub(segment.offset_ms)
            .min(segment.duration_ms);
        turns.entry(segment.index).or_default().push((at, text));
    }
    for segment in &segments {
        let own = turns.remove(&segment.index).unwrap_or_default();
        let pieces: Vec<AsrSegment> = own
            .iter()
            .enumerate()
            .map(|(i, (start_ms, text))| AsrSegment {
                start_ms: *start_ms,
                end_ms: own
                    .get(i + 1)
                    .map_or(segment.duration_ms, |(next, _)| *next)
                    .max(*start_ms),
                text: text.clone(),
            })
            .collect();
        let whole = Span {
            start_ms: 0,
            end_ms: segment.duration_ms,
        };
        let plan = WindowPlan {
            segment: segment.index,
            duration_ms: segment.duration_ms,
            voiced_ms: 0,
            voiced: Vec::new(),
            windows: vec![whole],
        };
        let result = WindowResult {
            segment: segment.index,
            window: 0,
            start_ms: whole.start_ms,
            end_ms: whole.end_ms,
            text: pieces
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            segments: pieces,
            words: None,
            live: false,
        };
        let put = |key: &str, payload: String| {
            store.put_checkpoint(session_id, Stage::Transcribe, key, &payload)
        };
        put(
            &checkpoint::plan_key(segment.index),
            serde_json::to_string(&plan)?,
        )?;
        put(
            &checkpoint::window_key(segment.index, 0),
            serde_json::to_string(&result)?,
        )?;
        put(
            &checkpoint::segment_done_key(segment.index),
            serde_json::to_string(&true)?,
        )?;
    }
    log::info!(
        "{session_id} resumes with the text of its {} recorded segment(s) from {}",
        segments.len(),
        path.display()
    );
    Ok(true)
}

/// Every window result and every live part of a session in timeline order,
/// for the stages that follow (join, verify, summarize). A gap-fill window
/// holds only the words that the merge kept (KTD7). A live part holds only
/// its words outside the echo spans of its segment (KTD10).
pub fn results(store: &SharedStore, session_id: &str) -> Result<Vec<WindowResult>, StageError> {
    let mut out = Vec::new();
    let mut echo: BTreeMap<u32, Vec<Span>> = BTreeMap::new();
    for (key, payload) in checkpoint::all(store, session_id, Stage::Transcribe)? {
        if key.contains("-win-") {
            out.push(serde_json::from_str::<WindowResult>(&payload)?);
        } else if key.contains("-live-") {
            let mut row = serde_json::from_str::<LiveRow>(&payload)?;
            let spans = match echo.entry(row.segment) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(echo_spans(store, session_id, row.segment)?)
                }
            };
            if !spans.is_empty() {
                row.words = without_echo(std::mem::take(&mut row.words), spans);
            }
            out.push(WindowResult::from_live(row));
        }
    }
    out.sort_by_key(|r| (r.segment, r.start_ms, r.live, r.window));
    Ok(out)
}
