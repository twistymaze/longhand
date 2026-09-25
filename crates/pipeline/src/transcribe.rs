//! The transcribe stage (KTD6, R17a window side). Per segment: VAD over the
//! processing file plans the windows, the plan is checkpointed so a resume
//! transcribes exactly the windows it planned, and each window's result is
//! checkpointed as it completes.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use asr::{AsrSegment, Recognizer, SAMPLES_PER_MS, Span, Voicing};
use serde::{Deserialize, Serialize};
use session_core::{Segment, Stage};

use crate::assemble::open_proc;
use crate::{Control, SharedStore, StageError, checkpoint};

const READ_BLOCK: usize = 16_000;

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

/// One transcribed window. Window bounds are within the segment; ASR
/// segment times are within the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowResult {
    pub segment: u32,
    pub window: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub segments: Vec<AsrSegment>,
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
        if slot.as_ref().map(|e| e.model_id()) != Some(self.model) {
            *slot = None;
            *slot = Some((self.load)(self.model).map_err(StageError::Failed)?);
        }
        Ok(slot)
    }
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
                checkpoint::put(
                    store,
                    session_id,
                    Stage::Transcribe,
                    &checkpoint::plan_key(segment.index),
                    &plan,
                )?;
                plan
            }
        };

        let prefix = checkpoint::window_prefix(segment.index);
        let finished: std::collections::HashSet<String> =
            checkpoint::all(store, session_id, Stage::Transcribe)?
                .into_iter()
                .map(|(k, _)| k)
                .filter(|k| k.starts_with(&prefix))
                .collect();

        let mut reader = open_proc(session_dir, segment)?;
        let total = plan.windows.len();
        let mut done = finished.len();
        progress(Progress {
            segment: segment.index,
            windows_done: done,
            windows_total: total,
        });
        for (i, window) in plan.windows.iter().enumerate() {
            let key = checkpoint::window_key(segment.index, i);
            if finished.contains(&key) {
                continue;
            }
            control.proceed()?;
            let pcm = read_span(&mut reader, *window)?;
            let text = {
                let mut slot = engine.hold(control)?;
                let recognizer = slot.as_deref_mut().expect("hold loads the model");
                recognizer.transcribe(&pcm)
            };
            let text = text.map_err(|e| {
                StageError::Failed(format!(
                    "segment {}, window {} ({}–{} ms): {e}",
                    segment.index, i, window.start_ms, window.end_ms
                ))
            })?;
            let result = WindowResult {
                segment: segment.index,
                window: i,
                start_ms: window.start_ms,
                end_ms: window.end_ms,
                text: text.text,
                segments: text.segments,
            };
            checkpoint::put(store, session_id, Stage::Transcribe, &key, &result)?;
            done += 1;
            progress(Progress {
                segment: segment.index,
                windows_done: done,
                windows_total: total,
            });
        }
        checkpoint::put(store, session_id, Stage::Transcribe, &done_key, &true)?;
    }
    checkpoint::mark_complete(store, session_id, Stage::Transcribe, segments.len())?;
    Ok(())
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

/// Every window result of a session in timeline order, for the stages that
/// follow (join, summarize).
pub fn results(store: &SharedStore, session_id: &str) -> Result<Vec<WindowResult>, StageError> {
    let mut out = Vec::new();
    for (key, payload) in checkpoint::all(store, session_id, Stage::Transcribe)? {
        if key.contains("-win-") {
            out.push(serde_json::from_str::<WindowResult>(&payload)?);
        }
    }
    out.sort_by_key(|r| (r.segment, r.window));
    Ok(out)
}
