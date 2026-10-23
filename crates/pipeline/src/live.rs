//! The live transcript during recording (KTD3, KTD4, KTD7, KTD9).
//!
//! A live worker runs next to each capture segment. The recorder offers it
//! each frame with the frame's sample index through a bounded channel, and
//! never waits: a full channel drops the frame. The worker resamples to
//! 16 kHz and streams the audio through the recognizer in parts
//! ([`asr::Parts`]). Each finished part is saved at once as a checkpoint
//! row, `seg-NNNN-live-NNNN` in the Transcribe stage. The row holds the
//! part's span of segment audio and its timed words.
//!
//! A gap is segment audio that no saved part covers. Processing after Stop
//! gives a gap a full recognition pass (R13). A gap starts when:
//! - frames are dropped: the part ends at the last frame the worker
//!   received, and a new part starts at the next frame;
//! - the stream fails: the part ends at the last audio the stream accepted;
//! - the stream stalls: [`STALL_MS`] of voiced audio give no new committed
//!   text, and the part ends where the committed text last changed;
//! - the process stops before the worker saves its open part.
//!
//! The recording never stops for a live failure.
//!
//! The worker saves a part (the step) before it counts the part as covered
//! and tells the surface (the record). A row is written in one statement,
//! so a crash leaves the row whole or absent, and an absent row is a gap.
//!
//! The worker shares the runner's one loaded engine (KTD9). It locks the
//! engine slot for as long as it streams, and loads the note's model into
//! the slot when the slot holds another model or is empty. Processing is
//! suspended while a worker runs, and [`Workers::busy`] keeps it suspended
//! after Stop until the last worker has saved its last part and released
//! the slot. Workers run one after another: a segment's worker waits for
//! the worker of the segment before it.

pub mod archive;
pub mod features;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use asr::{AsrError, Cut, LivePart, LiveStream, LiveText, LiveWord, Parts, Recognizer, join_words};
use serde::{Deserialize, Serialize};
use session_core::Stage;
use tokio_util::sync::CancellationToken;

use crate::resample::StreamResampler;
use crate::{SharedStore, StageError, checkpoint, lock};

/// The runner's one loaded engine (KTD9).
pub type EngineSlot = Arc<Mutex<Option<Box<dyn Recognizer>>>>;

/// Loads a recognition model by its catalog id.
pub type Loader = Arc<dyn Fn(&str) -> Result<Box<dyn Recognizer>, String> + Send + Sync>;
pub type ExactLoader = Arc<
    dyn Fn(&models::recognition::ExecutionSpec) -> Result<Box<dyn Recognizer>, String>
        + Send
        + Sync,
>;

/// Frames of 20 ms that wait for the worker: 5 s of audio.
pub const CHANNEL_FRAMES: usize = 250;

/// Voiced audio with no new committed text that counts as a stall
/// (Anarlog's stall rule).
pub const STALL_MS: u64 = 45_000;

/// How long a stream that failed to start waits, in segment audio, before
/// the worker tries again.
const RETRY_MS: u64 = 10_000;

/// How often a worker that waits for frames checks for a quit.
const POLL: Duration = Duration::from_millis(100);

/// One saved part of the live transcript: a checkpoint row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveRow {
    pub segment: u32,
    /// The part's number in its segment, from 0.
    pub part: u32,
    /// The span of segment audio that the stream heard, in milliseconds
    /// from the start of the segment.
    pub start_ms: u64,
    pub end_ms: u64,
    /// In milliseconds from the start of the segment.
    pub words: Vec<LiveWord>,
}

impl LiveRow {
    pub fn text(&self) -> String {
        join_words(&self.words)
    }
}

/// Save `live` as part `part` of `segment`.
pub fn save_part(
    store: &SharedStore,
    note_id: &str,
    segment: u32,
    part: u32,
    live: &LivePart,
) -> Result<LiveRow, StageError> {
    let row = LiveRow {
        segment,
        part,
        start_ms: live.start_ms,
        end_ms: live.end_ms,
        words: live.words.clone(),
    };
    checkpoint::put(
        store,
        note_id,
        Stage::Transcribe,
        &checkpoint::live_key(segment, part),
        &row,
    )?;
    Ok(row)
}

/// Every live row of a session, in segment and part order.
pub fn rows(store: &SharedStore, note_id: &str) -> Result<Vec<LiveRow>, StageError> {
    let mut out = Vec::new();
    for (key, payload) in checkpoint::all(store, note_id, Stage::Transcribe)? {
        if key.contains("-live-") {
            out.push(serde_json::from_str::<LiveRow>(&payload)?);
        }
    }
    out.sort_by_key(|r| (r.segment, r.part));
    Ok(out)
}

/// The live rows of one segment, in part order.
pub fn segment_rows(
    store: &SharedStore,
    note_id: &str,
    segment: u32,
) -> Result<Vec<LiveRow>, StageError> {
    let mut out = rows(store, note_id)?;
    out.retain(|r| r.segment == segment);
    Ok(out)
}

/// Delete the live rows of a segment that was discarded. Returns how many
/// rows it deleted.
pub fn delete_segment(
    store: &SharedStore,
    note_id: &str,
    segment: u32,
) -> Result<usize, StageError> {
    let prefix = checkpoint::live_prefix(segment);
    let guard = lock(store);
    let keys: Vec<String> = guard
        .checkpoints(note_id, Stage::Transcribe)?
        .into_iter()
        .map(|(k, _)| k)
        .filter(|k| k.starts_with(&prefix))
        .collect();
    for key in &keys {
        guard.delete_checkpoint(note_id, Stage::Transcribe, key)?;
    }
    Ok(keys.len())
}

/// The number of the next part of `segment`.
fn next_part(store: &SharedStore, note_id: &str, segment: u32) -> u32 {
    match segment_rows(store, note_id, segment) {
        Ok(rows) => rows.last().map_or(0, |r| r.part + 1),
        Err(e) => {
            log::warn!("could not read the live rows of {note_id}: {e}");
            0
        }
    }
}

/// One capture frame at the device's rate. `index` is the segment's sample
/// count before the frame.
pub struct LiveFrame {
    pub index: u64,
    pub pcm: Arc<[f32]>,
}

/// The recorder's end of the channel to a live worker.
#[derive(Clone)]
pub struct LiveTap {
    tx: SyncSender<LiveFrame>,
}

impl LiveTap {
    /// A wake-up hint contains no audio. The worker reads the flushed prefix.
    pub fn notify(&self, index: u64) -> bool {
        self.tx
            .try_send(LiveFrame {
                index,
                pcm: Arc::from([]),
            })
            .is_ok()
    }

    /// Offer a frame to the worker without waiting. False when the channel
    /// is full or the worker has stopped: the frame is dropped.
    pub fn offer(&self, index: u64, pcm: &Arc<[f32]>) -> bool {
        self.tx
            .try_send(LiveFrame {
                index,
                pcm: Arc::clone(pcm),
            })
            .is_ok()
    }
}

/// A bounded frame channel of `capacity` frames.
pub fn channel(capacity: usize) -> (LiveTap, Receiver<LiveFrame>) {
    let (tx, rx) = mpsc::sync_channel(capacity);
    (LiveTap { tx }, rx)
}

/// The live worker's way to the runner's engine slot (KTD9).
#[derive(Clone)]
pub struct EngineAccess {
    slot: EngineSlot,
    load: Loader,
    exact: Option<ExactLoader>,
    /// The runner's shutdown: after it, nothing loads a model again.
    shutdown: CancellationToken,
}

type SlotGuard<'a> = MutexGuard<'a, Option<Box<dyn Recognizer>>>;

impl EngineAccess {
    pub fn new(slot: EngineSlot, load: Loader, shutdown: CancellationToken) -> Self {
        Self {
            slot,
            load,
            exact: None,
            shutdown,
        }
    }

    pub fn with_exact_loader(mut self, loader: ExactLoader) -> Self {
        self.exact = Some(loader);
        self
    }
    fn hold_config(&self, config: &LiveConfig) -> Result<Option<SlotGuard<'_>>, String> {
        let Some(spec) = &config.snapshot else {
            return self.hold(&config.model);
        };
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if self.shutdown.is_cancelled() {
            return Ok(None);
        }
        let load = self
            .exact
            .as_ref()
            .ok_or("exact live transcription loader is unavailable")?;
        // The shared legacy slot records a model name, not artifact identity.
        // Reload the snapshot after taking the slot; an alias is not proof.
        *slot = None;
        *slot = Some(load(spec)?);
        Ok(Some(slot))
    }

    /// Lock the slot and make it hold `model`: the engine that the slot
    /// holds when it holds `model`, or `model` loaded in place of what the
    /// slot holds. `None` after the runner's shutdown, which frees the
    /// engine for good.
    pub fn hold(&self, model: &str) -> Result<Option<SlotGuard<'_>>, String> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        // Checked under the lock, as the runner's stages do.
        if self.shutdown.is_cancelled() {
            return Ok(None);
        }
        ensure_model(&mut slot, model, &*self.load)?;
        Ok(Some(slot))
    }
}

/// Load `model` into `slot` when the slot holds another model or none. The
/// old engine is freed before the load, so two models are never loaded at
/// the same time.
pub(crate) fn ensure_model(
    slot: &mut Option<Box<dyn Recognizer>>,
    model: &str,
    load: &crate::transcribe::Load<'_>,
) -> Result<(), String> {
    if slot.as_ref().map(|e| e.model_id()) != Some(model) {
        *slot = None;
        *slot = Some(load(model)?);
    }
    Ok(())
}

/// What the live view shows about the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveStatus {
    /// The worker waits for the engine or loads the model.
    Starting,
    /// Text arrives.
    Running,
    /// The live transcript fell behind: a gap started, or the stream
    /// stalled or failed. The text returns with the next part.
    Behind,
    /// This recording has no live transcript: the model did not load or
    /// does not stream. The note gets a full pass after Stop.
    Unavailable,
    /// The worker stopped.
    Ended,
}

/// Where the worker reports what it does. Every method has an empty
/// default.
pub trait LiveSink: Send {
    /// The worker holds `model` in the engine slot, or holds no model now.
    fn model(&mut self, _held: Option<&str>) {}
    fn status(&mut self, _status: LiveStatus) {}
    fn failed(&mut self, _message: &str) {}
    /// The text of the open part changed. `words` are the open part's timed
    /// words from index `word_start`, in milliseconds from the start of the
    /// segment, with the replay removed. The words before `word_start` did
    /// not change since the last call for this part. A stream that has no
    /// word times gives no words.
    fn text(&mut self, _part: u32, _text: &LiveText, _word_start: u32, _words: &[LiveWord]) {}
    /// A part was saved.
    fn saved(&mut self, _row: &LiveRow) {}
    fn recognition(&mut self, _block: &session_core::recognition::OutputBlock) {}
    /// Speaker batches are staged between begin and end. Only end publishes
    /// the complete replacement timeline for this scope and revision.
    fn snapshot(&mut self, _scope: &asr::SpeakerScope, _revision: u64, _begin: bool) {}
    fn interim_snapshot(&mut self, _scope: &asr::SpeakerScope, _revision: u64, _begin: bool) {}
}

/// A sink that reports nothing.
pub struct Silent;

impl LiveSink for Silent {}

/// One segment's live worker.
#[derive(Debug, Clone)]
pub struct LiveConfig {
    pub note_id: String,
    pub segment: u32,
    /// The note's model (KTD2).
    pub model: String,
    /// The capture rate of the frames.
    pub sample_rate: u32,
    pub stall_ms: u64,
    pub archive: Option<archive::Archive>,
    pub text_enabled: bool,
    pub speakers_enabled: bool,
    pub snapshot: Option<models::recognition::ExecutionSpec>,
}

impl LiveConfig {
    pub fn with_archive(mut self, dir: std::path::PathBuf, offset_ms: u64) -> Self {
        self.archive = Some(archive::Archive {
            dir,
            offset_ms,
            segment: self.segment,
            rate: self.sample_rate,
        });
        self
    }

    pub fn new(note_id: &str, segment: u32, model: &str, sample_rate: u32) -> Self {
        Self {
            note_id: note_id.to_string(),
            segment,
            model: model.to_string(),
            sample_rate: sample_rate.max(1),
            stall_ms: STALL_MS,
            archive: None,
            text_enabled: true,
            speakers_enabled: false,
            snapshot: None,
        }
    }
}

/// How the segment ended, from capture's close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveEnd {
    /// The segment is kept: save the open part.
    Keep,
    /// The segment ran under a second and was discarded: delete its rows.
    Discard,
}

/// What a worker reads.
pub struct Inputs {
    pub frames: Receiver<LiveFrame>,
    /// Sent after the frames end. A closed channel counts as
    /// [`LiveEnd::Keep`]. A dropped [`LiveWorker`] also sets
    /// [`LiveEnd::Keep`] in its [`CaptureEnd`], so every reader sees the
    /// same end.
    pub end: Receiver<LiveEnd>,
    /// A quit: stop at once and save nothing more.
    pub cancel: CancellationToken,
}

enum Ending {
    /// The frames and the end signal arrived.
    Closed,
    Cancelled,
    /// The model does not stream.
    NoStream,
}

/// Run one segment's live worker until its frames end and capture says how
/// the segment ended, or until a quit. Blocks: run it on a thread of its
/// own.
pub fn run(
    store: &SharedStore,
    engine: &EngineAccess,
    config: &LiveConfig,
    parts: Parts,
    inputs: &Inputs,
    sink: &mut dyn LiveSink,
) {
    let id = &config.note_id;
    sink.status(LiveStatus::Starting);
    let mut guard = match engine.hold_config(config) {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            sink.status(LiveStatus::Ended);
            return;
        }
        Err(e) => {
            log::warn!(
                "no live transcript for segment {} of {id}: {e}",
                config.segment
            );
            sink.status(LiveStatus::Unavailable);
            finish_without_stream(store, config, inputs);
            sink.status(LiveStatus::Ended);
            return;
        }
    };
    sink.model(Some(&config.model));
    let part = next_part(store, id, config.segment);
    let ending = {
        let recognizer = guard.as_deref_mut().expect("hold loads the model");
        let mut work = Work::new(store, config, sink, parts, part);
        stream_parts(recognizer, &mut work, inputs)
    };
    drop(guard);
    sink.model(None);
    if let Ending::NoStream = ending {
        log::warn!(
            "{} does not stream; segment {} of {id} gets a full pass after Stop",
            config.model,
            config.segment
        );
        sink.status(LiveStatus::Unavailable);
        finish_without_stream(store, config, inputs);
    }
    sink.status(LiveStatus::Ended);
}

/// Read the frames to their end with no stream, and delete the segment's
/// rows when capture discards it.
fn finish_without_stream(store: &SharedStore, config: &LiveConfig, inputs: &Inputs) {
    loop {
        if inputs.cancel.is_cancelled() {
            return;
        }
        match inputs.frames.recv_timeout(POLL) {
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    if wait_end(inputs) == Some(LiveEnd::Discard) {
        discard(store, config);
    }
}

/// How capture ended the segment. `None` on a quit.
fn wait_end(inputs: &Inputs) -> Option<LiveEnd> {
    loop {
        if inputs.cancel.is_cancelled() {
            return None;
        }
        match inputs.end.recv_timeout(POLL) {
            Ok(end) => return Some(end),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Some(LiveEnd::Keep),
        }
    }
}

fn discard(store: &SharedStore, config: &LiveConfig) {
    match delete_segment(store, &config.note_id, config.segment) {
        Ok(0) => {}
        Ok(n) => log::info!(
            "deleted {n} live part(s) of the discarded segment {} of {}",
            config.segment,
            config.note_id
        ),
        Err(e) => log::warn!(
            "could not delete the live parts of segment {} of {}: {e}",
            config.segment,
            config.note_id
        ),
    }
}

/// The frame loop. Each new stream borrows `recognizer`, so every start
/// comes after the previous stream is dropped.
fn stream_parts(recognizer: &mut dyn Recognizer, w: &mut Work<'_>, inputs: &Inputs) -> Ending {
    let mut stream = match recognizer.start_stream() {
        Ok(Some(stream)) => Some(stream),
        Ok(None) => return Ending::NoStream,
        Err(e) => {
            log::warn!("the live stream did not start: {e}");
            None
        }
    };
    // The next stream starts a new part at the next frame, after a gap.
    let mut fresh = true;
    // Segment time before which a failed start is not tried again.
    let mut retry_at_ms = 0;
    let mut archive = w.config.archive.clone().map(archive::ArchiveReader::new);
    let mut closed = false;
    let mut archive_end = None;
    loop {
        if inputs.cancel.is_cancelled() {
            return Ending::Cancelled;
        }
        let frame = if let Some(archive) = &mut archive {
            match archive.next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) if closed => break,
                Ok(None) => {
                    match inputs.frames.recv_timeout(POLL) {
                        Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => {
                            archive_end = wait_end(inputs);
                            if archive_end.is_none() {
                                return Ending::Cancelled;
                            }
                            closed = true;
                        }
                    }
                    continue;
                }
                Err(error) => {
                    log::warn!("live durable audio read failed: {error}");
                    return Ending::Cancelled;
                }
            }
        } else {
            match inputs.frames.recv_timeout(POLL) {
                Ok(frame) => frame,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };
        let len = frame.pcm.len() as u64;
        let start_ms = w.ms(frame.index);
        let end_ms = w.ms(frame.index + len);

        if let Some(expected) = w.expected
            && frame.index != expected
        {
            log::info!(
                "the live worker dropped audio from {} to {start_ms} ms of segment {}",
                w.ms(expected),
                w.config.segment
            );
            if let Some(s) = stream.as_deref_mut() {
                let tail = w.flush();
                if w.feed(s, &tail, w.ms(expected)).is_ok() {
                    w.finish(s);
                }
            }
            drop(stream);
            stream = None;
            fresh = true;
            retry_at_ms = 0;
            w.set_status(LiveStatus::Behind);
        }
        w.expected = Some(frame.index + len);

        if stream.is_none() && start_ms >= retry_at_ms {
            drop(stream);
            stream = match recognizer.start_stream() {
                Ok(Some(stream)) => Some(stream),
                Ok(None) => return Ending::NoStream,
                Err(e) => {
                    log::warn!("the live stream did not start: {e}");
                    retry_at_ms = start_ms + RETRY_MS;
                    None
                }
            };
        }
        let Some(s) = stream.as_deref_mut() else {
            continue;
        };
        if fresh {
            w.restart(start_ms);
            w.began(start_ms);
            fresh = false;
        }
        let pcm = w.resample(&frame.pcm);
        let before_part = w.part;
        let next = match w.feed(s, &pcm, end_ms) {
            Ok(Some(_)) => {
                w.finish(s);
                Next::Continue
            }
            Ok(None) if w.stalled() => {
                log::warn!(
                    "the live stream of segment {} gave no new text for {} s of speech; \
                     the span is transcribed after Stop",
                    w.config.segment,
                    w.config.stall_ms / 1000
                );
                w.finish_stalled(s);
                w.set_status(LiveStatus::Behind);
                Next::Continue
            }
            Ok(None) => Next::Same,
            Err(e) => {
                log::warn!(
                    "the live stream of segment {} failed at {start_ms} ms: {e}",
                    w.config.segment
                );
                w.finish(s);
                w.set_status(LiveStatus::Behind);
                Next::Fresh
            }
        };
        if w.part != before_part
            && let Some(reader) = &archive
            && let Err(error) = archive::save_proof(w.store, w.config, reader)
        {
            log::warn!("live source proof was not saved: {error}");
            return Ending::Cancelled;
        }
        match next {
            Next::Same => {}
            // The part ended with no gap: the next stream goes on from
            // where it ended.
            Next::Continue => {
                drop(stream);
                stream = match recognizer.start_stream() {
                    Ok(Some(stream)) => {
                        w.began(end_ms);
                        Some(stream)
                    }
                    Ok(None) => return Ending::NoStream,
                    Err(e) => {
                        log::warn!("the live stream did not start again: {e}");
                        fresh = true;
                        retry_at_ms = end_ms + RETRY_MS;
                        None
                    }
                };
            }
            Next::Fresh => {
                drop(stream);
                stream = None;
                fresh = true;
                retry_at_ms = end_ms;
            }
        }
    }

    if let Some(reader) = &archive
        && archive_end == Some(LiveEnd::Keep)
        && let Err(error) = archive::save_proof(w.store, w.config, reader)
    {
        log::warn!("live source proof was not saved: {error}");
        return Ending::Cancelled;
    }
    match archive_end.or_else(|| wait_end(inputs)) {
        None => Ending::Cancelled,
        Some(LiveEnd::Keep) => {
            if let Some(s) = stream.as_deref_mut() {
                let at_ms = w.expected.map_or(0, |e| w.ms(e));
                let tail = w.flush();
                if w.feed(s, &tail, at_ms).is_ok() {
                    w.finish(s);
                }
            }
            Ending::Closed
        }
        Some(LiveEnd::Discard) => {
            if let Some(s) = stream.as_deref_mut() {
                s.reset();
            }
            discard(w.store, w.config);
            Ending::Closed
        }
    }
}

enum Next {
    /// Go on with the same stream.
    Same,
    /// The part ended: start the next stream at once.
    Continue,
    /// The stream failed: start the next stream at the next frame.
    Fresh,
}

/// A worker's state apart from its stream.
struct Work<'a> {
    store: &'a SharedStore,
    config: &'a LiveConfig,
    sink: &'a mut dyn LiveSink,
    parts: Parts,
    resampler: StreamResampler,
    /// The number of the open part.
    part: u32,
    /// The index of the next frame when no frame is dropped.
    expected: Option<u64>,
    /// The committed text of the open stream.
    committed: String,
    /// [`Parts::voiced_ms`] when the committed text last changed.
    voiced_at_commit: u64,
    /// Segment time when the committed text last changed.
    commit_ms: u64,
    /// The open-part words that the sink has, in segment time.
    sent_words: Vec<LiveWord>,
    status: LiveStatus,
}

impl<'a> Work<'a> {
    fn new(
        store: &'a SharedStore,
        config: &'a LiveConfig,
        sink: &'a mut dyn LiveSink,
        parts: Parts,
        part: u32,
    ) -> Self {
        Self {
            store,
            config,
            sink,
            parts,
            resampler: StreamResampler::new(config.sample_rate, asr::SAMPLE_RATE),
            part,
            expected: None,
            committed: String::new(),
            voiced_at_commit: 0,
            commit_ms: 0,
            sent_words: Vec::new(),
            status: LiveStatus::Starting,
        }
    }

    /// Segment time of a sample index at the capture rate.
    fn ms(&self, index: u64) -> u64 {
        index * 1000 / u64::from(self.config.sample_rate)
    }

    fn set_status(&mut self, status: LiveStatus) {
        if self.status != status {
            self.status = status;
            self.sink.status(status);
        }
    }

    /// Start the next part at `at_ms`, after a gap: nothing of the audio
    /// before the gap goes into it.
    fn restart(&mut self, at_ms: u64) {
        self.resampler = StreamResampler::new(self.config.sample_rate, asr::SAMPLE_RATE);
        self.parts.restart_at(at_ms);
    }

    /// A new stream starts at segment time `at_ms`.
    fn began(&mut self, at_ms: u64) {
        self.committed.clear();
        self.sent_words.clear();
        self.voiced_at_commit = self.parts.voiced_ms();
        self.commit_ms = at_ms;
    }

    fn resample(&mut self, pcm: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(pcm.len());
        self.resampler
            .push(pcm, &mut |s: &[f32]| out.extend_from_slice(s));
        out
    }

    /// The audio still in the resampler, before a gap or the end.
    fn flush(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        self.resampler
            .finish(&mut |s: &[f32]| out.extend_from_slice(s));
        out
    }

    /// Feed 16 kHz audio that ends at segment time `at_ms`. Returns the
    /// cut when the part must end now.
    fn feed(
        &mut self,
        stream: &mut dyn LiveStream,
        pcm: &[f32],
        at_ms: u64,
    ) -> Result<Option<Cut>, AsrError> {
        if pcm.is_empty() {
            return Ok(None);
        }
        let fed = self.parts.feed(stream, pcm)?;
        if fed.changed {
            let text = stream.text();
            if text.committed != self.committed {
                self.committed.clone_from(&text.committed);
                self.voiced_at_commit = self.parts.voiced_ms();
                self.commit_ms = at_ms;
            }
            if !text.committed.is_empty() || !text.tentative.is_empty() {
                self.set_status(LiveStatus::Running);
            }
            let words = self.parts.open_words(stream);
            let same = self
                .sent_words
                .iter()
                .zip(&words)
                .take_while(|(a, b)| a == b)
                .count();
            let word_start = u32::try_from(same).unwrap_or(u32::MAX);
            self.sink.text(self.part, &text, word_start, &words[same..]);
            self.sent_words = words;
        }
        Ok(fed.cut)
    }

    fn stalled(&self) -> bool {
        self.parts.voiced_ms().saturating_sub(self.voiced_at_commit) >= self.config.stall_ms
    }

    /// Finish the open part and save it. A stream that fails to finish
    /// loses its part: that span is a gap.
    fn finish(&mut self, stream: &mut dyn LiveStream) {
        match self.parts.finish(stream) {
            Ok(part) => self.save(part),
            Err(e) => log::warn!(
                "the live part {} of segment {} was lost: {e}",
                self.part,
                self.config.segment
            ),
        }
    }

    /// Finish a stalled part. The saved part ends where its committed text
    /// last changed, so the stalled span is a gap.
    fn finish_stalled(&mut self, stream: &mut dyn LiveStream) {
        match self.parts.finish(stream) {
            Ok(mut part) => {
                let end = self.commit_ms.clamp(part.start_ms, part.end_ms);
                part.words.retain(|w| w.start_ms < end);
                for word in &mut part.words {
                    word.end_ms = word.end_ms.min(end);
                }
                part.end_ms = end;
                self.save(part);
            }
            Err(e) => log::warn!(
                "the stalled live part {} of segment {} was lost: {e}",
                self.part,
                self.config.segment
            ),
        }
    }

    /// Save a finished part (the step), then count it and tell the sink
    /// (the record). A part that heard nothing is not saved.
    fn save(&mut self, part: LivePart) {
        if part.end_ms <= part.start_ms {
            return;
        }
        let id = &self.config.note_id;
        match save_part(self.store, id, self.config.segment, self.part, &part) {
            Ok(row) => {
                self.part += 1;
                self.sent_words.clear();
                self.sink.saved(&row);
            }
            Err(e) => log::warn!(
                "could not save the live part {} of segment {} of {id}: {e}",
                self.part,
                self.config.segment
            ),
        }
    }
}

/// Runs live workers on threads of their own, one after another, and says
/// while one runs.
pub struct Workers {
    store: SharedStore,
    engine: EngineAccess,
    running: Arc<AtomicUsize>,
    cancel: CancellationToken,
    /// The last worker started. Each worker waits for the one before it.
    last: Mutex<Option<JoinHandle<()>>>,
    /// Called when the last running worker ends.
    idle: Option<Idle>,
    recognition: Option<RecognitionWork>,
}

type Idle = Arc<dyn Fn() + Send + Sync>;
pub type CaptureEnd = Arc<Mutex<Option<LiveEnd>>>;
pub type RecognitionWork = Arc<
    dyn Fn(
            &SharedStore,
            &LiveConfig,
            &CancellationToken,
            &CaptureEnd,
            &mut dyn LiveSink,
        ) -> Result<(), StageError>
        + Send
        + Sync,
>;
#[derive(Clone)]
struct SharedSink(Arc<Mutex<Box<dyn LiveSink>>>);
impl LiveSink for SharedSink {
    fn interim_snapshot(&mut self, scope: &asr::SpeakerScope, revision: u64, begin: bool) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .interim_snapshot(scope, revision, begin);
    }
    fn failed(&mut self, message: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .failed(message);
    }
    fn snapshot(&mut self, scope: &asr::SpeakerScope, revision: u64, begin: bool) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot(scope, revision, begin);
    }
    fn model(&mut self, held: Option<&str>) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).model(held);
    }
    fn status(&mut self, status: LiveStatus) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status(status);
    }
    fn text(&mut self, part: u32, text: &LiveText, word_start: u32, words: &[LiveWord]) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .text(part, text, word_start, words);
    }
    fn saved(&mut self, row: &LiveRow) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).saved(row);
    }
    fn recognition(&mut self, block: &session_core::recognition::OutputBlock) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recognition(block);
    }
}

/// Capture's end of a running worker.
pub struct LiveWorker {
    end: mpsc::Sender<LiveEnd>,
    capture_end: CaptureEnd,
}

impl LiveWorker {
    /// Tell the worker how its segment ended. Call it after the frames end.
    pub fn end(self, how: LiveEnd) {
        *self.capture_end.lock().unwrap_or_else(|e| e.into_inner()) = Some(how);
        let _ = self.end.send(how);
    }
}

impl Drop for LiveWorker {
    /// A worker that drops with no [`LiveWorker::end`] ends as
    /// [`LiveEnd::Keep`], as the closed channel does. Capture drops the
    /// worker when it closes a segment with an error.
    fn drop(&mut self) {
        let mut how = self.capture_end.lock().unwrap_or_else(|e| e.into_inner());
        if how.is_none() {
            *how = Some(LiveEnd::Keep);
        }
    }
}

/// Counts a worker as running until it drops.
struct Running(Arc<AtomicUsize>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Workers {
    pub fn new(store: SharedStore, engine: EngineAccess) -> Self {
        Self {
            store,
            engine,
            running: Arc::new(AtomicUsize::new(0)),
            cancel: CancellationToken::new(),
            last: Mutex::new(None),
            idle: None,
            recognition: None,
        }
    }

    pub fn with_recognition(mut self, recognition: RecognitionWork) -> Self {
        self.recognition = Some(recognition);
        self
    }

    /// Call `idle` each time the last running worker ends: processing can
    /// start again.
    pub fn with_idle(mut self, idle: impl Fn() + Send + Sync + 'static) -> Self {
        self.idle = Some(Arc::new(idle));
        self
    }

    /// Start a worker for one segment. It counts as running from now, so
    /// processing is suspended before the first frame.
    pub fn start(
        &self,
        config: LiveConfig,
        parts: Parts,
        sink: Box<dyn LiveSink>,
    ) -> std::io::Result<(LiveTap, LiveWorker)> {
        let (tap, frames) = channel(CHANNEL_FRAMES);
        let capture_end: CaptureEnd = Arc::new(Mutex::new(None));
        let work_end = capture_end.clone();
        let recognition = self.recognition.clone();
        let mut sink = SharedSink(Arc::new(Mutex::new(sink)));
        let (end, ends) = mpsc::channel();
        let inputs = Inputs {
            frames,
            end: ends,
            cancel: self.cancel.child_token(),
        };
        self.running.fetch_add(1, Ordering::SeqCst);
        let running = Running(Arc::clone(&self.running));
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let previous = last.take();
        let (store, engine) = (Arc::clone(&self.store), self.engine.clone());
        let (count, idle) = (Arc::clone(&self.running), self.idle.clone());
        let name = format!("longhand-live-{}", config.segment);
        let thread = std::thread::Builder::new().name(name).spawn(move || {
            if let Some(previous) = previous {
                let _ = previous.join();
            }
            std::thread::scope(|scope| {
                if config.speakers_enabled {
                    let mut native_sink = sink.clone();
                    let native_store = &store;
                    let native_config = &config;
                    let cancel = &inputs.cancel;
                    scope.spawn(move || {
                        let result = recognition
                            .ok_or_else(|| {
                                StageError::Failed("live speaker worker is unavailable".into())
                            })
                            .and_then(|work| {
                                work(
                                    native_store,
                                    native_config,
                                    cancel,
                                    &work_end,
                                    &mut native_sink,
                                )
                            });
                        if let Err(error) = result {
                            let message = error.to_string();
                            let _ = checkpoint::put(
                                native_store,
                                &native_config.note_id,
                                Stage::Transcribe,
                                &format!("live-failure-{:08}", native_config.segment),
                                &message,
                            );
                            native_sink.failed(&message);
                            log::warn!("live speaker analysis stopped: {error}");
                        }
                    });
                }
                if config.text_enabled {
                    run(&store, &engine, &config, parts, &inputs, &mut sink);
                } else {
                    finish_without_stream(&store, &config, &inputs);
                }
            });
            drop(running);
            if count.load(Ordering::SeqCst) == 0
                && let Some(idle) = idle
            {
                idle();
            }
        })?;
        *last = Some(thread);
        Ok((tap, LiveWorker { end, capture_end }))
    }

    /// True while a worker runs: it streams, or it saves its last part
    /// after Stop.
    pub fn busy(&self) -> bool {
        self.running.load(Ordering::SeqCst) > 0
    }

    /// Stop every worker and wait for it to release the engine slot. The
    /// open part of a worker is not saved: it is a gap. Call before the
    /// runner's shutdown frees the engine.
    pub fn shutdown(&self) {
        self.cancel.cancel();
        let last = self.last.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(last) = last {
            let _ = last.join();
        }
    }
}

/// Execute the selected native live task. The runner and paced acceptance
/// harness use the same entry point and exact artifact provider.
pub fn recognize(
    store: &SharedStore,
    config: &LiveConfig,
    provider: &dyn crate::EngineProvider,
    cancel: &CancellationToken,
    end: &CaptureEnd,
    sink: &mut dyn LiveSink,
) -> Result<(), StageError> {
    if config.snapshot.as_ref().is_some_and(|spec| {
        spec.config.speaker_source == models::recognition::SpeakerSource::Pyannote
    }) {
        let mut analyzer = provider
            .analyzer_exact(config.snapshot.as_ref().unwrap())
            .map_err(|e| StageError::Failed(e.to_string()))?;
        features::extract(store, config, analyzer.as_mut(), cancel, end)
    } else {
        crate::recognition::nemo::run_live(store, config, provider, cancel, end, sink)
    }
}

/// What a worker that reads the archive of its segment does next.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step<T> {
    /// Process this frame.
    Frame(T),
    /// Capture ended the segment and no frame is left: finish.
    Finish,
    /// The recorder has not closed the next chunk yet: wait and read again.
    Wait,
}

/// Decide the next step of a worker. Read the end before the frame.
/// Capture closes the last chunk before it sets the end. Thus, when the
/// end is set before the read and the read finds no frame, the worker
/// finishes. When the end is set during the read, the worker waits and
/// reads again, so it also reads the last chunk.
pub(crate) fn next_step<T, E>(
    end: &CaptureEnd,
    read: impl FnOnce() -> Result<Option<T>, E>,
) -> Result<Step<T>, E> {
    let ended = end.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    Ok(match read()? {
        Some(frame) => Step::Frame(frame),
        None if ended => Step::Finish,
        None => Step::Wait,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn a_chunk_that_capture_closes_during_the_read_is_read_before_the_finish() {
        let end: CaptureEnd = Arc::new(Mutex::new(None));
        let queued = Mutex::new(VecDeque::new());
        // The first read finds no frame. Capture then closes the last
        // chunk and sets the end before the worker reads the end.
        let read = || -> Result<Option<u32>, ()> {
            let mut queued = queued.lock().unwrap();
            if let Some(frame) = queued.pop_front() {
                return Ok(Some(frame));
            }
            let mut how = end.lock().unwrap();
            if how.is_none() {
                *how = Some(LiveEnd::Keep);
                queued.push_back(7);
            }
            Ok(None)
        };
        assert_eq!(next_step(&end, read), Ok(Step::Wait));
        assert_eq!(next_step(&end, read), Ok(Step::Frame(7)));
        assert_eq!(next_step(&end, read), Ok(Step::Finish));
    }
}
