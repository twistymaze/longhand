//! Wires the live worker (U6, KTD3, KTD9, KTD10) into the app: one worker
//! for each capture segment of a note in live mode, its events out to the
//! surface, and the catch-up text for a Transcript tab that opens during
//! recording.
//!
//! The worker itself is `pipeline::live`. It shares the processing
//! runner's engine slot, so this manager gets the workers from the
//! processing manager once the runner exists.

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
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct LiveTranscriptEvent {
    pub session_id: String,
    pub segment: u32,
    pub part: u32,
    pub committed: String,
    pub tentative: String,
}

/// A part of the live transcript was saved. The open part of the next
/// event starts after it.
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct LivePartSaved {
    pub session_id: String,
    pub segment: u32,
    pub part: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
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

/// One saved part of the live transcript.
#[derive(Serialize, Clone, Type, Debug, PartialEq, Eq)]
pub struct LivePartText {
    pub segment: u32,
    pub part: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

impl From<&LiveRow> for LivePartText {
    fn from(row: &LiveRow) -> Self {
        Self {
            segment: row.segment,
            part: row.part,
            start_ms: row.start_ms,
            end_ms: row.end_ms,
            text: row.text(),
        }
    }
}

/// The text of the part that is open now.
#[derive(Serialize, Clone, Type, Debug, PartialEq, Eq)]
pub struct LiveCurrent {
    pub segment: u32,
    pub part: u32,
    pub committed: String,
    pub tentative: String,
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

pub struct LiveManager {
    models: Arc<ModelsManager>,
    workers: OnceLock<Workers>,
    now: Arc<Mutex<Option<Now>>>,
}

impl LiveManager {
    pub fn new(models: Arc<ModelsManager>) -> Self {
        Self {
            models,
            workers: OnceLock::new(),
            now: Arc::new(Mutex::new(None)),
        }
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
        let sink = || AppSink {
            app: app.clone(),
            models: Arc::clone(&self.models),
            now: Arc::clone(&self.now),
            session_id: session_id.to_string(),
            segment,
            part: 0,
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
        let parts = pipeline::live::rows(store, session_id)
            .map_err(|e| e.to_string())?
            .iter()
            .map(LivePartText::from)
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
    now: Arc<Mutex<Option<Now>>>,
    session_id: String,
    segment: u32,
    /// The number of the open part.
    part: u32,
}

impl AppSink {
    fn update(&self, change: impl FnOnce(&mut Now)) {
        let mut now = self.now.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = || Now {
            session_id: self.session_id.clone(),
            current: LiveCurrent {
                segment: self.segment,
                part: self.part,
                committed: String::new(),
                tentative: String::new(),
            },
            state: LiveState::Starting,
            failure: None,
        };
        let entry = now.get_or_insert_with(fresh);
        if entry.session_id != self.session_id || entry.current.segment != self.segment {
            *entry = fresh();
        }
        change(entry);
    }
}

impl LiveSink for AppSink {
    fn interim_snapshot(&mut self, scope: &asr::SpeakerScope, revision: u64, begin: bool) {
        let _ = LiveRecognitionEvent {
            session_id: self.session_id.clone(),
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
            session_id: self.session_id.clone(),
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
        self.update(|now| now.failure = Some(failure.clone()));
        let _ = failure.emit(&self.app);
    }

    fn snapshot(&mut self, scope: &asr::SpeakerScope, revision: u64, begin: bool) {
        let _ = LiveRecognitionEvent {
            session_id: self.session_id.clone(),
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
        let _ = recognition_event(&self.session_id, block).emit(&self.app);
    }

    fn model(&mut self, held: Option<&str>) {
        // Delete refuses the model of the live worker (KTD12).
        self.models.set_live_model(held.map(str::to_string));
    }

    fn status(&mut self, status: LiveStatus) {
        let state = LiveState::from(status);
        self.update(|now| now.state = state);
        let _ = LiveTranscriptStatus {
            session_id: self.session_id.clone(),
            segment: self.segment,
            state,
        }
        .emit(&self.app);
    }

    fn text(&mut self, part: u32, text: &LiveText) {
        self.part = part;
        self.update(|now| {
            now.current = LiveCurrent {
                segment: self.segment,
                part,
                committed: text.committed.clone(),
                tentative: text.tentative.clone(),
            };
        });
        let _ = LiveTranscriptEvent {
            session_id: self.session_id.clone(),
            segment: self.segment,
            part,
            committed: text.committed.clone(),
            tentative: text.tentative.clone(),
        }
        .emit(&self.app);
    }

    fn saved(&mut self, row: &LiveRow) {
        self.part = row.part + 1;
        let next = self.part;
        self.update(|now| {
            now.current = LiveCurrent {
                segment: row.segment,
                part: next,
                committed: String::new(),
                tentative: String::new(),
            };
        });
        let text = LivePartText::from(row);
        let _ = LivePartSaved {
            session_id: self.session_id.clone(),
            segment: text.segment,
            part: text.part,
            start_ms: text.start_ms,
            end_ms: text.end_ms,
            text: text.text,
        }
        .emit(&self.app);
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
                committed: committed.into(),
                tentative: "and".into(),
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

    #[test]
    fn a_manager_without_workers_is_not_busy() {
        let dir = tempfile::tempdir().unwrap();
        let live = manager(dir.path());
        assert!(!live.busy());
        live.shutdown();
    }
}
