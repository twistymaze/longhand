//! Owns capture: the microphone, the segment being recorded, and the
//! hand-off of segment open and close to the session core (KTD3, KTD4, KTD9).
//!
//! Each segment records on a thread of its own, which also holds the
//! idle-sleep assertion (U22): it must be created and dropped by the thread
//! that owns active capture. A crash leaves the open segment's
//! chunks on disk under their names; those names, not the job store, are
//! what launch recovery rebuilds the segment from (R8).
//!
//! A segment that ends on its own is restarted within the source budget
//! (U14): the session stays Capturing across the attempts, and only an
//! exhausted budget interrupts it. While it is Interrupted(device) the
//! system's inputs are polled for the microphone's return; anarlog listens
//! for platform device events instead, through a crate this does not take.

use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use audio::AudioProvider;
use audio_cpal::{CpalAudio, Inputs};
use chrono::{DateTime, FixedOffset, Local};
use recorder::storage::DISK_RESERVE_BYTES;
use recorder::{ChunkedWriter, SegmentArgs, SegmentEnd, SegmentOutcome};
use serde::Serialize;
use session_core::{
    Anchor, Closed, DEFAULT_LOOKBACK_MS, Event, InterruptReason, SessionState, SpeakerCount, Store,
    Timeline,
};
use specta::Type;
use supervisor::{RestartTracker, SOURCE_RESTART_BUDGET};
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;
use tokio_util::sync::CancellationToken;

use crate::managers::alerts::{self, AlertsManager};

/// Frames of 20 ms: small enough for a responsive level meter (U7), large
/// enough to keep per-frame overhead negligible.
const FRAMES_PER_SECOND: u32 = 50;

/// How often the inputs are compared while waiting for a lost microphone.
const DEVICE_POLL: Duration = Duration::from_secs(2);

pub use pipeline::RECOVERY_DIR;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("a session is already recording")]
    AlreadyCapturing,
    #[error("nothing is recording")]
    NotCapturing,
    #[error("microphone: {0}")]
    Audio(#[from] audio::Error),
    #[error(transparent)]
    Recorder(#[from] recorder::RecorderError),
    #[error(transparent)]
    Store(#[from] session_core::StoreError),
    #[error(transparent)]
    Timeline(#[from] session_core::TimelineError),
    #[error("this session is already moving to its destination and can no longer be resumed")]
    Landing,
    #[error("{0}")]
    Other(String),
}

/// A quarter-second of capture: level and position (R9). Ticks follow frame
/// arrival, so their absence is what a stalled input looks like.
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct CaptureTick {
    pub session_id: String,
    pub peak: f32,
    /// On the session timeline, gaps between segments included: where a
    /// note typed now is anchored.
    pub position_ms: u64,
    /// Audio recorded in the session so far, gaps excluded: the elapsed
    /// time shown while recording, which a resume continues rather than
    /// jumping by the time spent away.
    pub recorded_ms: u64,
}

/// A session started, stopped, or resumed recording.
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct RecordingChanged {
    /// The session now recording; `None` once nothing is.
    pub session_id: Option<String>,
}

#[derive(Serialize, Clone, Type, Debug)]
pub struct NoteView {
    pub id: i64,
    pub text: String,
    /// Position on the session timeline; `None` when un-anchored (R13).
    pub position_ms: Option<u64>,
    pub gap_anchored: bool,
}

impl From<session_core::Note> for NoteView {
    fn from(note: session_core::Note) -> Self {
        let (position_ms, gap_anchored) = match note.anchor {
            Anchor::Unanchored => (None, false),
            Anchor::At {
                position_ms,
                gap_anchored,
            } => (Some(position_ms), gap_anchored),
        };
        Self {
            id: note.id,
            text: note.text,
            position_ms,
            gap_anchored,
        }
    }
}

#[derive(Serialize, Clone, Type, Debug)]
pub struct RecordingStatus {
    /// The open session: recording, or interrupted and resumable.
    pub session_id: Option<String>,
    pub state: Option<String>,
    pub recording: bool,
    /// The microphone went away and is being reopened (U14); the session
    /// still counts as recording.
    pub reconnecting: bool,
    /// Why the open session stopped recording, while it is interrupted: the
    /// failed-capture state, with Resume and Stop (R5c).
    pub interruption: Option<Interruption>,
    /// Free bytes where audio is being written.
    pub working_free_bytes: Option<u64>,
}

#[derive(Serialize, Clone, Type, Debug)]
pub struct Interruption {
    pub headline: String,
    /// A disk interruption resumes only once space is freed (R7a).
    pub disk: bool,
}

/// Capture stopped on its own: the device went away or the disk filled.
/// Interruption handling (U14, U15) consumes it.
#[derive(Serialize, Clone, Type, tauri_specta::Event)]
pub struct CaptureEnded {
    pub session_id: String,
    pub reason: String,
}

struct ActiveSegment {
    session_id: String,
    segment: u32,
    stop: CancellationToken,
    outcome: std_mpsc::Receiver<SegmentOutcome>,
    thread: JoinHandle<()>,
}

/// The session whose timeline this process has been measuring.
struct Clock {
    session_id: String,
    timeline: Timeline,
    /// The monotonic origin of the timeline, while this process holds it.
    epoch: Instant,
    /// Restarts after a loss, across the session: a microphone that keeps
    /// dropping runs the budget out rather than resetting it at each drop.
    restarts: RestartTracker,
}

impl Clock {
    fn new(session_id: &str, timeline: Timeline, epoch: Instant) -> Self {
        Self {
            session_id: session_id.to_string(),
            timeline,
            epoch,
            restarts: RestartTracker::new(),
        }
    }
}

/// Background work on a session's behalf that a user action supersedes.
struct Pending {
    session_id: String,
    cancel: CancellationToken,
}

impl Pending {
    fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            cancel: CancellationToken::new(),
        }
    }
}

/// Why a segment ended on its own.
struct Loss {
    reason: InterruptReason,
    message: String,
    /// Whether a restart can help: a full disk stays full (R7a).
    restartable: bool,
}

impl Loss {
    fn of(end: &SegmentEnd) -> Option<Self> {
        match end {
            SegmentEnd::Stopped => None,
            SegmentEnd::SourceLost(e) => Some(Self {
                reason: InterruptReason::Device,
                message: format!("microphone: {e}"),
                restartable: true,
            }),
            SegmentEnd::Storage(e @ recorder::RecorderError::LowDisk { .. }) => Some(Self {
                reason: InterruptReason::Disk,
                message: e.to_string(),
                restartable: false,
            }),
            SegmentEnd::Storage(e) => Some(Self {
                reason: InterruptReason::Disk,
                message: e.to_string(),
                restartable: true,
            }),
        }
    }
}

#[derive(Default)]
struct Inner {
    active: Option<ActiveSegment>,
    clock: Option<Clock>,
    sample_rate: Option<u32>,
    /// A lost segment being restarted.
    recovering: Option<Pending>,
    /// An Interrupted(device) session waiting for its microphone.
    watching: Option<Pending>,
}

impl Inner {
    /// Stop any restart or device watch: a user action decides now.
    fn supersede(&mut self) -> Option<String> {
        if let Some(watch) = self.watching.take() {
            watch.cancel.cancel();
        }
        self.recovering.take().map(|p| {
            p.cancel.cancel();
            p.session_id
        })
    }
}

pub struct CaptureManager {
    audio: CpalAudio,
    working_root: PathBuf,
    /// Shared with the processing runner, so every lifecycle transition goes
    /// through one connection.
    store: pipeline::SharedStore,
    inner: Mutex<Inner>,
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

fn segment_audio_file(index: u32) -> String {
    format!("audio/segment-{:02}.opus", index + 1)
}

/// Where a resumed segment starts on the session timeline. While this
/// process still holds the session's monotonic origin, it is the elapsed
/// time since then; across a restart the origin is gone, and the wall-clock
/// gap since the last segment ended is the best measure left (KTD9).
pub fn resume_offset(
    last_end_ms: u64,
    last_wall_end: DateTime<FixedOffset>,
    now_wall: DateTime<FixedOffset>,
    since_epoch_ms: Option<u64>,
) -> u64 {
    match since_epoch_ms {
        Some(elapsed) => elapsed.max(last_end_ms),
        None => {
            let gap = (now_wall - last_wall_end).num_milliseconds().max(0) as u64;
            last_end_ms + gap
        }
    }
}

impl CaptureManager {
    pub fn new(app: &AppHandle) -> Result<Self, CaptureError> {
        let data = app
            .path()
            .app_data_dir()
            .map_err(|e| CaptureError::Other(e.to_string()))?;
        std::fs::create_dir_all(&data).map_err(|e| CaptureError::Other(e.to_string()))?;
        let store = Store::open(&data.join("longhand.db"))?;
        Ok(Self {
            audio: CpalAudio::new(),
            working_root: data.join("sessions"),
            store: Arc::new(Mutex::new(store)),
            inner: Mutex::new(Inner::default()),
        })
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn shared_store(&self) -> pipeline::SharedStore {
        Arc::clone(&self.store)
    }

    pub fn working_root(&self) -> &Path {
        &self.working_root
    }

    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.working_root.join(session_id)
    }

    /// Resolve the microphone and its format ahead of the press, so pressing
    /// the shortcut only opens the stream (R1b).
    pub fn prepare(&self) -> Result<u32, CaptureError> {
        let rate = self.audio.prepare(None)?;
        self.inner().sample_rate = Some(rate);
        log::info!("microphone prepared at {rate} Hz");
        Ok(rate)
    }

    /// Recording, or reopening a lost microphone to go on recording.
    pub fn is_capturing(&self) -> bool {
        let inner = self.inner();
        inner.active.is_some() || inner.recovering.is_some()
    }

    /// Start a new session recording (R1, R6c). A session already recording
    /// is closed first and then stopped and queued by the store.
    pub fn start_session(
        &self,
        app: &AppHandle,
        speaker_count: Option<SpeakerCount>,
    ) -> Result<String, CaptureError> {
        let mut inner = self.inner();
        inner.supersede();
        if let Some(active) = inner.active.take() {
            self.close_segment(&mut inner, active)?;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let started = now();
        let mut timeline = Timeline::new();
        let index = timeline.open(0, started)?;
        let active = self.open_segment(app, &mut inner, &id, index, 0)?;
        let stopped = match self.store().start_session(&id, started, speaker_count) {
            Ok(stopped) => stopped,
            Err(e) => {
                abandon(active);
                let _ = std::fs::remove_dir_all(self.session_dir(&id));
                return Err(e.into());
            }
        };
        for other in stopped {
            self.record_model_at_stop(app, &other);
            log::info!("session {other} stopped by a new session");
        }
        inner.active = Some(active);
        inner.clock = Some(Clock::new(&id, timeline, Instant::now()));
        log::info!("session {id} recording");
        changed(app, Some(&id));
        Ok(id)
    }

    /// Stop the recording session (R6b, R35). Returns its state afterwards:
    /// queued for processing, or discarded when it kept no audio.
    pub fn stop_session(&self, app: &AppHandle) -> Result<(String, SessionState), CaptureError> {
        let mut inner = self.inner();
        let reconnecting = inner.supersede();
        let id = match inner.active.take() {
            Some(active) => {
                let id = active.session_id.clone();
                self.close_segment(&mut inner, active)?;
                id
            }
            None => match reconnecting {
                // Stopped between restart attempts: the lost segment is closed.
                Some(id) => id,
                // Stopped from the failed-capture state, which acknowledges
                // it (R5c).
                None => match self.open_session()? {
                    Some((id, SessionState::Interrupted { .. })) => id,
                    _ => return Err(CaptureError::NotCapturing),
                },
            },
        };
        let retained_audio = !self.store().segments(&id)?.is_empty();
        let state = self.store().apply(
            &id,
            &Event::UserStop {
                at: now(),
                retained_audio,
            },
        )?;
        if retained_audio {
            self.record_model_at_stop(app, &id);
        } else {
            // Nothing survived R35; the notice is the surface's (U7).
            let _ = std::fs::remove_dir_all(self.session_dir(&id));
            log::info!("session {id} kept no audio and was discarded");
        }
        drop(inner);
        app.state::<Arc<AlertsManager>>().clear(&id);
        changed(app, None);
        Ok((id, state))
    }

    /// A queued session keeps the model selected when it stopped (R20c).
    fn record_model_at_stop(&self, app: &AppHandle, id: &str) {
        let model = crate::managers::settings::get_settings(app).asr_model;
        if let Err(e) = self.store().set_asr_model(id, &model) {
            log::warn!("could not record the model for {id}: {e}");
        }
    }

    /// Resume an interrupted session as its next segment (R6). The store
    /// decides whether the resume is allowed (window, most recent, space).
    pub fn resume_session(&self, app: &AppHandle, id: &str) -> Result<SessionState, CaptureError> {
        let mut inner = self.inner();
        if inner.active.is_some() || inner.recovering.is_some() {
            return Err(CaptureError::AlreadyCapturing);
        }
        let space_ok = recorder::storage::check(
            recorder::storage::available_space(&self.working_root),
            DISK_RESERVE_BYTES,
        )
        .is_ok();
        let resumed_at = now();
        // Under one lock with landing's claim: either landing has begun and
        // the resume is refused, or the resume wins and landing stops. A
        // completed session has finished landing and resumes into its folder
        // while that holds its audio (R6d).
        let state = {
            let mut store = self.store();
            let other = |e: pipeline::StageError| CaptureError::Other(e.to_string());
            let completed = store
                .session(id)?
                .is_some_and(|r| r.state == SessionState::Landed);
            if !completed && pipeline::land::begun(&store, id).map_err(other)? {
                return Err(CaptureError::Landing);
            }
            let audio_intact = pipeline::land::home_intact(&store, id).map_err(other)?;
            store.apply(
                id,
                &Event::UserResume {
                    space_ok,
                    audio_intact,
                },
            )?
        };
        if !matches!(state, SessionState::Capturing) {
            return Ok(state);
        }
        inner.supersede();
        let held = inner
            .clock
            .take()
            .filter(|c| c.session_id == id)
            .map(|c| (c.timeline, Some(c.epoch)));
        let (mut timeline, epoch) = match held {
            Some(held) => held,
            None => (Timeline::from_segments(self.store().segments(id)?)?, None),
        };
        let offset = match timeline.segments().last() {
            Some(last) => resume_offset(
                last.end_ms(),
                last.wall_end,
                resumed_at,
                epoch.map(|e| e.elapsed().as_millis() as u64),
            ),
            None => 0,
        };
        let index = timeline.open(offset, resumed_at)?;
        let active = self.open_segment(app, &mut inner, id, index, offset)?;
        inner.active = Some(active);
        let epoch = epoch.unwrap_or_else(|| {
            let now = Instant::now();
            now.checked_sub(Duration::from_millis(offset))
                .unwrap_or(now)
        });
        inner.clock = Some(Clock::new(id, timeline, epoch));
        log::info!("session {id} resumed as segment {index} at {offset} ms");
        app.state::<Arc<AlertsManager>>().clear(id);
        changed(app, Some(id));
        Ok(state)
    }

    /// The session a note typed now belongs to: the one recording, or the
    /// most recent one interrupted and still resumable.
    fn open_session(&self) -> Result<Option<(String, SessionState)>, CaptureError> {
        let store = self.store();
        for name in ["capturing", "interrupted"] {
            if let Some(record) = store.sessions_in(name)?.into_iter().next() {
                return Ok(Some((record.id, record.state)));
            }
        }
        Ok(None)
    }

    pub fn session_state(&self, id: &str) -> Option<SessionState> {
        self.store().session(id).ok().flatten().map(|r| r.state)
    }

    /// The session left in failed-capture, if any: interrupted by a device,
    /// the disk or a crash, and not yet acknowledged (R5c).
    pub fn failed_session(&self) -> Option<(String, InterruptReason)> {
        let sessions = self.store().sessions_in("interrupted").ok()?;
        sessions.into_iter().find_map(|r| match r.state {
            SessionState::Interrupted {
                reason,
                acknowledged: false,
                ..
            } if reason != InterruptReason::Sleep => Some((r.id, reason)),
            _ => None,
        })
    }

    pub fn status(&self) -> Result<RecordingStatus, CaptureError> {
        let (recording, reconnecting) = {
            let inner = self.inner();
            let reconnecting = inner.recovering.is_some();
            (inner.active.is_some() || reconnecting, reconnecting)
        };
        let open = self.open_session()?;
        let interruption = match open.as_ref().map(|(_, state)| state) {
            Some(SessionState::Interrupted { reason, .. }) => Some(Interruption {
                headline: alerts::headline(*reason).to_string(),
                disk: *reason == InterruptReason::Disk,
            }),
            _ => None,
        };
        Ok(RecordingStatus {
            session_id: open.as_ref().map(|(id, _)| id.clone()),
            state: open.as_ref().map(|(_, state)| state.name().to_string()),
            recording,
            reconnecting,
            interruption,
            working_free_bytes: recorder::storage::available_space(
                self.working_root.parent().unwrap_or(&self.working_root),
            ),
        })
    }

    /// Record a note at its first keystroke (R12, R12a, R13). The anchor is
    /// resolved here, on the capture clock, and never moves afterwards.
    pub fn begin_note(&self, text: &str) -> Result<NoteView, CaptureError> {
        let inner = self.inner();
        let Some((session_id, _)) = self.open_session()? else {
            let id = self.store().add_pending_note(text)?;
            return Ok(NoteView {
                id,
                text: text.to_string(),
                position_ms: None,
                gap_anchored: false,
            });
        };
        let anchor = match inner.clock.as_ref().filter(|c| c.session_id == session_id) {
            Some(clock) => session_core::resolve(
                &clock.timeline,
                Some(clock.epoch.elapsed().as_millis() as u64),
                DEFAULT_LOOKBACK_MS,
            ),
            // Interrupted before this launch: the process clock is gone, and
            // any moment now is in the gap after the last segment (R12a).
            None => {
                let timeline = Timeline::from_segments(self.store().segments(&session_id)?)?;
                session_core::resolve(&timeline, Some(u64::MAX), DEFAULT_LOOKBACK_MS)
            }
        };
        let id = self.store().add_note(&session_id, text, anchor)?;
        Ok(NoteView::from(session_core::Note {
            id,
            session_id: Some(session_id),
            text: text.to_string(),
            anchor,
        }))
    }

    /// Change a note's text. On a landed session the folder's `notes.md` is
    /// rewritten whole in the same breath (R22f), under the lock landing
    /// takes to write it, so no edit is lost or written twice.
    pub fn edit_note(&self, note_id: i64, text: &str) -> Result<(), CaptureError> {
        let mut store = self.store();
        store.edit_note(note_id, text)?;
        if let Some(session_id) = store.note(note_id)?.and_then(|n| n.session_id) {
            pipeline::land::rewrite_notes(&mut store, &session_id)
                .map_err(|e| CaptureError::Other(e.to_string()))?;
        }
        Ok(())
    }

    /// The open session's notes, or the pending buffer when none is open.
    pub fn notes(&self) -> Result<Vec<NoteView>, CaptureError> {
        let notes = match self.open_session()? {
            Some((id, _)) => self.store().notes(&id)?,
            None => self.store().pending_notes()?,
        };
        Ok(notes.into_iter().map(NoteView::from).collect())
    }

    /// Bound the open session's diarization (R36).
    pub fn set_speaker_count(&self, count: Option<SpeakerCount>) -> Result<(), CaptureError> {
        let (id, _) = self.open_session()?.ok_or(CaptureError::NotCapturing)?;
        Ok(self.store().set_speaker_count(&id, count)?)
    }

    /// A segment ended on its own (U14). Close it; then, when a restart can
    /// help, reopen the microphone within the source budget, sleeping 1, 2
    /// and 4 s between attempts. The session stays Capturing throughout and
    /// is interrupted only once the budget runs out.
    fn segment_lost(self: Arc<Self>, app: &AppHandle, id: &str, segment: u32, loss: Loss) {
        let (cancel, restarts) = {
            let mut inner = self.inner();
            let ours = inner
                .active
                .as_ref()
                .is_some_and(|a| a.session_id == id && a.segment == segment);
            // Already stopped, or replaced by a new session.
            if !ours {
                return;
            }
            if let Some(active) = inner.active.take()
                && let Err(e) = self.close_segment(&mut inner, active)
            {
                log::error!("closing the lost segment of {id} failed: {e}");
            }
            if !loss.restartable {
                drop(inner);
                self.interrupt(app, id, &loss, None);
                return;
            }
            begin_restart(&mut inner, id)
        };
        self.restart(app, id, cancel, restarts, loss);
    }

    /// The system is about to sleep (U13): close the segment, so it is whole
    /// on disk before the machine stops, and pause the session without an
    /// alert. Runs on the thread delivering the notice, which sleep waits on.
    pub fn will_sleep(&self, app: &AppHandle) {
        let mut inner = self.inner();
        // A restart in progress is moot; the wake starts a fresh one. A
        // device watch carries on through the sleep.
        let reconnecting = inner.recovering.take().map(|p| {
            p.cancel.cancel();
            p.session_id
        });
        let id = match inner.active.take() {
            Some(active) => {
                let id = active.session_id.clone();
                if let Err(e) = self.close_segment(&mut inner, active) {
                    log::error!("closing {id} for sleep failed: {e}");
                }
                id
            }
            None => match reconnecting {
                Some(id) => id,
                None => return,
            },
        };
        let paused = Event::CaptureLost {
            reason: InterruptReason::Sleep,
            at: now(),
        };
        if let Err(e) = self.store().apply(&id, &paused) {
            log::error!("pausing {id} for sleep failed: {e}");
            return;
        }
        drop(inner);
        log::info!("session {id} paused for sleep");
        changed(app, None);
    }

    /// The system woke (U13). A session paused for sleep resumes as its next
    /// segment under the same budget as a lost device (U14), so a microphone
    /// that did not come back ends in Interrupted(device) and its alert.
    pub fn did_wake(self: Arc<Self>, app: &AppHandle) {
        let (id, cancel, restarts) = {
            let mut inner = self.inner();
            let Some(clock) = inner.clock.as_mut() else {
                return;
            };
            // The monotonic clock stands still while a Mac sleeps; the wall
            // clock is the only measure of the time away.
            if let Some(last) = clock.timeline.segments().last() {
                let away = (now() - last.wall_end).num_milliseconds().max(0) as u64;
                let elapsed = clock.epoch.elapsed().as_millis() as u64;
                let position = position_after_wake(elapsed, last.end_ms(), away);
                if position > elapsed
                    && let Some(epoch) = Instant::now().checked_sub(Duration::from_millis(position))
                {
                    clock.epoch = epoch;
                }
            }
            let id = clock.session_id.clone();
            if inner.active.is_some() || inner.recovering.is_some() {
                return;
            }
            // A sleep past the auto-queue wait (R5e) is taken as the end of
            // the session, whichever of the wake and the queue's tick is
            // first; it can still be resumed by hand.
            let paused = self.store().session(&id).ok().flatten().is_some_and(|r| {
                matches!(
                    r.state,
                    SessionState::Interrupted {
                        reason: InterruptReason::Sleep,
                        ..
                    }
                ) && !r.state.auto_queue_due(now())
            });
            if !paused {
                return;
            }
            if let Err(e) = self.store().apply(&id, &Event::DidWake) {
                log::error!("waking {id} failed: {e}");
                return;
            }
            if let Some(clock) = inner.clock.as_mut() {
                clock.restarts = RestartTracker::new();
            }
            let (cancel, restarts) = begin_restart(&mut inner, &id);
            (id, cancel, restarts)
        };
        log::info!("session {id} resuming after sleep");
        let loss = Loss {
            reason: InterruptReason::Device,
            message: "the microphone did not come back after sleep".into(),
            restartable: true,
        };
        self.restart(app, &id, cancel, restarts, loss);
    }

    /// Reopen the microphone within the source budget, sleeping 1, 2 and 4 s
    /// before the attempts; interrupt the session once the budget runs out.
    fn restart(
        self: Arc<Self>,
        app: &AppHandle,
        id: &str,
        cancel: CancellationToken,
        mut restarts: RestartTracker,
        loss: Loss,
    ) {
        changed(app, Some(id));

        let backoff = Backoff(&cancel);
        let recovery = supervisor::recover(&mut restarts, &SOURCE_RESTART_BUDGET, &backoff, || {
            let mut inner = self.inner();
            if cancel.is_cancelled() {
                return Err(CaptureError::Other("superseded".into()));
            }
            self.reopen(app, &mut inner, id)
        });

        let mut inner = self.inner();
        // A stop or a new session took over; it has settled the session.
        if cancel.is_cancelled() {
            return;
        }
        inner.recovering = None;
        if let Some(clock) = inner.clock.as_mut().filter(|c| c.session_id == id) {
            clock.restarts = restarts;
        }
        match recovery {
            supervisor::Recovery::Resumed(()) => {
                if let Err(e) = self.store().apply(id, &Event::AutoResumed) {
                    log::error!("recording the restart of {id} failed: {e}");
                }
                drop(inner);
                changed(app, Some(id));
            }
            supervisor::Recovery::Exhausted(last) => {
                drop(inner);
                self.interrupt(app, id, &loss, last);
            }
            supervisor::Recovery::Cancelled => {}
        }
    }

    /// Capture cannot go on by itself: interrupt the session, say why, and
    /// for a lost microphone watch for its return.
    fn interrupt(self: Arc<Self>, app: &AppHandle, id: &str, loss: &Loss, last: Option<String>) {
        let lost = Event::CaptureLost {
            reason: loss.reason,
            at: now(),
        };
        if let Err(e) = self.store().apply(id, &lost) {
            log::error!("interrupting {id} failed: {e}");
            return;
        }
        let reason = match last {
            Some(last) => format!("{}; reopening it failed: {last}", loss.message),
            None => loss.message.clone(),
        };
        log::warn!("session {id} interrupted: {reason}");
        // In this order: the surface clears the reason on a change.
        changed(app, None);
        let _ = CaptureEnded {
            session_id: id.to_string(),
            reason: reason.clone(),
        }
        .emit(app);
        if loss.reason != InterruptReason::Sleep {
            app.state::<Arc<AlertsManager>>().raise(
                app,
                &self,
                id,
                loss.reason,
                Some(&reason),
                true,
            );
        }
        if loss.reason == InterruptReason::Device {
            let manager = Arc::clone(&self);
            let app = app.clone();
            let id = id.to_string();
            let spawned = std::thread::Builder::new()
                .name("longhand-device-watch".into())
                .spawn(move || manager.watch_for_device(&app, &id));
            if let Err(e) = spawned {
                log::error!("could not watch for the microphone's return: {e}");
            }
        }
    }

    /// While the session is Interrupted(device), compare the system's inputs
    /// every [`DEVICE_POLL`]; each change that leaves a default input is one
    /// attempt to resume (U14). A failed attempt waits for the next change.
    fn watch_for_device(self: Arc<Self>, app: &AppHandle, id: &str) {
        let pending = Pending::new(id);
        let cancel = pending.cancel.clone();
        if let Some(previous) = self.inner().watching.replace(pending) {
            previous.cancel.cancel();
        }
        let mut before = CpalAudio::inputs();
        while sleep_unless(&cancel, DEVICE_POLL) {
            let waiting = self.store().session(id).ok().flatten().is_some_and(|r| {
                matches!(
                    r.state,
                    SessionState::Interrupted {
                        reason: InterruptReason::Device,
                        ..
                    }
                )
            });
            if !waiting {
                break;
            }
            let inputs = CpalAudio::inputs();
            let returned = device_returned(&before, &inputs);
            before = inputs;
            if !returned {
                continue;
            }
            let mut inner = self.inner();
            if cancel.is_cancelled() {
                return;
            }
            if inner.active.is_some() || inner.recovering.is_some() {
                break;
            }
            log::info!("inputs changed while {id} waits for its microphone; trying it");
            if let Err(e) = self.reopen(app, &mut inner, id) {
                log::warn!("the microphone is still not usable: {e}");
                continue;
            }
            if let Err(e) = self.store().apply(id, &Event::DeviceReturned) {
                log::error!("resuming {id} failed: {e}");
                if let Some(active) = inner.active.take() {
                    let _ = self.close_segment(&mut inner, active);
                }
                break;
            }
            if let Some(clock) = inner.clock.as_mut() {
                clock.restarts = RestartTracker::new();
            }
            inner.watching = None;
            drop(inner);
            log::info!("session {id} resumed on its microphone's return");
            app.state::<Arc<AlertsManager>>().clear(id);
            changed(app, Some(id));
            return;
        }
        let mut inner = self.inner();
        if !cancel.is_cancelled() {
            inner.watching = None;
        }
    }

    /// Open the held session's next segment where its clock stands now, on
    /// whatever input is the default by then and at that input's rate.
    fn reopen(&self, app: &AppHandle, inner: &mut Inner, id: &str) -> Result<(), CaptureError> {
        self.audio.forget();
        inner.sample_rate = None;
        let (index, offset) = {
            let clock = inner
                .clock
                .as_mut()
                .filter(|c| c.session_id == id)
                .ok_or_else(|| CaptureError::Other("the session's timeline is not held".into()))?;
            let last_end = clock.timeline.segments().last().map_or(0, |s| s.end_ms());
            let offset = (clock.epoch.elapsed().as_millis() as u64).max(last_end);
            (clock.timeline.open(offset, now())?, offset)
        };
        match self.open_segment(app, inner, id, index, offset) {
            Ok(active) => {
                inner.active = Some(active);
                log::info!("session {id} reopened as segment {index} at {offset} ms");
                Ok(())
            }
            Err(e) => {
                if let Some(clock) = inner.clock.as_mut() {
                    clock.timeline.abandon_open();
                }
                Err(e)
            }
        }
    }

    fn open_segment(
        &self,
        app: &AppHandle,
        inner: &mut Inner,
        session_id: &str,
        segment: u32,
        offset_ms: u64,
    ) -> Result<ActiveSegment, CaptureError> {
        let rate = match inner.sample_rate {
            Some(rate) => rate,
            None => {
                let rate = self.audio.prepare(None)?;
                inner.sample_rate = Some(rate);
                rate
            }
        };
        let stream = match self
            .audio
            .open_mic_capture(None, 0, (rate / FRAMES_PER_SECOND) as usize)
        {
            Ok(stream) => stream,
            Err(e) => {
                // The device may have changed since `prepare`.
                inner.sample_rate = None;
                return Err(e.into());
            }
        };
        let recorded_before: u64 = self
            .store()
            .segments(session_id)?
            .iter()
            .map(|s| s.duration_ms)
            .sum();
        let capture_started_at = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let writer = ChunkedWriter::create(SegmentArgs {
            dir: self.session_dir(session_id).join(RECOVERY_DIR),
            segment,
            capture_started_at,
            offset_ms,
            sample_rate: rate,
            floor_bytes: DISK_RESERVE_BYTES,
        })?;

        let stop = CancellationToken::new();
        let (tx, outcome) = std_mpsc::channel();
        let thread_stop = stop.clone();
        let thread_app = app.clone();
        let thread_session = session_id.to_string();
        let thread = std::thread::Builder::new()
            .name(format!("longhand-segment-{segment}"))
            .spawn(move || {
                // Held by this thread for exactly as long as it records (R4, R5d).
                let _awake = power::IdleGuard::acquire();
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        log::error!("segment runtime failed to start: {e}");
                        return;
                    }
                };
                let tick_app = thread_app.clone();
                let tick_session = thread_session.clone();
                let result = runtime.block_on(recorder::run_segment(
                    stream,
                    writer,
                    thread_stop,
                    move |tick| {
                        let _ = CaptureTick {
                            session_id: tick_session.clone(),
                            peak: tick.peak,
                            position_ms: tick.position_ms,
                            recorded_ms: recorded_before + tick.segment_ms,
                        }
                        .emit(&tick_app);
                    },
                ));
                let loss = Loss::of(&result.end);
                // Sent before anything takes the manager's lock: a stop in
                // progress holds it while it waits for this outcome.
                let _ = tx.send(result);
                if let Some(loss) = loss {
                    log::warn!(
                        "segment {segment} of {thread_session} ended: {}",
                        loss.message
                    );
                    let manager = Arc::clone(&thread_app.state::<Arc<CaptureManager>>());
                    let spawned = std::thread::Builder::new()
                        .name("longhand-restart".into())
                        .spawn(move || {
                            manager.segment_lost(&thread_app, &thread_session, segment, loss)
                        });
                    if let Err(e) = spawned {
                        log::error!("could not start the restart: {e}");
                    }
                }
            })
            .map_err(|e| CaptureError::Other(e.to_string()))?;
        Ok(ActiveSegment {
            session_id: session_id.to_string(),
            segment,
            stop,
            outcome,
            thread,
        })
    }

    /// Stop the segment, then keep it on the timeline and in the store, or
    /// drop it and its audio when it ran under a second (R35).
    fn close_segment(&self, inner: &mut Inner, active: ActiveSegment) -> Result<(), CaptureError> {
        let ActiveSegment {
            session_id,
            stop,
            outcome,
            thread,
            ..
        } = active;
        stop.cancel();
        let outcome = outcome.recv();
        let _ = thread.join();
        let dir = self.session_dir(&session_id).join(RECOVERY_DIR);
        let span = match outcome {
            Ok(SegmentOutcome { span: Ok(span), .. }) => span,
            Ok(SegmentOutcome { span: Err(e), .. }) => {
                log::error!("closing the last chunk failed: {e}; recovering from disk");
                recovered_span(&dir)?
            }
            Err(_) => recovered_span(&dir)?,
        };
        let Some(clock) = inner.clock.as_mut().filter(|c| c.session_id == session_id) else {
            return Err(CaptureError::Other(
                "segment closed without its timeline".into(),
            ));
        };
        let index = span.segment;
        match clock
            .timeline
            .close(span.end_ms, now(), segment_audio_file(index))?
        {
            Closed::Kept(segment) => self.store().put_segment(&session_id, &segment)?,
            Closed::Discarded => {
                recorder::remove_segment(&dir, index).map_err(recorder::RecorderError::from)?;
                log::info!("segment {index} of {session_id} was under a second and was dropped");
            }
        }
        Ok(())
    }
}

fn changed(app: &AppHandle, session_id: Option<&str>) {
    let _ = RecordingChanged {
        session_id: session_id.map(str::to_string),
    }
    .emit(app);
}

/// The span of the last segment on disk, after closing any chunk a failed
/// writer left open.
fn recovered_span(dir: &Path) -> Result<recorder::SegmentSpan, CaptureError> {
    recorder::recover_partial_chunks(dir).map_err(recorder::RecorderError::from)?;
    let chunks = recorder::list_chunks(dir).map_err(recorder::RecorderError::from)?;
    let last = chunks.iter().map(|c| c.segment).max();
    let Some(segment) = last else {
        return Err(CaptureError::Other("no audio was written".into()));
    };
    let of_segment: Vec<_> = chunks.iter().filter(|c| c.segment == segment).collect();
    Ok(recorder::SegmentSpan {
        segment,
        start_ms: of_segment.first().map(|c| c.start_ms).unwrap_or(0),
        end_ms: of_segment.last().map(|c| c.end_ms).unwrap_or(0),
    })
}

/// Where the session clock stands after a wake: the wall-clock time away
/// since the last segment ended, when the monotonic clock shows less. Only
/// ever forward, and never back into the last segment, so a wall-clock step
/// cannot overlap it.
fn position_after_wake(elapsed_ms: u64, last_end_ms: u64, away_ms: u64) -> u64 {
    elapsed_ms.max(last_end_ms + away_ms)
}

/// Mark the session as restarting and hand out its budget.
fn begin_restart(inner: &mut Inner, id: &str) -> (CancellationToken, RestartTracker) {
    let pending = Pending::new(id);
    let cancel = pending.cancel.clone();
    inner.recovering = Some(pending);
    let restarts = match inner.clock.as_mut().filter(|c| c.session_id == id) {
        Some(clock) => std::mem::take(&mut clock.restarts),
        None => RestartTracker::new(),
    };
    (cancel, restarts)
}

/// Whether the inputs changed in a way worth one attempt: something changed
/// and there is a default input to open.
fn device_returned(before: &Inputs, now: &Inputs) -> bool {
    now.default.is_some() && now != before
}

/// The restart budget's clock: real time, with sleeps a stop can cut short.
struct Backoff<'a>(&'a CancellationToken);

impl supervisor::Clock for Backoff<'_> {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) -> bool {
        sleep_unless(self.0, duration)
    }
}

/// Sleep for `duration`, or until `cancel` fires. True when it slept out.
fn sleep_unless(cancel: &CancellationToken, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    while !cancel.is_cancelled() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
    false
}

/// Stop a segment whose session never came to exist, discarding its audio.
fn abandon(active: ActiveSegment) {
    active.stop.cancel();
    let _ = active.outcome.recv();
    let _ = active.thread.join();
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(secs: i64) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0)
            .unwrap()
            .timestamp_opt(secs, 0)
            .unwrap()
    }

    #[test]
    fn a_resume_in_the_same_process_measures_the_monotonic_gap() {
        // Stopped at 5 min, resumed at 7 min by the process clock; the wall
        // clock jumped (an NTP step) and is ignored.
        assert_eq!(
            resume_offset(300_000, at(1_000), at(5_000), Some(420_000)),
            420_000
        );
    }

    #[test]
    fn a_resume_after_a_restart_falls_back_to_the_wall_clock_gap() {
        assert_eq!(resume_offset(300_000, at(1_000), at(1_120), None), 420_000);
    }

    #[test]
    fn a_resume_never_overlaps_the_previous_segment() {
        assert_eq!(resume_offset(300_000, at(1_000), at(900), None), 300_000);
        assert_eq!(
            resume_offset(300_000, at(1_000), at(1_000), Some(10)),
            300_000
        );
    }

    fn inputs(default: Option<&str>, all: &[&str]) -> Inputs {
        Inputs {
            default: default.map(str::to_string),
            all: all.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_microphone_plugged_back_in_is_worth_an_attempt() {
        let gone = inputs(None, &[]);
        let back = inputs(Some("USB mic"), &["USB mic"]);
        assert!(device_returned(&gone, &back));
    }

    #[test]
    fn a_new_device_with_the_default_unchanged_is_still_worth_an_attempt() {
        // The default kept its name but failed; a new device may be the fix.
        let before = inputs(Some("Built-in"), &["Built-in"]);
        let after = inputs(Some("Built-in"), &["Built-in", "USB mic"]);
        assert!(device_returned(&before, &after));
    }

    #[test]
    fn no_change_or_no_default_is_no_attempt() {
        let same = inputs(Some("Built-in"), &["Built-in"]);
        assert!(!device_returned(&same, &same.clone()));
        let before = inputs(None, &["USB mic"]);
        assert!(!device_returned(&before, &inputs(None, &[])));
    }

    #[test]
    fn a_disk_floor_is_never_restarted_but_other_losses_are() {
        let floor = SegmentEnd::Storage(recorder::RecorderError::LowDisk {
            available: 1,
            floor: 2,
        });
        let loss = Loss::of(&floor).unwrap();
        assert!(!loss.restartable);
        assert_eq!(loss.reason, InterruptReason::Disk);
        let device = Loss::of(&SegmentEnd::SourceLost(audio::Error::MicStreamEnded)).unwrap();
        assert!(device.restartable);
        assert_eq!(device.reason, InterruptReason::Device);
        assert!(Loss::of(&SegmentEnd::Stopped).is_none());
    }

    #[test]
    fn a_wake_after_ten_minutes_leaves_a_ten_minute_gap() {
        // Paused at 5 min; the monotonic clock stood still through the sleep.
        assert_eq!(position_after_wake(300_000, 300_000, 600_000), 900_000);
        // A clock that did advance (Windows) is not pushed back.
        assert_eq!(position_after_wake(910_000, 300_000, 600_000), 910_000);
        // A wall clock stepped backwards cannot overlap the last segment.
        assert_eq!(position_after_wake(300_000, 300_000, 0), 300_000);
    }

    #[test]
    fn a_stop_cuts_the_backoff_short() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let started = Instant::now();
        assert!(!sleep_unless(&cancel, Duration::from_secs(4)));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(sleep_unless(
            &CancellationToken::new(),
            Duration::from_millis(1)
        ));
    }

    #[test]
    fn segment_audio_files_number_from_one() {
        assert_eq!(segment_audio_file(0), "audio/segment-01.opus");
        assert_eq!(segment_audio_file(11), "audio/segment-12.opus");
    }
}
