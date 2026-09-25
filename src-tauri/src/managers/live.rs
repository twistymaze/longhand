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
use pipeline::live::{LiveConfig, LiveRow, LiveSink, LiveStatus, LiveTap, LiveWorker, Workers};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use tauri_specta::Event as _;

use crate::managers::models::ModelsManager;

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
    pub fn start(
        &self,
        app: &AppHandle,
        session_id: &str,
        segment: u32,
        model: &str,
        sample_rate: u32,
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
        let config = LiveConfig::new(session_id, segment, model, sample_rate);
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
        Ok(LiveTranscript {
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
        };
        let entry = now.get_or_insert_with(fresh);
        if entry.session_id != self.session_id || entry.current.segment != self.segment {
            *entry = fresh();
        }
        change(entry);
    }
}

impl LiveSink for AppSink {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn manager(dir: &std::path::Path) -> LiveManager {
        LiveManager::new(Arc::new(ModelsManager::at(dir.to_path_buf())))
    }

    fn store() -> pipeline::SharedStore {
        let store = session_core::Store::open_in_memory().unwrap();
        Arc::new(Mutex::new(store))
    }

    fn now(session_id: &str, committed: &str) -> Now {
        Now {
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
    fn a_manager_without_workers_is_not_busy() {
        let dir = tempfile::tempdir().unwrap();
        let live = manager(dir.path());
        assert!(!live.busy());
        live.shutdown();
    }
}
