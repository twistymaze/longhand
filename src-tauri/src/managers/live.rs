//! Wires the live worker (U6, KTD3, KTD9, KTD10) into the app: one worker
//! for each capture segment of a note in live mode, its events out to the
//! surface, and the catch-up text for a Transcript tab that opens during
//! recording.
//!
//! The worker itself is `pipeline::live`. It shares the processing
//! runner's engine slot, so this manager gets the workers from the
//! processing manager once the runner exists.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use asr::{LiveText, PartPolicy, Parts};
use pipeline::live::{
    LiveConfig, LiveEnd, LiveRow, LiveSink, LiveStatus, LiveTap, LiveWorker, Workers,
};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use tauri_specta::Event as _;

use crate::managers::models::ModelsManager;

type StartLive = Box<dyn FnOnce() -> Option<(LiveTap, LiveWorker)> + Send>;

/// Capture writes audio before the session row exists. Start its live worker
/// only after that row is saved. The worker reads early audio from the archive.
pub(super) struct DeferredLive {
    start: Option<StartLive>,
    tap: Arc<OnceLock<LiveTap>>,
    worker: Option<LiveWorker>,
}

impl DeferredLive {
    pub(super) fn new(
        start: impl FnOnce() -> Option<(LiveTap, LiveWorker)> + Send + 'static,
    ) -> Self {
        Self {
            start: Some(Box::new(start)),
            tap: Arc::new(OnceLock::new()),
            worker: None,
        }
    }

    pub(super) fn tap(&self) -> Arc<OnceLock<LiveTap>> {
        Arc::clone(&self.tap)
    }

    pub(super) fn activate(&mut self) {
        if let Some(start) = self.start.take()
            && let Some((tap, worker)) = start()
        {
            let _ = self.tap.set(tap);
            self.worker = Some(worker);
        }
    }

    pub(super) fn end(self, how: LiveEnd) {
        if let Some(worker) = self.worker {
            worker.end(how);
        }
    }
}

/// Durable text or speaker revisions on the canonical 16 kHz timeline.
#[derive(Serialize, Clone, Type, Debug, PartialEq, Eq, tauri_specta::Event)]
pub struct LiveRecognitionEvent {
    pub session_id: String,
    pub phase: LiveRecognitionPhase,
    pub key: String,
    pub generation: String,
    pub epoch: u64,
    pub revision: u64,
    pub rows: Vec<LiveRecognitionRow>,
}
#[derive(Serialize, Clone, Copy, Type, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LiveRecognitionPhase {
    Patch,
    InterimBegin,
    InterimBatch,
    InterimEnd,
    SnapshotBegin,
    SnapshotBatch,
    SnapshotEnd,
    Failed,
}
#[derive(Serialize, Clone, Type, Debug, PartialEq, Eq)]
pub struct LiveRecognitionRow {
    pub start_sample: u64,
    pub end_sample: u64,
    pub text: Option<String>,
    pub speaker: Option<String>,
    pub text_final: bool,
    pub speaker_final: bool,
}

/// The text of the open part changed (KTD10). `committed` does not change
/// again; `tentative` can change with the next event.
///
/// `words` are the timed words of the open part from index `word_start`
/// (KTD3). The words before `word_start` did not change. A new part starts
/// at index 0.
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct LiveTranscriptEvent {
    pub session_id: String,
    pub segment: u32,
    pub part: u32,
    /// The start of the segment, in milliseconds from the start of the note.
    pub offset_ms: u64,
    pub committed: String,
    pub tentative: String,
    pub word_start: u32,
    pub words: Vec<LiveWord>,
}

/// A part of the live transcript was saved. The open part of the next
/// event starts after it. The part has the same shape as a part of the
/// catch-up (KTD4).
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct LivePartSaved {
    pub session_id: String,
    #[serde(flatten)]
    pub part: LivePartText,
}

/// What the live worker does now, for the lines of the live view (U8).
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct LiveTranscriptStatus {
    pub session_id: String,
    pub segment: u32,
    pub state: LiveState,
}

#[derive(Serialize, Clone, Copy, Type, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LiveState {
    /// The model loads, or the worker waits for the engine.
    Starting,
    Running,
    /// A gap started: the missing audio is transcribed after Stop.
    Paused,
    /// No live transcript for this segment: it is transcribed after Stop.
    Unavailable,
    Ended,
}

impl From<LiveStatus> for LiveState {
    fn from(status: LiveStatus) -> Self {
        match status {
            LiveStatus::Starting => Self::Starting,
            LiveStatus::Running => Self::Running,
            LiveStatus::Paused => Self::Paused,
            LiveStatus::Unavailable => Self::Unavailable,
            LiveStatus::Ended => Self::Ended,
        }
    }
}

/// One timed word, in milliseconds from the start of its segment.
#[derive(Serialize, Clone, Type, Debug, PartialEq, Eq)]
pub struct LiveWord {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

impl From<&asr::LiveWord> for LiveWord {
    fn from(word: &asr::LiveWord) -> Self {
        Self {
            start_ms: word.start_ms,
            end_ms: word.end_ms,
            text: word.text.clone(),
        }
    }
}

/// One saved part of the live transcript.
#[derive(Serialize, Clone, Type, Debug, PartialEq, Eq)]
pub struct LivePartText {
    pub segment: u32,
    pub part: u32,
    /// The start of the segment, in milliseconds from the start of the note.
    pub offset_ms: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// In milliseconds from the start of the segment.
    pub words: Vec<LiveWord>,
}

impl LivePartText {
    fn new(row: &LiveRow, offset_ms: u64) -> Self {
        Self {
            segment: row.segment,
            part: row.part,
            offset_ms,
            start_ms: row.start_ms,
            end_ms: row.end_ms,
            text: row.text(),
            words: row.words.iter().map(LiveWord::from).collect(),
        }
    }
}

/// The text of the part that is open now.
#[derive(Serialize, Clone, Type, Debug, PartialEq, Eq)]
pub struct LiveCurrent {
    pub segment: u32,
    pub part: u32,
    /// The start of the segment, in milliseconds from the start of the note.
    pub offset_ms: u64,
    pub committed: String,
    pub tentative: String,
    /// All timed words of the open part, in milliseconds from the start of
    /// the segment.
    pub words: Vec<LiveWord>,
}

/// The live transcript so far: the catch-up for a tab that opens during
/// recording (KTD10).
#[derive(Serialize, Clone, Type, Debug, PartialEq, Eq)]
pub struct LiveTranscript {
    /// Durable native output for a tab opened after processing began.
    pub recognition: Vec<LiveRecognitionEvent>,
    pub parts: Vec<LivePartText>,
    /// `None` when no worker of this note has text open.
    pub current: Option<LiveCurrent>,
    /// `None` when no worker of this note ran in this process.
    pub state: Option<LiveState>,
}

/// What the last worker reported.
#[derive(Clone)]
struct Now {
    session_id: String,
    current: LiveCurrent,
    state: LiveState,
    failure: Option<LiveRecognitionEvent>,
}

/// The segment offsets that `start` received, by note and segment. The
/// open segment has no segment row until capture closes it (KTD4).
type Offsets = BTreeMap<(String, u32), u64>;

pub struct LiveManager {
    models: Arc<ModelsManager>,
    workers: OnceLock<Workers>,
    now: Arc<Mutex<Option<Now>>>,
    offsets: Mutex<Offsets>,
}

impl LiveManager {
    pub fn new(models: Arc<ModelsManager>) -> Self {
        Self {
            models,
            workers: OnceLock::new(),
            now: Arc::new(Mutex::new(None)),
            offsets: Mutex::new(BTreeMap::new()),
        }
    }

    /// Keep the offset of a segment that starts. Only the offsets of the
    /// newest note stay: the segments of an earlier note have their rows.
    fn keep_offset(&self, session_id: &str, segment: u32, offset_ms: u64) {
        let mut offsets = self.offsets.lock().unwrap_or_else(|e| e.into_inner());
        offsets.retain(|(id, _), _| id == session_id);
        offsets.insert((session_id.to_string(), segment), offset_ms);
    }

    /// Give the manager its workers, once the runner exists.
    pub fn attach(&self, workers: Workers) {
        if self.workers.set(workers).is_err() {
            log::warn!("the live workers were already attached");
        }
    }

    /// True while a worker streams or saves its last part: processing
    /// waits for it (KTD9).
    pub fn busy(&self) -> bool {
        self.workers.get().is_some_and(Workers::busy)
    }

    /// Before the process exits, and before the runner frees the engine:
    /// stop every worker and release the engine slot.
    pub fn shutdown(&self) {
        if let Some(workers) = self.workers.get() {
            workers.shutdown();
        }
    }

    /// Start the live worker for a segment that records with `model` at
    /// `sample_rate`. `None` when no worker can start: the segment then
    /// gets a full pass after Stop.
    #[allow(
        clippy::too_many_arguments,
        reason = "Capture supplies the segment, durable audio, and immutable settings together"
    )]
    pub fn start(
        &self,
        app: &AppHandle,
        session_id: &str,
        segment: u32,
        model: &str,
        sample_rate: u32,
        archive: std::path::PathBuf,
        offset_ms: u64,
        snapshot: Option<models::recognition::ExecutionSpec>,
    ) -> Option<(LiveTap, LiveWorker)> {
        self.keep_offset(session_id, segment, offset_ms);
        let sink = || AppSink {
            app: app.clone(),
            models: Arc::clone(&self.models),
            report: Report {
                now: Arc::clone(&self.now),
                session_id: session_id.to_string(),
                segment,
                offset_ms,
                part: 0,
            },
        };
        let Some(workers) = self.workers.get() else {
            log::warn!("no live workers are attached; {session_id} is transcribed after Stop");
            sink().status(LiveStatus::Unavailable);
            return None;
        };
        let mut config = LiveConfig::new(session_id, segment, model, sample_rate)
            .with_archive(archive, offset_ms);
        if let Some(spec) = snapshot {
            config.text_enabled = spec.enabled_live_transcription;
            config.speakers_enabled = spec.enabled_live_speaker_analysis;
            if pipeline::recognition::TaskGraph::resolve(&spec)
                .is_ok_and(|graph| graph.route == pipeline::recognition::Route::CombinedNemo)
            {
                config.text_enabled = false;
                config.speakers_enabled =
                    spec.enabled_live_transcription || spec.enabled_live_speaker_analysis;
            }
            config.snapshot = Some(spec);
        }
        match workers.start(
            config,
            Parts::new(0, PartPolicy::default()),
            Box::new(sink()),
        ) {
            Ok(started) => {
                log::info!("live transcript of {session_id} starts for segment {segment}");
                Some(started)
            }
            Err(e) => {
                log::warn!("the live worker for {session_id} did not start: {e}");
                // The surface shows that the transcript comes after Stop,
                // not a start that never finishes.
                sink().status(LiveStatus::Unavailable);
                None
            }
        }
    }

    /// The saved parts of a note and the text open now.
    pub fn transcript(
        &self,
        store: &pipeline::SharedStore,
        session_id: &str,
    ) -> Result<LiveTranscript, String> {
        // A segment row is canonical. The open segment has only the offset
        // that `start` received.
        let mut offsets: BTreeMap<u32, u64> = self
            .offsets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|((id, _), _)| id == session_id)
            .map(|((_, segment), offset)| (*segment, *offset))
            .collect();
        let segments = store
            .lock()
            .map_err(|e| e.to_string())?
            .segments(session_id)
            .map_err(|e| e.to_string())?;
        offsets.extend(segments.iter().map(|s| (s.index, s.offset_ms)));
        let parts = pipeline::live::rows(store, session_id)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|row| {
                let offset = offsets.get(&row.segment).copied().unwrap_or_else(|| {
                    log::warn!(
                        "no offset is known for segment {} of {session_id}; its live words use 0",
                        row.segment
                    );
                    0
                });
                LivePartText::new(row, offset)
            })
            .collect();
        let now = self
            .now
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .filter(|now| now.session_id == session_id);
        let mut recognition = recognition_catchup(store, session_id)?;
        if let Some(failure) = now.as_ref().and_then(|n| n.failure.clone()) {
            recognition.push(failure);
        }
        Ok(LiveTranscript {
            recognition,
            parts,
            current: now
                .as_ref()
                .filter(|n| !n.current.committed.is_empty() || !n.current.tentative.is_empty())
                .map(|n| n.current.clone()),
            state: now.map(|n| n.state),
        })
    }
}

/// Reports a worker's work to the surface and to the models manager.
struct AppSink {
    app: AppHandle,
    models: Arc<ModelsManager>,
    report: Report,
}

/// Keeps what one worker reported in `Now`, and makes its event payloads.
struct Report {
    now: Arc<Mutex<Option<Now>>>,
    session_id: String,
    segment: u32,
    /// The start of the segment, in milliseconds from the start of the note.
    offset_ms: u64,
    /// The number of the open part.
    part: u32,
}

impl Report {
    fn update(&self, change: impl FnOnce(&mut Now)) {
        let mut now = self.now.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = || Now {
            session_id: self.session_id.clone(),
            current: self.empty(self.part),
            state: LiveState::Starting,
            failure: None,
        };
        let entry = now.get_or_insert_with(fresh);
        if entry.session_id != self.session_id || entry.current.segment != self.segment {
            *entry = fresh();
        }
        change(entry);
    }

    fn empty(&self, part: u32) -> LiveCurrent {
        LiveCurrent {
            segment: self.segment,
            part,
            offset_ms: self.offset_ms,
            committed: String::new(),
            tentative: String::new(),
            words: Vec::new(),
        }
    }

    /// Keep the full word list of the open part: the words before
    /// `word_start`, then `words` (KTD3).
    fn text(
        &mut self,
        part: u32,
        text: &LiveText,
        word_start: u32,
        words: &[asr::LiveWord],
    ) -> LiveTranscriptEvent {
        self.part = part;
        let words: Vec<LiveWord> = words.iter().map(LiveWord::from).collect();
        self.update(|now| {
            if now.current.part != part {
                now.current = self.empty(part);
            }
            let current = &mut now.current;
            current.committed = text.committed.clone();
            current.tentative = text.tentative.clone();
            current
                .words
                .truncate(usize::try_from(word_start).unwrap_or(usize::MAX));
            current.words.extend(words.iter().cloned());
        });
        LiveTranscriptEvent {
            session_id: self.session_id.clone(),
            segment: self.segment,
            part,
            offset_ms: self.offset_ms,
            committed: text.committed.clone(),
            tentative: text.tentative.clone(),
            word_start,
            words,
        }
    }

    fn saved(&mut self, row: &LiveRow) -> LivePartSaved {
        self.part = row.part + 1;
        let next = self.empty(self.part);
        self.update(|now| now.current = next);
        LivePartSaved {
            session_id: self.session_id.clone(),
            part: LivePartText::new(row, self.offset_ms),
        }
    }
}

impl LiveSink for AppSink {
    fn interim_snapshot(&mut self, scope: &asr::SpeakerScope, revision: u64, begin: bool) {
        let _ = LiveRecognitionEvent {
            session_id: self.report.session_id.clone(),
            phase: if begin {
                LiveRecognitionPhase::InterimBegin
            } else {
                LiveRecognitionPhase::InterimEnd
            },
            key: "interim".into(),
            generation: scope.generation.clone(),
            epoch: scope.epoch,
            revision,
            rows: vec![],
        }
        .emit(&self.app);
    }

    fn failed(&mut self, message: &str) {
        let failure = LiveRecognitionEvent {
            session_id: self.report.session_id.clone(),
            phase: LiveRecognitionPhase::Failed,
            key: "failure".into(),
            generation: String::new(),
            epoch: 0,
            revision: 0,
            rows: vec![LiveRecognitionRow {
                start_sample: 0,
                end_sample: 0,
                text: Some(message.into()),
                speaker: None,
                text_final: false,
                speaker_final: false,
            }],
        };
        self.report
            .update(|now| now.failure = Some(failure.clone()));
        let _ = failure.emit(&self.app);
    }

    fn snapshot(&mut self, scope: &asr::SpeakerScope, revision: u64, begin: bool) {
        let _ = LiveRecognitionEvent {
            session_id: self.report.session_id.clone(),
            phase: if begin {
                LiveRecognitionPhase::SnapshotBegin
            } else {
                LiveRecognitionPhase::SnapshotEnd
            },
            key: format!("speakers/{}", scope.epoch),
            generation: scope.generation.clone(),
            epoch: scope.epoch,
            revision,
            rows: vec![],
        }
        .emit(&self.app);
    }

    fn recognition(&mut self, block: &session_core::recognition::OutputBlock) {
        let _ = recognition_event(&self.report.session_id, block).emit(&self.app);
    }

    fn model(&mut self, held: Option<&str>) {
        // Delete refuses the model of the live worker (KTD12).
        self.models.set_live_model(held.map(str::to_string));
    }

    fn status(&mut self, status: LiveStatus) {
        let state = LiveState::from(status);
        self.report.update(|now| now.state = state);
        let _ = LiveTranscriptStatus {
            session_id: self.report.session_id.clone(),
            segment: self.report.segment,
            state,
        }
        .emit(&self.app);
    }

    fn text(&mut self, part: u32, text: &LiveText, word_start: u32, words: &[asr::LiveWord]) {
        let _ = self
            .report
            .text(part, text, word_start, words)
            .emit(&self.app);
    }

    fn saved(&mut self, row: &LiveRow) {
        let _ = self.report.saved(row).emit(&self.app);
    }
}

fn recognition_catchup(
    store: &pipeline::SharedStore,
    session_id: &str,
) -> Result<Vec<LiveRecognitionEvent>, String> {
    let generation: Option<String> = pipeline::checkpoint::get(
        store,
        session_id,
        session_core::Stage::Transcribe,
        "recognition-generation-v1",
    )
    .map_err(|e| e.to_string())?;
    let Some(generation) = generation else {
        return Ok(vec![]);
    };
    let store = store.lock().map_err(|e| e.to_string())?;
    let mut events = Vec::new();
    let mut completed_speakers = std::collections::BTreeMap::new();
    let mut batches = std::collections::BTreeMap::<(u64, bool), Vec<LiveRecognitionEvent>>::new();
    pipeline::recognition::durable::visit_catchup_blocks(
        &store,
        session_id,
        &generation,
        |block| {
            if block.key == format!("nemo/{:020}/turn-commit", block.scope.epoch) {
                completed_speakers.insert(block.scope.epoch, block.revision);
                batches.entry((block.scope.epoch, true)).or_default();
                return Ok(());
            }
            let event = recognition_event(session_id, &block);
            match event.phase {
                LiveRecognitionPhase::Patch => events.push(event),
                LiveRecognitionPhase::SnapshotBatch => {
                    batches.entry((event.epoch, true)).or_default().push(event)
                }
                LiveRecognitionPhase::InterimBatch => {
                    batches.entry((event.epoch, false)).or_default().push(event)
                }
                _ => {}
            }
            Ok(())
        },
    )
    .map_err(|e| e.to_string())?;
    for ((epoch, speakers), mut batch) in batches {
        let revision = if speakers {
            if let Some(revision) = completed_speakers.get(&epoch) {
                batch.retain(|event| event.revision == *revision);
                *revision
            } else {
                // Native turns are provisional until their completion marker commits.
                batch.retain(|event| !event.key.starts_with("nemo/"));
                if batch.is_empty() {
                    continue;
                }
                batch
                    .iter()
                    .map(|event| event.revision)
                    .max()
                    .unwrap_or_default()
            }
        } else {
            batch
                .iter()
                .map(|event| event.revision)
                .max()
                .unwrap_or_default()
        };
        let marker = LiveRecognitionEvent {
            session_id: session_id.into(),
            phase: if speakers {
                LiveRecognitionPhase::SnapshotBegin
            } else {
                LiveRecognitionPhase::InterimBegin
            },
            key: String::new(),
            generation: generation.clone(),
            epoch,
            revision,
            rows: vec![],
        };
        events.push(marker.clone());
        // Catch-up publishes the full durable set as one completed snapshot.
        for event in &mut batch {
            event.revision = revision;
        }
        events.extend(batch);
        events.push(LiveRecognitionEvent {
            phase: if speakers {
                LiveRecognitionPhase::SnapshotEnd
            } else {
                LiveRecognitionPhase::InterimEnd
            },
            ..marker
        });
    }
    Ok(events)
}

fn recognition_event(
    session_id: &str,
    block: &session_core::recognition::OutputBlock,
) -> LiveRecognitionEvent {
    let speaker = |key: &asr::SpeakerKey| {
        format!(
            "{}:{}:{}:{}",
            key.scope.generation, key.scope.epoch, key.input_start_sample, key.local_id
        )
    };
    let mut rows: Vec<_> = block
        .rows
        .iter()
        .map(|row| LiveRecognitionRow {
            start_sample: row.range.start,
            end_sample: row.range.end,
            text: Some(row.text.clone()),
            speaker: row.speaker.as_ref().map(&speaker),
            text_final: row.text_finality == asr::Finality::Final,
            speaker_final: row.speaker_finality == asr::Finality::Final,
        })
        .collect();
    rows.extend(block.speakers.iter().map(|turn| LiveRecognitionRow {
        start_sample: turn.range.start,
        end_sample: turn.range.end,
        text: None,
        speaker: Some(speaker(&turn.speaker)),
        text_final: false,
        speaker_final: turn.finality == asr::Finality::Final,
    }));
    let phase = if !block.speakers.is_empty() {
        LiveRecognitionPhase::SnapshotBatch
    } else if block.key.contains("/interim/") {
        LiveRecognitionPhase::InterimBatch
    } else {
        LiveRecognitionPhase::Patch
    };
    LiveRecognitionEvent {
        session_id: session_id.into(),
        phase,
        key: if matches!(phase, LiveRecognitionPhase::Patch) && block.rows.len() == 1 {
            let id = &block.rows[0].id;
            format!(
                "row/{}/{}/{}/{}",
                id.scope.generation, id.scope.epoch, id.input_start_sample, id.ordinal
            )
        } else {
            block.key.clone()
        },
        generation: block.scope.generation.clone(),
        epoch: block.scope.epoch,
        revision: block.revision,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_live_starts_once_after_the_session_row_is_saved() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        use std::time::Duration;

        let store = store();
        let key = "recognition-generation-v1";
        // This is the native worker's first durable write. SQLite rejects it
        // before capture saves the session row, even when no audio is ready.
        let early_error = pipeline::checkpoint::put(
            &store,
            "new",
            session_core::Stage::Transcribe,
            key,
            &"generation",
        )
        .unwrap_err();
        assert!(early_error.to_string().contains("FOREIGN KEY"));
        let (sent, received) = mpsc::channel();
        let engine = pipeline::live::EngineAccess::new(
            Arc::new(Mutex::new(None)),
            Arc::new(|_| Err("this test does not load a model".into())),
            tokio_util::sync::CancellationToken::new(),
        );
        let workers = Arc::new(Workers::new(Arc::clone(&store), engine).with_recognition(
            Arc::new(move |store, config, _, _, _| {
                let result = pipeline::checkpoint::put(
                    store,
                    &config.session_id,
                    session_core::Stage::Transcribe,
                    key,
                    &"generation",
                );
                sent.send(result.map_err(|error| error.to_string()))
                    .unwrap();
                Ok(())
            }),
        ));
        let starts = Arc::new(AtomicUsize::new(0));
        let start_count = Arc::clone(&starts);
        let start_workers = Arc::clone(&workers);
        let mut live = DeferredLive::new(move || {
            start_count.fetch_add(1, Ordering::SeqCst);
            let mut config = LiveConfig::new("new", 0, "test", 16_000);
            config.text_enabled = false;
            config.speakers_enabled = true;
            Some(
                start_workers
                    .start(
                        config,
                        Parts::new(0, PartPolicy::default()),
                        Box::new(pipeline::live::Silent),
                    )
                    .unwrap(),
            )
        });
        let tap = live.tap();
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        assert!(!workers.busy());
        assert!(tap.get().is_none());
        assert_eq!(received.try_recv(), Err(mpsc::TryRecvError::Empty));

        store
            .lock()
            .unwrap()
            .create_session("new", chrono::Local::now().fixed_offset(), None)
            .unwrap();
        live.activate();
        live.activate();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert!(tap.get().is_some());
        received
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(
            pipeline::checkpoint::get::<String>(
                &store,
                "new",
                session_core::Stage::Transcribe,
                key,
            )
            .unwrap(),
            Some("generation".into()),
        );
        drop(tap);
        live.end(LiveEnd::Keep);
        workers.shutdown();
    }

    #[test]
    fn deferred_live_abandoned_before_persistence_never_starts() {
        for discard in [false, true] {
            let live =
                DeferredLive::new(|| panic!("an abandoned segment must not start live work"));
            let tap = live.tap();
            if discard {
                live.end(LiveEnd::Discard);
            } else {
                drop(live);
            }
            assert!(tap.get().is_none());
        }
    }

    fn manager(dir: &std::path::Path) -> LiveManager {
        LiveManager::new(Arc::new(ModelsManager::at(dir.to_path_buf())))
    }

    fn store() -> pipeline::SharedStore {
        let store = session_core::Store::open_in_memory().unwrap();
        Arc::new(Mutex::new(store))
    }

    fn now(session_id: &str, committed: &str) -> Now {
        Now {
            failure: None,
            session_id: session_id.into(),
            current: LiveCurrent {
                segment: 0,
                part: 2,
                offset_ms: 0,
                committed: committed.into(),
                tentative: "and".into(),
                words: vec![],
            },
            state: LiveState::Running,
        }
    }

    #[test]
    fn the_catch_up_has_the_saved_parts_and_the_open_text_of_the_note() {
        let dir = tempfile::tempdir().unwrap();
        let live = manager(dir.path());
        let store = store();
        store
            .lock()
            .unwrap()
            .create_session("a", chrono::Local::now().fixed_offset(), None)
            .unwrap();
        for (part, word) in [(0u32, "hello"), (1, "there")] {
            let at = u64::from(part) * 1_000;
            let saved = asr::LivePart {
                start_ms: at,
                end_ms: at + 1_000,
                words: vec![asr::LiveWord {
                    start_ms: at,
                    end_ms: at + 300,
                    text: word.into(),
                }],
            };
            pipeline::live::save_part(&store, "a", 0, part, &saved).unwrap();
        }
        *live.now.lock().unwrap() = Some(now("a", "so"));

        let got = live.transcript(&store, "a").unwrap();
        let texts: Vec<_> = got.parts.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, vec!["hello", "there"]);
        assert_eq!(got.current.unwrap().committed, "so");
        assert_eq!(got.state, Some(LiveState::Running));
    }

    #[test]
    fn the_catch_up_of_another_note_has_no_open_text() {
        let dir = tempfile::tempdir().unwrap();
        let live = manager(dir.path());
        *live.now.lock().unwrap() = Some(now("a", "so"));
        let got = live.transcript(&store(), "b").unwrap();
        assert!(got.parts.is_empty());
        assert_eq!(got.current, None);
        assert_eq!(got.state, None);
    }

    #[test]
    fn speaker_catchup_uses_only_the_completed_revision_including_empty() {
        use session_core::recognition::*;
        let store = store();
        store
            .lock()
            .unwrap()
            .create_session("s", chrono::Local::now().fixed_offset(), None)
            .unwrap();
        pipeline::checkpoint::put(
            &store,
            "s",
            session_core::Stage::Transcribe,
            "recognition-generation-v1",
            &"g",
        )
        .unwrap();
        let scope = SpeakerScope {
            generation: "g".into(),
            epoch: 0,
        };
        let range = SampleRange {
            start: 0,
            end: 16000,
        };
        for revision in [1, 2] {
            store
                .lock()
                .unwrap()
                .commit_recognition(
                    "s",
                    &OutputBlock {
                        key: format!("nemo/{:020}/turn{revision}/0", 0),
                        scope: scope.clone(),
                        revision,
                        coverage: Coverage {
                            received: Some(range),
                            computed: Some(range),
                            provisional: Some(range),
                            committed: None,
                        },
                        rows: vec![],
                        speakers: vec![SpeakerTurn {
                            range,
                            timing: TimingPrecision::NativeSegment,
                            speaker: SpeakerKey {
                                scope: scope.clone(),
                                input_start_sample: 0,
                                local_id: revision as u32,
                            },
                            finality: Finality::Provisional,
                        }],
                    },
                )
                .unwrap();
        }
        let marker = |revision| OutputBlock {
            key: format!("nemo/{:020}/turn-commit", 0),
            scope: scope.clone(),
            revision,
            coverage: Coverage::default(),
            rows: vec![],
            speakers: vec![],
        };
        store
            .lock()
            .unwrap()
            .commit_recognition("s", &marker(1))
            .unwrap();
        let got = recognition_catchup(&store, "s").unwrap();
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|event| event.revision == 1));
        assert_eq!(got[1].rows.len(), 1);
        assert!(got[1].rows[0].speaker.as_ref().unwrap().ends_with(":1"));
        store
            .lock()
            .unwrap()
            .commit_recognition("s", &marker(3))
            .unwrap();
        let got = recognition_catchup(&store, "s").unwrap();
        assert_eq!(
            got.iter().map(|event| event.phase).collect::<Vec<_>>(),
            vec![
                LiveRecognitionPhase::SnapshotBegin,
                LiveRecognitionPhase::SnapshotEnd
            ]
        );
        assert!(
            got.iter()
                .all(|event| event.revision == 3 && event.rows.is_empty())
        );
    }

    #[test]
    fn native_catchup_reads_current_durable_generation_and_complete_batches() {
        use session_core::recognition::{
            Coverage, Finality, OutputBlock, RecognitionId, RecognitionSegment, SampleRange,
            SpeakerScope, TimingPrecision,
        };
        let dir = tempfile::tempdir().unwrap();
        let live = manager(dir.path());
        let store = store();
        store
            .lock()
            .unwrap()
            .create_session("native", chrono::Local::now().fixed_offset(), None)
            .unwrap();
        pipeline::checkpoint::put(
            &store,
            "native",
            session_core::Stage::Transcribe,
            "recognition-generation-v1",
            &"current",
        )
        .unwrap();
        for (generation, key, text, revision) in [
            ("old", "old", "Stale", 1),
            ("current", "native/interim/0", "Provisional", 1),
            ("current", "native/final/0", "Final", 2),
        ] {
            let scope = SpeakerScope {
                generation: generation.into(),
                epoch: 0,
            };
            let range = SampleRange {
                start: 0,
                end: 16000,
            };
            let finality = if revision == 2 {
                Finality::Final
            } else {
                Finality::Provisional
            };
            let block = OutputBlock {
                key: key.into(),
                scope: scope.clone(),
                revision,
                coverage: Coverage {
                    received: Some(range),
                    computed: Some(range),
                    provisional: Some(range),
                    committed: None,
                },
                rows: vec![RecognitionSegment {
                    id: RecognitionId {
                        scope,
                        input_start_sample: 0,
                        ordinal: 0,
                    },
                    range,
                    timing: TimingPrecision::NativeSegment,
                    text: text.into(),
                    speaker: None,
                    revision,
                    text_finality: finality,
                    speaker_finality: finality,
                }],
                speakers: vec![],
            };
            store
                .lock()
                .unwrap()
                .commit_recognition("native", &block)
                .unwrap();
        }
        let got = live.transcript(&store, "native").unwrap();
        assert!(got.recognition.iter().all(|e| e.generation == "current"));
        assert_eq!(
            got.recognition.iter().map(|e| e.phase).collect::<Vec<_>>(),
            vec![
                LiveRecognitionPhase::Patch,
                LiveRecognitionPhase::InterimBegin,
                LiveRecognitionPhase::InterimBatch,
                LiveRecognitionPhase::InterimEnd
            ]
        );
        assert_eq!(got.recognition[0].rows[0].text.as_deref(), Some("Final"));
        assert!(got.recognition[0].rows[0].text_final);
        let mut state = now("native", "");
        state.failure = Some(LiveRecognitionEvent {
            session_id: "native".into(),
            phase: LiveRecognitionPhase::Failed,
            key: "failure".into(),
            generation: String::new(),
            epoch: 0,
            revision: 0,
            rows: vec![],
        });
        *live.now.lock().unwrap() = Some(state);
        let reopened = live.transcript(&store, "native").unwrap();
        assert_eq!(
            reopened.recognition.last().unwrap().phase,
            LiveRecognitionPhase::Failed
        );
        assert_eq!(
            reopened.recognition[0].rows[0].text.as_deref(),
            Some("Final")
        );
    }

    #[test]
    fn replay_catchup_emits_only_retained_final_text_for_overlapping_interim() {
        use session_core::recognition::*;
        let store = store();
        store
            .lock()
            .unwrap()
            .create_session("s", chrono::Local::now().fixed_offset(), None)
            .unwrap();
        pipeline::checkpoint::put(
            &store,
            "s",
            session_core::Stage::Transcribe,
            "recognition-generation-v1",
            &"g",
        )
        .unwrap();
        for (epoch, finality, kind, text) in [
            (1, Finality::Final, "word", "retained"),
            (2, Finality::Provisional, "interim", "replay"),
        ] {
            let scope = SpeakerScope {
                generation: "g".into(),
                epoch,
            };
            let range = SampleRange {
                start: 0,
                end: 16000,
            };
            store
                .lock()
                .unwrap()
                .commit_recognition(
                    "s",
                    &OutputBlock {
                        key: format!("nemo/{epoch}/{kind}/0"),
                        scope: scope.clone(),
                        revision: 1,
                        coverage: Coverage {
                            received: Some(range),
                            computed: Some(range),
                            provisional: Some(range),
                            committed: None,
                        },
                        speakers: vec![],
                        rows: vec![RecognitionSegment {
                            id: RecognitionId {
                                scope,
                                input_start_sample: 0,
                                ordinal: 0,
                            },
                            range,
                            timing: TimingPrecision::NativeSegment,
                            text: text.into(),
                            speaker: None,
                            revision: 1,
                            text_finality: finality,
                            speaker_finality: Finality::Provisional,
                        }],
                    },
                )
                .unwrap();
        }
        let events = recognition_catchup(&store, "s").unwrap();
        let visible: Vec<_> = events
            .iter()
            .flat_map(|event| event.rows.iter())
            .filter_map(|row| row.text.as_deref())
            .collect();
        assert_eq!(visible, vec!["retained"]);
        assert!(
            events
                .iter()
                .all(|event| event.phase == LiveRecognitionPhase::Patch)
        );
    }

    #[test]
    fn recognition_patch_identity_survives_durable_copies_and_late_live_events() {
        use session_core::recognition::*;
        let scope = SpeakerScope {
            generation: "g".into(),
            epoch: 7,
        };
        let mut block = OutputBlock {
            key: "nemo/00000000000000000007/word/0/0".into(),
            scope: scope.clone(),
            revision: 4,
            coverage: Coverage::default(),
            speakers: vec![],
            rows: vec![RecognitionSegment {
                id: RecognitionId {
                    scope,
                    input_start_sample: 16000,
                    ordinal: 3,
                },
                range: SampleRange {
                    start: 16000,
                    end: 17000,
                },
                timing: TimingPrecision::NativeSegment,
                text: "word".into(),
                speaker: None,
                revision: 4,
                text_finality: Finality::Final,
                speaker_finality: Finality::Provisional,
            }],
        };
        let live = recognition_event("s", &block);
        for key in ["native-final/7/row/0", "result/9/row/0"] {
            block.key = key.into();
            assert_eq!(recognition_event("s", &block).key, live.key);
        }
        block.key = "nemo/7/interim/0".into();
        let interim = recognition_event("s", &block);
        assert_eq!(interim.phase, LiveRecognitionPhase::InterimBatch);
        assert_ne!(interim.key, live.key);
        block.key = "result/9/row/0".into();
        block.rows[0].id.ordinal += 1;
        assert_ne!(recognition_event("s", &block).key, live.key);
    }

    fn word(start_ms: u64, text: &str) -> asr::LiveWord {
        asr::LiveWord {
            start_ms,
            end_ms: start_ms + 200,
            text: text.into(),
        }
    }

    fn shown(words: &[LiveWord]) -> Vec<&str> {
        words.iter().map(|w| w.text.as_str()).collect()
    }

    fn report(live: &LiveManager, session_id: &str, segment: u32, offset_ms: u64) -> Report {
        Report {
            now: Arc::clone(&live.now),
            session_id: session_id.into(),
            segment,
            offset_ms,
            part: 0,
        }
    }

    fn session_row(store: &pipeline::SharedStore, session_id: &str) {
        store
            .lock()
            .unwrap()
            .create_session(session_id, chrono::Local::now().fixed_offset(), None)
            .unwrap();
    }

    fn close_segment(store: &pipeline::SharedStore, session_id: &str, index: u32, offset_ms: u64) {
        let at = chrono::Local::now().fixed_offset();
        store
            .lock()
            .unwrap()
            .put_segment(
                session_id,
                &session_core::Segment {
                    index,
                    offset_ms,
                    duration_ms: 10_000,
                    wall_start: at,
                    wall_end: at,
                    audio_file: format!("audio/segment-{index:02}.opus"),
                    channels: 1,
                },
            )
            .unwrap();
    }

    fn save(store: &pipeline::SharedStore, session_id: &str, segment: u32, part: u32) -> LiveRow {
        let at = u64::from(part) * 1_000;
        pipeline::live::save_part(
            store,
            session_id,
            segment,
            part,
            &asr::LivePart {
                start_ms: at,
                end_ms: at + 1_000,
                words: vec![word(at, "one"), word(at + 300, "two")],
            },
        )
        .unwrap()
    }

    #[test]
    fn a_saved_part_event_has_the_row_words_and_the_segment_offset() {
        let dir = tempfile::tempdir().unwrap();
        let live = manager(dir.path());
        let store = store();
        session_row(&store, "a");
        let row = save(&store, "a", 1, 3);
        let mut report = report(&live, "a", 1, 90_000);

        let event = report.saved(&row);
        assert_eq!(event.session_id, "a");
        assert_eq!(event.part.segment, 1);
        assert_eq!(event.part.part, 3);
        assert_eq!(event.part.offset_ms, 90_000);
        assert_eq!(event.part.text, "one two");
        assert_eq!(shown(&event.part.words), vec!["one", "two"]);
        assert_eq!(event.part.words[1].start_ms, 3_300);
        // The saved event and the catch-up give the same part.
        live.keep_offset("a", 1, 90_000);
        let got = live.transcript(&store, "a").unwrap();
        assert_eq!(got.parts, vec![event.part]);
    }

    #[test]
    fn the_catch_up_gives_each_part_the_offset_of_its_segment() {
        let dir = tempfile::tempdir().unwrap();
        let live = manager(dir.path());
        let store = store();
        session_row(&store, "a");
        close_segment(&store, "a", 0, 0);
        close_segment(&store, "a", 1, 60_000);
        for (segment, part) in [(0, 0), (0, 1), (1, 0)] {
            save(&store, "a", segment, part);
        }
        // The segment row is canonical for a closed segment.
        live.keep_offset("a", 1, 59_000);

        let got = live.transcript(&store, "a").unwrap();
        let offsets: Vec<_> = got
            .parts
            .iter()
            .map(|p| (p.segment, p.part, p.offset_ms))
            .collect();
        assert_eq!(offsets, vec![(0, 0, 0), (0, 1, 0), (1, 0, 60_000)]);
        assert!(got.parts.iter().all(|p| shown(&p.words) == ["one", "two"]));
    }

    #[test]
    fn the_catch_up_in_the_open_second_segment_uses_the_offset_that_start_received() {
        let dir = tempfile::tempdir().unwrap();
        let live = manager(dir.path());
        let store = store();
        session_row(&store, "a");
        live.keep_offset("a", 0, 0);
        close_segment(&store, "a", 0, 0);
        save(&store, "a", 0, 0);
        // Capture writes no row for the open segment until it closes it.
        live.keep_offset("a", 1, 75_000);
        save(&store, "a", 1, 0);
        save(&store, "a", 1, 1);
        let mut report = report(&live, "a", 1, 75_000);
        report.text(
            2,
            &LiveText {
                committed: "so".into(),
                tentative: String::new(),
            },
            0,
            &[word(2_000, "so")],
        );

        let got = live.transcript(&store, "a").unwrap();
        let offsets: Vec<_> = got.parts.iter().map(|p| p.offset_ms).collect();
        assert_eq!(offsets, vec![0, 75_000, 75_000]);
        assert_eq!(got.current.unwrap().offset_ms, 75_000);
    }

    #[test]
    fn the_catch_up_current_has_all_open_part_words() {
        let dir = tempfile::tempdir().unwrap();
        let live = manager(dir.path());
        let store = store();
        session_row(&store, "a");
        let mut report = report(&live, "a", 1, 40_000);
        let text = |committed: &str| LiveText {
            committed: committed.into(),
            tentative: "x".into(),
        };
        let first = report.text(
            0,
            &text("a b c"),
            0,
            &[word(0, "a"), word(300, "b"), word(600, "c")],
        );
        assert_eq!(first.word_start, 0);
        assert_eq!(first.offset_ms, 40_000);
        // A revision of word 2 and one new word.
        let second = report.text(
            0,
            &text("a b see d"),
            2,
            &[word(600, "see"), word(900, "d")],
        );
        assert_eq!(second.word_start, 2);
        assert_eq!(shown(&second.words), vec!["see", "d"]);
        report.text(0, &text("a b see d e"), 4, &[word(1_200, "e")]);

        let current = live.transcript(&store, "a").unwrap().current.unwrap();
        assert_eq!((current.segment, current.part), (1, 0));
        assert_eq!(current.offset_ms, 40_000);
        assert_eq!(shown(&current.words), vec!["a", "b", "see", "d", "e"]);

        // A saved part starts the next part with no words.
        save(&store, "a", 1, 0);
        report.saved(&pipeline::live::segment_rows(&store, "a", 1).unwrap()[0]);
        report.text(1, &text("f"), 0, &[word(1_500, "f")]);
        let current = live.transcript(&store, "a").unwrap().current.unwrap();
        assert_eq!(current.part, 1);
        assert_eq!(shown(&current.words), vec!["f"]);
    }

    #[test]
    fn speaker_catchup_in_the_second_epoch_has_its_latest_complete_revision() {
        use session_core::recognition::*;
        let store = store();
        session_row(&store, "s");
        pipeline::checkpoint::put(
            &store,
            "s",
            session_core::Stage::Transcribe,
            "recognition-generation-v1",
            &"g",
        )
        .unwrap();
        let range = SampleRange {
            start: 0,
            end: 16000,
        };
        let commit = |block: OutputBlock| {
            store
                .lock()
                .unwrap()
                .commit_recognition("s", &block)
                .unwrap();
        };
        for (epoch, revisions, complete) in [(0u64, 1..=1u64, 1u64), (1, 1..=3, 2)] {
            let scope = SpeakerScope {
                generation: "g".into(),
                epoch,
            };
            for revision in revisions {
                commit(OutputBlock {
                    key: format!("nemo/{epoch:020}/turn{revision}/0"),
                    scope: scope.clone(),
                    revision,
                    coverage: Coverage {
                        received: Some(range),
                        computed: Some(range),
                        provisional: Some(range),
                        committed: None,
                    },
                    rows: vec![],
                    speakers: vec![SpeakerTurn {
                        range,
                        timing: TimingPrecision::NativeSegment,
                        speaker: SpeakerKey {
                            scope: scope.clone(),
                            input_start_sample: 0,
                            local_id: revision as u32,
                        },
                        finality: Finality::Provisional,
                    }],
                });
            }
            commit(OutputBlock {
                key: format!("nemo/{epoch:020}/turn-commit"),
                scope,
                revision: complete,
                coverage: Coverage::default(),
                rows: vec![],
                speakers: vec![],
            });
        }
        let got = recognition_catchup(&store, "s").unwrap();
        let second: Vec<_> = got.iter().filter(|event| event.epoch == 1).collect();
        assert_eq!(
            second.iter().map(|event| event.phase).collect::<Vec<_>>(),
            vec![
                LiveRecognitionPhase::SnapshotBegin,
                LiveRecognitionPhase::SnapshotBatch,
                LiveRecognitionPhase::SnapshotEnd
            ]
        );
        assert!(second.iter().all(|event| event.revision == 2));
        assert!(
            second[1].rows[0]
                .speaker
                .as_ref()
                .unwrap()
                .ends_with(":1:0:2")
        );
    }

    #[test]
    fn a_manager_without_workers_is_not_busy() {
        let dir = tempfile::tempdir().unwrap();
        let live = manager(dir.path());
        assert!(!live.busy());
        live.shutdown();
    }
}
