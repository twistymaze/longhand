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
//!
//! A note records from its chosen microphone (KTD1–KTD3 of the system audio
//! plan). A chosen microphone that is missing is replaced by the system
//! default at once, and the capture watcher moves the note back when the
//! chosen microphone is present in two polls in a row. Each input change
//! is a segment boundary through `reopen`.

use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use audio::CaptureStream;
use audio_cpal::{CpalAudio, Input, Inputs, MicChoice};
use chrono::{DateTime, FixedOffset, Local};
use pipeline::live::{LiveEnd, LiveWorker};
use pipeline::recover::segment_audio_file;
use recorder::storage::DISK_RESERVE_BYTES;
use recorder::{ChunkedWriter, SegmentArgs, SegmentEnd, SegmentOutcome};
use serde::Serialize;
use session_core::{
    CaptureChoice, Closed, Event, InterruptReason, Microphone, RecordingModel, SessionState,
    SpeakerCount, Store, Timeline,
};
use specta::Type;
use supervisor::{RestartTracker, SOURCE_RESTART_BUDGET};
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;
use tokio_util::sync::CancellationToken;

use crate::managers::alerts::{self, AlertsManager};
use crate::managers::live::LiveManager;
use crate::managers::models::ModelsManager;
use crate::managers::settings::{self, MicrophoneChoice, Settings};

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
    Save,
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
    /// What a four-hour session at the microphone's rate needs there, its
    /// processing included (R7b); `None` until the microphone is prepared.
    pub session_demand_bytes: Option<u64>,
    /// What each recorded second will need once processing begins.
    pub processing_bytes_per_second: u64,
    /// The free-space floor recording stops at (R7a).
    pub reserve_bytes: u64,
    /// The name of the input that the segment records from, while a
    /// segment records (R2, R4).
    pub microphone: Option<String>,
    /// The note's chosen microphone is missing, and the system default
    /// records in its place (R4, R5).
    pub microphone_fallback: bool,
    /// The microphone that the open note chose. With no note open, the
    /// microphone that the next note uses. `None` is "System default".
    pub microphone_choice: Option<MicrophoneChoice>,
    /// The Computer audio permission is unknown, the output plays, and the
    /// system audio channel has been digital silence for 10 s: Computer
    /// audio can be missing (U5 step 6). It clears when audio appears.
    pub computer_audio_silent: bool,
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
    /// The segment's live worker, when the note is in live mode (U6).
    live: Option<LiveWorker>,
    /// The name of the input that the segment records from.
    input: String,
    /// The note's chosen microphone is missing, and the system default
    /// records in its place (R4, R5).
    fallback: bool,
    /// An input change opened the segment. It is kept when it closes, even
    /// when it is shorter than a second (KTD3).
    opened_at_switch: bool,
}

/// What [`CaptureManager::open_segment`] opens.
struct Opening<'a> {
    session_id: &'a str,
    segment: u32,
    offset_ms: u64,
    /// The model of a note in live mode (U6).
    live: Option<&'a str>,
    /// The note's microphone. `None` is "System default".
    choice: Option<&'a MicChoice>,
    /// An input change opens the segment (KTD3).
    at_switch: bool,
}

/// An input resolved ahead of the open (R1b, R3): its name and native rate.
#[derive(Debug, Clone)]
struct Prepared {
    /// The choice it was resolved for. `None` is "System default".
    choice: Option<MicChoice>,
    input: Input,
    rate: u32,
    /// The choice did not resolve, and `input` is the system default.
    fallback: bool,
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
    /// The input that the next open uses, when it matches the choice.
    prepared: Option<Prepared>,
    /// A lost segment being restarted.
    recovering: Option<Pending>,
    /// An Interrupted(device) session waiting for its microphone.
    watching: Option<Pending>,
    /// The capture watcher of the recording note (KTD3): it polls while the
    /// note records on a fallback, to move back to the chosen microphone.
    capture_watch: Option<Pending>,
    /// A microphone chosen on the recording page before the note's row
    /// exists (KTD2). The next start writes it with the row. The inner
    /// `None` is "System default".
    held_microphone: Option<Option<MicrophoneChoice>>,
    /// The silent Computer audio flag (U5 step 6). While Computer audio is
    /// on and the permission is unknown, the capture watcher polls it with
    /// the permission, `audio_system::output_running()` and the right
    /// channel's `SilenceRun` (U6). `status` reports it.
    silent_check: audio_system::SilentCheck,
}

impl Inner {
    /// Stop any restart, device watch or capture watch: a user action
    /// decides now.
    fn supersede(&mut self) -> Option<String> {
        if let Some(watch) = self.watching.take() {
            watch.cancel.cancel();
        }
        self.stop_capture_watch();
        self.recovering.take().map(|p| {
            p.cancel.cancel();
            p.session_id
        })
    }

    /// Stop the capture watcher: Stop, sleep, supersede and restart do this
    /// (KTD3).
    fn stop_capture_watch(&mut self) {
        if let Some(watch) = self.capture_watch.take() {
            watch.cancel.cancel();
        }
    }
}

/// A saved choice as the capture crate takes it.
impl From<&MicrophoneChoice> for MicChoice {
    fn from(choice: &MicrophoneChoice) -> Self {
        Self {
            id: choice.id.clone(),
            label: choice.label.clone(),
        }
    }
}

impl From<&MicrophoneChoice> for Microphone {
    fn from(choice: &MicrophoneChoice) -> Self {
        Self {
            id: choice.id.clone(),
            label: choice.label.clone(),
        }
    }
}

impl From<&Microphone> for MicrophoneChoice {
    fn from(microphone: &Microphone) -> Self {
        Self {
            id: microphone.id.clone(),
            label: microphone.label.clone(),
        }
    }
}

/// The microphone of a note's capture choice as the capture crate takes
/// it. `None` is "System default".
fn mic_of(choice: &CaptureChoice) -> Option<MicChoice> {
    choice.microphone.as_ref().map(|m| MicChoice {
        id: m.id.clone(),
        label: m.label.clone(),
    })
}

/// A microphone choice needs a device ID and a name, as in Settings.
pub fn check_microphone(choice: Option<&MicrophoneChoice>) -> Result<(), String> {
    match choice {
        Some(c) if c.id.trim().is_empty() || c.label.trim().is_empty() => {
            Err("A microphone choice needs a device ID and a name.".into())
        }
        _ => Ok(()),
    }
}

/// The capture choice that a note records when it starts (KTD2): the
/// microphone held from the recording page, else the one from Settings,
/// and the Computer audio default.
pub fn capture_choice_at_start(
    held: Option<Option<MicrophoneChoice>>,
    settings: &Settings,
) -> CaptureChoice {
    let microphone = held.unwrap_or_else(|| settings.microphone.clone());
    CaptureChoice {
        microphone: microphone.as_ref().map(Microphone::from),
        computer_audio: settings.computer_audio_default,
    }
}

/// Open `choice` with `open`. When a chosen microphone is missing, open the
/// system default in its place (R4, KD9). Returns what `open` gave and
/// whether it is the fallback. Only a missing input falls back: another
/// failure goes to the restart path.
fn with_fallback<T>(
    choice: Option<&MicChoice>,
    mut open: impl FnMut(Option<&MicChoice>) -> Result<T, audio::Error>,
) -> Result<(T, bool), audio::Error> {
    match open(choice) {
        Err(audio::Error::NoInputDevice) if choice.is_some() => {
            log::info!("the chosen microphone is not connected; using the system default");
            open(None).map(|opened| (opened, true))
        }
        opened => opened.map(|opened| (opened, false)),
    }
}

/// Whether the system lists the chosen microphone now.
fn chosen_present(choice: Option<&MicChoice>, inputs: &Inputs) -> bool {
    choice.is_some_and(|c| audio_cpal::find_choice(&inputs.all, c).is_some())
}

/// What a segment that ended on its own does next (KTD3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterLoss {
    /// The chosen microphone went away and a default input exists: open the
    /// default at once, with no restart backoff (R5, KD10).
    FallBackNow,
    /// Reopen through the restart budget. When no input remains, the budget
    /// runs out and the note is Interrupted(device), as today.
    Restart,
}

/// The source selection after a loss (KTD3). A loss of the fallback input
/// uses the restart budget, so a default that keeps failing does not
/// reopen in a loop.
fn after_loss(choice: Option<&MicChoice>, was_fallback: bool, inputs: &Inputs) -> AfterLoss {
    let chosen_gone = choice.is_some() && !chosen_present(choice, inputs);
    if chosen_gone && !was_fallback && inputs.default.is_some() {
        AfterLoss::FallBackNow
    } else {
        AfterLoss::Restart
    }
}

/// The move back from a fallback (KTD3): the chosen microphone must be
/// present in two polls in a row, so a headset at the edge of its range
/// does not switch the note back and forth.
#[derive(Debug, Default)]
struct MoveBack {
    seen: u32,
}

impl MoveBack {
    /// Record one poll. True when the note moves back now.
    fn poll(&mut self, present: bool) -> bool {
        self.seen = if present { self.seen + 1 } else { 0 };
        if self.seen >= 2 {
            self.seen = 0;
            return true;
        }
        false
    }
}

/// What the capture watcher of `id` does at a poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchStep {
    /// A restart runs: do nothing at this poll.
    Wait,
    /// The note records on a fallback: look for the chosen microphone.
    MoveBack,
    /// Nothing to watch: the watcher ends.
    End,
}

fn watch_step(inner: &Inner, id: &str) -> WatchStep {
    if inner.recovering.is_some() {
        return WatchStep::Wait;
    }
    match &inner.active {
        Some(active) if active.session_id == id && active.fallback => WatchStep::MoveBack,
        _ => WatchStep::End,
    }
}

/// Whether a change of `id`'s choice opens a new segment now (KTD2). Only
/// a segment that records does. During Interrupted, restart backoff or a
/// sleep pause, the next segment reads the stored choice.
fn change_reopens(inner: &Inner, id: &str) -> bool {
    inner.recovering.is_none() && inner.active.as_ref().is_some_and(|a| a.session_id == id)
}

/// Close the timeline's open segment. At a switch, a short segment is kept
/// (KTD3); otherwise a segment under a second is dropped (R35).
fn close_timeline(
    timeline: &mut Timeline,
    end_ms: u64,
    wall_end: DateTime<FixedOffset>,
    audio_file: String,
    at_switch: bool,
) -> Result<Closed, session_core::TimelineError> {
    if at_switch {
        timeline.close_at_switch(end_ms, wall_end, audio_file)
    } else {
        timeline.close(end_ms, wall_end, audio_file)
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

/// The model and the live mode that a session records when it starts
/// (KTD2). The live mode is true only when the selected model streams and
/// the Live transcript switch is on (R18).
pub fn recording_model(settings: &Settings) -> RecordingModel {
    let streams = models::model(&settings.asr_model).is_some_and(|m| m.streaming);
    RecordingModel {
        asr_model: settings.asr_model.clone(),
        live_transcript: streams && settings.live_transcript,
    }
}

/// The model that a resume records (KTD2). A session keeps its model while
/// that model is downloaded, and then this returns `None`. Otherwise the
/// session gets the selected model.
pub fn model_at_resume(
    recorded: Option<&str>,
    settings: &Settings,
    is_present: impl Fn(&str) -> bool,
) -> Option<RecordingModel> {
    match recorded {
        Some(model) if is_present(model) => None,
        _ => Some(recording_model(settings)),
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

    /// At launch, before anything records: sessions a dead process left
    /// recording get their open segment back from its chunks and become
    /// Interrupted(crash), or are discarded if they kept nothing (U16).
    pub fn recover_at_launch(&self) {
        match pipeline::recover::recover_crashed(&self.store, &self.working_root) {
            Ok(recovered) => {
                for session in recovered {
                    log::warn!(
                        "recovered {} after a crash: {}",
                        session.session_id,
                        session.state.name()
                    );
                }
            }
            Err(e) => log::error!("crash recovery at launch failed: {e}"),
        }
    }

    /// Resolve the microphone and its format ahead of the press, so pressing
    /// the shortcut only opens the stream (R1b).
    /// The microphone is the one that the next note uses: the one held from
    /// the recording page, else the one from Settings (R3).
    pub fn prepare(&self, app: &AppHandle) -> Result<u32, CaptureError> {
        let held = self.inner().held_microphone.clone();
        let microphone = held.unwrap_or_else(|| settings::get_settings(app).microphone);
        let choice = microphone.as_ref().map(MicChoice::from);
        let ((input, rate), fallback) =
            with_fallback(choice.as_ref(), |target| self.audio.prepare_input(target))?;
        log::info!(
            "microphone '{}' prepared at {rate} Hz (fallback: {fallback})",
            input.name
        );
        self.inner().prepared = Some(Prepared {
            choice,
            input,
            rate,
            fallback,
        });
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
        let mut ended = inner.supersede();
        if let Some(active) = inner.active.take() {
            ended = Some(active.session_id.clone());
            self.close_segment(&mut inner, active, false)?;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let started = now();
        let mut timeline = Timeline::new();
        let index = timeline.open(0, started)?;
        let settings = settings::get_settings(app);
        let model = recording_model(&settings);
        let capture = capture_choice_at_start(inner.held_microphone.clone(), &settings);
        let live = model.live_transcript.then_some(model.asr_model.as_str());
        let choice = mic_of(&capture);
        let opening = Opening {
            session_id: &id,
            segment: index,
            offset_ms: 0,
            live,
            choice: choice.as_ref(),
            at_switch: false,
        };
        let active = match self.open_segment(app, &mut inner, opening) {
            Ok(active) => active,
            Err(e) => {
                // The session this press ended is already closed: stop and
                // queue it, as the new session would have, rather than leave
                // it Capturing with nothing recording.
                let _ = std::fs::remove_dir_all(self.session_dir(&id));
                if let Some(other) = ended {
                    match self.stop_in_store(app, &other) {
                        Ok(_) => log::info!("session {other} stopped by a start that failed"),
                        Err(stop) => log::error!("stopping {other} failed: {stop}"),
                    }
                    drop(inner);
                    changed(app, None);
                }
                return Err(e);
            }
        };
        let stopped =
            match self
                .store()
                .start_session(&id, started, speaker_count, &model, &capture)
            {
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
        inner.held_microphone = None;
        inner.active = Some(active);
        inner.clock = Some(Clock::new(&id, timeline, Instant::now()));
        log::info!(
            "session {id} recording with {} (live transcript: {})",
            model.asr_model,
            model.live_transcript
        );
        self.watch_capture(app, &mut inner, &id);
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
                self.close_segment(&mut inner, active, false)?;
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
        let state = self.stop_in_store(app, &id)?;
        drop(inner);
        app.state::<Arc<AlertsManager>>().clear(&id);
        changed(app, None);
        Ok((id, state))
    }

    /// Stop `id` in the store: queued for processing, or discarded when it
    /// kept no audio. Its segment must already be closed.
    fn stop_in_store(&self, app: &AppHandle, id: &str) -> Result<SessionState, CaptureError> {
        let retained_audio = !self.store().segments(id)?.is_empty();
        let state = self.store().apply(
            id,
            &Event::UserStop {
                at: now(),
                retained_audio,
            },
        )?;
        if retained_audio {
            self.record_model_at_stop(app, id);
        } else {
            // Nothing survived R35; the notice is the surface's (U7).
            let _ = std::fs::remove_dir_all(self.session_dir(id));
            log::info!("session {id} kept no audio and was discarded");
        }
        Ok(state)
    }

    /// A session keeps the model that it recorded at its start (KTD2). Only
    /// a session with no recorded model, one that started before the
    /// update, gets the model selected now.
    fn record_model_at_stop(&self, app: &AppHandle, id: &str) {
        let model = settings::get_settings(app).asr_model;
        match self.store().record_model_if_unset(id, &model) {
            Ok(true) => log::info!("session {id} had no model and records {model} at stop"),
            Ok(false) => {}
            Err(e) => log::warn!("could not record the model for {id}: {e}"),
        }
    }

    /// A resume keeps the session's model while that model is downloaded.
    /// Otherwise the session records the selected model and live mode
    /// (KTD2).
    fn record_model_at_resume(&self, app: &AppHandle, id: &str) {
        let recorded = match self.store().session(id) {
            Ok(record) => record.and_then(|r| r.asr_model),
            Err(e) => {
                log::warn!("could not read the model of {id}: {e}");
                return;
            }
        };
        let models = app.state::<Arc<ModelsManager>>();
        let settings = settings::get_settings(app);
        let Some(model) = model_at_resume(recorded.as_deref(), &settings, |m| models.is_present(m))
        else {
            return;
        };
        match self.store().set_recording_model(id, &model) {
            Ok(()) => log::info!(
                "session {id} resumes with {} (live transcript: {}) in place of {recorded:?}",
                model.asr_model,
                model.live_transcript
            ),
            Err(e) => log::warn!("could not record the model for {id}: {e}"),
        }
    }

    /// Resume an interrupted session as its next segment (R6). The store
    /// decides whether the resume is allowed (window, most recent, space).
    pub fn resume_session(
        self: &Arc<Self>,
        app: &AppHandle,
        id: &str,
    ) -> Result<SessionState, CaptureError> {
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
        // Under one lock with the save claim: either saving has begun and
        // the resume is refused, or the resume wins and saving stops. A
        // completed session has finished saving and resumes into its folder
        // while that holds its audio (R6d).
        let state = {
            let mut store = self.store();
            let other = |e: pipeline::StageError| CaptureError::Other(e.to_string());
            let completed = store
                .session(id)?
                .is_some_and(|r| r.state == SessionState::Saved);
            if !completed && pipeline::save::begun(&store, id).map_err(other)? {
                return Err(CaptureError::Save);
            }
            let audio_intact = pipeline::save::home_intact(&store, id).map_err(other)?;
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
        self.record_model_at_resume(app, id);
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
        let epoch = epoch.unwrap_or_else(|| {
            let now = Instant::now();
            now.checked_sub(Duration::from_millis(offset))
                .unwrap_or(now)
        });
        let live = self.live_model(id);
        // A user resume and a crash resume keep the note's choice (KTD2).
        let choice = self.note_microphone(id);
        let opening = Opening {
            session_id: id,
            segment: index,
            offset_ms: offset,
            live: live.as_deref(),
            choice: choice.as_ref(),
            at_switch: false,
        };
        let active = match self.open_segment(app, &mut inner, opening) {
            Ok(active) => active,
            Err(e) => {
                // The store already says Capturing: put the session back to
                // Interrupted, with its alert and the watch for the
                // microphone's return, which reopens from the held clock.
                timeline.abandon_open();
                inner.clock = Some(Clock::new(id, timeline, epoch));
                drop(inner);
                let loss = Loss {
                    reason: InterruptReason::Device,
                    message: format!("the microphone could not be opened: {e}"),
                    restartable: false,
                };
                Arc::clone(self).interrupt(app, id, &loss, None);
                return Err(e);
            }
        };
        inner.active = Some(active);
        inner.clock = Some(Clock::new(id, timeline, epoch));
        log::info!("session {id} resumed as segment {index} at {offset} ms");
        self.watch_capture(app, &mut inner, id);
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

    pub fn status(&self, app: &AppHandle) -> Result<RecordingStatus, CaptureError> {
        let (recording, reconnecting, sample_rate, input, held, computer_audio_silent) = {
            let inner = self.inner();
            let reconnecting = inner.recovering.is_some();
            (
                inner.active.is_some() || reconnecting,
                reconnecting,
                inner.prepared.as_ref().map(|p| p.rate),
                inner.active.as_ref().map(|a| (a.input.clone(), a.fallback)),
                inner.held_microphone.clone(),
                inner.silent_check.flagged(),
            )
        };
        let open = self.open_session()?;
        let microphone_choice = match open.as_ref() {
            Some((id, _)) => self
                .store()
                .capture_choice(id)?
                .microphone
                .as_ref()
                .map(MicrophoneChoice::from),
            None => held.unwrap_or_else(|| settings::get_settings(app).microphone),
        };
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
            session_demand_bytes: sample_rate.map(|rate| {
                recorder::storage::session_demand(rate, recorder::storage::PLANNED_SESSION_SECONDS)
            }),
            processing_bytes_per_second: recorder::storage::PROCESSING_BYTES_PER_SECOND,
            reserve_bytes: DISK_RESERVE_BYTES,
            microphone_fallback: input.as_ref().is_some_and(|(_, fallback)| *fallback),
            microphone: input.map(|(name, _)| name),
            microphone_choice,
            computer_audio_silent,
        })
    }

    /// The microphone that a note chose, from the store (KTD2). A note with
    /// no stored choice, or a store that cannot be read, records from the
    /// system default.
    fn note_microphone(&self, id: &str) -> Option<MicChoice> {
        match self.store().capture_choice(id) {
            Ok(choice) => mic_of(&choice),
            Err(e) => {
                log::warn!("could not read the capture choice of {id}: {e}");
                None
            }
        }
    }

    /// Change the microphone of one note from the recording page (R2, KTD2).
    /// Settings does not change. With no `session_id`, the choice is held
    /// for the next note and written with its row. For a note, the stored
    /// choice changes, and a segment that records now closes and the next
    /// one opens on the new choice (AE8). During Interrupted, restart
    /// backoff or a sleep pause, the next segment reads the stored choice.
    pub fn set_note_microphone(
        self: &Arc<Self>,
        app: &AppHandle,
        session_id: Option<&str>,
        microphone: Option<MicrophoneChoice>,
    ) -> Result<(), CaptureError> {
        check_microphone(microphone.as_ref()).map_err(CaptureError::Other)?;
        let Some(id) = session_id else {
            log::info!("the next note uses the microphone {microphone:?}");
            self.inner().held_microphone = Some(microphone);
            // Resolve it off the start path (R3).
            if let Err(e) = self.prepare(app) {
                log::warn!("the held microphone was not prepared: {e}");
            }
            return Ok(());
        };
        let mut inner = self.inner();
        let mut choice = self.store().capture_choice(id)?;
        let next = microphone.as_ref().map(Microphone::from);
        if choice.microphone == next {
            return Ok(());
        }
        choice.microphone = next;
        self.store().set_capture_choice(id, &choice)?;
        log::info!("session {id} changes its microphone to {microphone:?}");
        if !change_reopens(&inner, id) {
            return Ok(());
        }
        self.switch_input(app, &mut inner, id);
        Ok(())
    }

    /// Close the recording segment of `id` at an input change and open the
    /// next one on the note's choice (KTD3). When the open fails, the note
    /// restarts through the restart budget on its own thread.
    fn switch_input(&self, app: &AppHandle, inner: &mut Inner, id: &str) {
        if let Some(active) = inner.active.take()
            && let Err(e) = self.close_segment(inner, active, true)
        {
            log::error!("closing the segment of {id} at an input change failed: {e}");
        }
        match self.reopen(app, inner, id, true) {
            Ok(()) => changed(app, Some(id)),
            Err(e) => {
                log::warn!("the input change of {id} failed: {e}; restarting");
                let (cancel, restarts) = begin_restart(inner, id);
                let loss = Loss {
                    reason: InterruptReason::Device,
                    message: format!("the microphone could not be opened: {e}"),
                    restartable: true,
                };
                let manager = Arc::clone(&app.state::<Arc<CaptureManager>>());
                let app = app.clone();
                let id = id.to_string();
                let spawned = std::thread::Builder::new()
                    .name("longhand-restart".into())
                    .spawn(move || manager.restart(&app, &id, cancel, restarts, loss));
                if let Err(e) = spawned {
                    log::error!("could not start the restart: {e}");
                }
            }
        }
    }

    /// Start the capture watcher of `id` when its segment records on a
    /// fallback and no watcher runs (KTD3). U6 adds the speaker stream and
    /// the permission to what it watches.
    fn watch_capture(&self, app: &AppHandle, inner: &mut Inner, id: &str) {
        if watch_step(inner, id) != WatchStep::MoveBack {
            return;
        }
        if inner
            .capture_watch
            .as_ref()
            .is_some_and(|w| w.session_id == id && !w.cancel.is_cancelled())
        {
            return;
        }
        inner.stop_capture_watch();
        let pending = Pending::new(id);
        let cancel = pending.cancel.clone();
        inner.capture_watch = Some(pending);
        let manager = Arc::clone(&app.state::<Arc<CaptureManager>>());
        let app = app.clone();
        let id = id.to_string();
        let spawned = std::thread::Builder::new()
            .name("longhand-capture-watch".into())
            .spawn(move || manager.run_capture_watch(&app, &id, &cancel));
        if let Err(e) = spawned {
            log::error!("could not start the capture watcher: {e}");
            inner.capture_watch = None;
        }
    }

    /// The capture watcher (KTD3). Every [`DEVICE_POLL`] it looks for the
    /// note's chosen microphone while the note records on a fallback, and
    /// after two polls in a row it moves back, in a new segment. It takes
    /// the inner lock like `reopen`, and it does nothing while a restart
    /// runs. Stop, sleep, supersede and restart cancel it.
    fn run_capture_watch(self: Arc<Self>, app: &AppHandle, id: &str, cancel: &CancellationToken) {
        let mut move_back = MoveBack::default();
        let mut watched: Option<MicChoice> = None;
        while sleep_unless(cancel, DEVICE_POLL) {
            let choice = self.note_microphone(id);
            if choice != watched {
                move_back = MoveBack::default();
                watched.clone_from(&choice);
            }
            let inputs = CpalAudio::inputs();
            let mut inner = self.inner();
            if cancel.is_cancelled() {
                return;
            }
            match watch_step(&inner, id) {
                WatchStep::Wait => {
                    move_back = MoveBack::default();
                    continue;
                }
                WatchStep::End => {
                    inner.capture_watch = None;
                    return;
                }
                WatchStep::MoveBack => {}
            }
            if !move_back.poll(chosen_present(choice.as_ref(), &inputs)) {
                continue;
            }
            log::info!("the chosen microphone of {id} is back; moving back to it");
            self.switch_input(app, &mut inner, id);
        }
    }

    /// A session's notes document, in any state (R20).
    pub fn session_notes(&self, session_id: &str) -> Result<String, CaptureError> {
        Ok(self.store().notes(session_id)?)
    }

    /// Replace a session's notes document (KTD5). On a saved session the
    /// home folder's `notes.md` is rewritten whole under the store lock
    /// (R20). A session id that the store does not hold is refused: the
    /// app keeps no notes outside a session (R27).
    pub fn set_session_notes(&self, session_id: &str, document: &str) -> Result<(), CaptureError> {
        let mut store = self.store();
        pipeline::save::set_notes(&mut store, session_id, document)
            .map_err(|e| CaptureError::Other(e.to_string()))?;
        Ok(())
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
        // Read before the lock: the list queries the audio HAL.
        let inputs = CpalAudio::inputs();
        let choice = self.note_microphone(id);
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
            let was_fallback = inner.active.as_ref().is_some_and(|a| a.fallback);
            if let Some(active) = inner.active.take()
                && let Err(e) = self.close_segment(&mut inner, active, false)
            {
                log::error!("closing the lost segment of {id} failed: {e}");
            }
            if !loss.restartable {
                drop(inner);
                self.interrupt(app, id, &loss, None);
                return;
            }
            if loss.reason == InterruptReason::Device
                && after_loss(choice.as_ref(), was_fallback, &inputs) == AfterLoss::FallBackNow
            {
                log::info!("the chosen microphone of {id} went away; using the system default");
                match self.reopen(app, &mut inner, id, true) {
                    Ok(()) => {
                        drop(inner);
                        changed(app, Some(id));
                        return;
                    }
                    Err(e) => log::warn!("the fallback of {id} failed: {e}; restarting"),
                }
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
        inner.stop_capture_watch();
        let id = match inner.active.take() {
            Some(active) => {
                let id = active.session_id.clone();
                if let Err(e) = self.close_segment(&mut inner, active, false) {
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
            self.reopen(app, &mut inner, id, false)
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
            if let Err(e) = self.reopen(app, &mut inner, id, false) {
                log::warn!("the microphone is still not usable: {e}");
                continue;
            }
            if let Err(e) = self.store().apply(id, &Event::DeviceReturned) {
                log::error!("resuming {id} failed: {e}");
                inner.stop_capture_watch();
                if let Some(active) = inner.active.take() {
                    let _ = self.close_segment(&mut inner, active, false);
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
    /// the note's choice as the inputs are by then (KTD3), and at that
    /// input's rate. `at_switch` marks an input change: the new segment is
    /// kept when it closes, even when it is short.
    fn reopen(
        &self,
        app: &AppHandle,
        inner: &mut Inner,
        id: &str,
        at_switch: bool,
    ) -> Result<(), CaptureError> {
        self.audio.forget();
        inner.prepared = None;
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
        let live = self.live_model(id);
        let choice = self.note_microphone(id);
        let opening = Opening {
            session_id: id,
            segment: index,
            offset_ms: offset,
            live: live.as_deref(),
            choice: choice.as_ref(),
            at_switch,
        };
        match self.open_segment(app, inner, opening) {
            Ok(active) => {
                inner.active = Some(active);
                log::info!("session {id} reopened as segment {index} at {offset} ms");
                self.watch_capture(app, inner, id);
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

    /// The model of a note in live mode, from the store (KTD2). `None` when
    /// the note is not in live mode.
    fn live_model(&self, id: &str) -> Option<String> {
        match self.store().session(id) {
            Ok(Some(record)) if record.live_transcript == Some(true) => record.asr_model,
            Ok(_) => None,
            Err(e) => {
                log::warn!("could not read the live mode of {id}: {e}");
                None
            }
        }
    }

    /// Open the next segment. With `live`, the model of a note in live mode,
    /// a live worker streams the segment (U6); a worker that cannot start
    /// leaves the segment to a full pass after Stop.
    /// Open the microphone for `choice`, or the system default when a
    /// chosen microphone is missing (R4, KTD3). A prepared input for the
    /// same choice skips the resolve (R1b). A prepared fallback is resolved
    /// again, so a chosen microphone connected since then is used.
    fn open_microphone(
        &self,
        inner: &mut Inner,
        choice: Option<&MicChoice>,
    ) -> Result<(Prepared, CaptureStream), CaptureError> {
        let mut cached = inner
            .prepared
            .take()
            .filter(|p| p.choice.as_ref() == choice && !p.fallback)
            .map(|p| (p.input, p.rate));
        let opened = with_fallback(choice, |target| {
            let (input, rate) = match cached.take() {
                Some(hit) if target == choice => hit,
                _ => self.audio.prepare_input(target)?,
            };
            let chunk = (rate / FRAMES_PER_SECOND) as usize;
            let stream = self.audio.open_input(target.cloned(), 0, chunk)?;
            Ok((input, rate, stream))
        });
        match opened {
            Ok(((input, rate, stream), fallback)) => {
                let prepared = Prepared {
                    choice: choice.cloned(),
                    input,
                    rate,
                    fallback,
                };
                inner.prepared = Some(prepared.clone());
                Ok((prepared, stream))
            }
            // The device may have changed since `prepare`.
            Err(e) => Err(e.into()),
        }
    }

    fn open_segment(
        &self,
        app: &AppHandle,
        inner: &mut Inner,
        opening: Opening<'_>,
    ) -> Result<ActiveSegment, CaptureError> {
        let Opening {
            session_id,
            segment,
            offset_ms,
            live,
            choice,
            at_switch,
        } = opening;
        let (prepared, stream) = self.open_microphone(inner, choice)?;
        let rate = prepared.rate;
        log::info!(
            "segment {segment} of {session_id} records from '{}' (fallback: {})",
            prepared.input.name,
            prepared.fallback
        );
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
        let (tap, live) = live
            .and_then(|model| {
                app.state::<Arc<LiveManager>>()
                    .start(app, session_id, segment, model, rate)
            })
            .unzip();

        let stop = CancellationToken::new();
        let (tx, outcome) = std_mpsc::channel();
        let thread_stop = stop.clone();
        let thread_app = app.clone();
        let thread_session = session_id.to_string();
        let spawned = std::thread::Builder::new()
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
                let result = runtime.block_on(recorder::run_segment_with(
                    stream,
                    writer,
                    thread_stop,
                    move |tick| {
                        let recorded_ms = recorded_before + tick.segment_ms;
                        crate::tray::recording_tick(&tick_app, &tick_session, recorded_ms);
                        let _ = CaptureTick {
                            session_id: tick_session.clone(),
                            peak: tick.peak,
                            position_ms: tick.position_ms,
                            recorded_ms,
                        }
                        .emit(&tick_app);
                    },
                    // Never waits: a full channel drops the frame (KTD3).
                    move |index, pcm: &Arc<[f32]>| {
                        if let Some(tap) = &tap {
                            tap.offer(index, pcm);
                        }
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
            .map_err(|e| CaptureError::Other(e.to_string()));
        let thread = match spawned {
            Ok(thread) => thread,
            Err(e) => {
                if let Some(live) = live {
                    live.end(LiveEnd::Discard);
                }
                return Err(e);
            }
        };
        Ok(ActiveSegment {
            session_id: session_id.to_string(),
            segment,
            stop,
            outcome,
            thread,
            live,
            input: prepared.input.name,
            fallback: prepared.fallback,
            opened_at_switch: at_switch,
        })
    }

    /// Stop the segment, then keep it on the timeline and in the store, or
    /// drop it and its audio when it ran under a second (R35). The live
    /// worker then saves its last part, or deletes the parts of a dropped
    /// segment, on its own thread: processing waits for it (KTD9). A
    /// segment that ends at an input change (`at_switch`), or that one
    /// opened, is kept even when it is short (KTD3).
    fn close_segment(
        &self,
        inner: &mut Inner,
        active: ActiveSegment,
        at_switch: bool,
    ) -> Result<(), CaptureError> {
        let ActiveSegment {
            session_id,
            segment,
            stop,
            outcome,
            thread,
            live,
            opened_at_switch,
            ..
        } = active;
        // A close that fails before it decides keeps the live parts: the
        // worker reads a dropped end as a kept segment.
        let end_live = |how: LiveEnd| {
            if let Some(live) = live {
                live.end(how);
            }
        };
        stop.cancel();
        let outcome = outcome.recv();
        let _ = thread.join();
        let dir = self.session_dir(&session_id).join(RECOVERY_DIR);
        let end_ms = match outcome {
            Ok(SegmentOutcome { span: Ok(span), .. }) => span.end_ms,
            Ok(SegmentOutcome { span: Err(e), .. }) => {
                log::error!("closing the last chunk failed: {e}; recovering from disk");
                recovered_end(&dir, segment)?
            }
            Err(_) => recovered_end(&dir, segment)?,
        };
        let Some(clock) = inner.clock.as_mut().filter(|c| c.session_id == session_id) else {
            return Err(CaptureError::Other(
                "segment closed without its timeline".into(),
            ));
        };
        // The open segment's own index names its audio: never an earlier
        // segment's, whose chunks a discard would otherwise remove.
        let index = clock
            .timeline
            .open_segment()
            .map_or(segment, |open| open.index);
        match close_timeline(
            &mut clock.timeline,
            end_ms,
            now(),
            segment_audio_file(index),
            at_switch || opened_at_switch,
        )? {
            Closed::Kept(segment) => {
                end_live(LiveEnd::Keep);
                self.store().put_segment(&session_id, &segment)?
            }
            Closed::Discarded => {
                end_live(LiveEnd::Discard);
                recorder::remove_segment(&dir, index).map_err(recorder::RecorderError::from)?;
                log::info!("segment {index} of {session_id} was under a second and was dropped");
            }
        }
        Ok(())
    }
}

fn changed(app: &AppHandle, session_id: Option<&str>) {
    crate::tray::recording_changed(app, session_id);
    let _ = RecordingChanged {
        session_id: session_id.map(str::to_string),
    }
    .emit(app);
}

/// Where `segment` ends on disk, after closing any chunk a failed writer
/// left open. A segment that wrote nothing ends at 0, which the timeline
/// discards.
fn recovered_end(dir: &Path, segment: u32) -> Result<u64, CaptureError> {
    recorder::recover_partial_chunks(dir).map_err(recorder::RecorderError::from)?;
    let chunks = recorder::list_chunks(dir).map_err(recorder::RecorderError::from)?;
    Ok(chunks
        .iter()
        .filter(|c| c.segment == segment)
        .map(|c| c.end_ms)
        .max()
        .unwrap_or(0))
}

/// Where the session clock stands after a wake: the wall-clock time away
/// since the last segment ended, when the monotonic clock shows less. Only
/// ever forward, and never back into the last segment, so a wall-clock step
/// cannot overlap it.
fn position_after_wake(elapsed_ms: u64, last_end_ms: u64, away_ms: u64) -> u64 {
    elapsed_ms.max(last_end_ms + away_ms)
}

/// Mark the session as restarting and hand out its budget. A restart stops
/// the capture watcher; the reopened segment starts it again when it needs
/// one (KTD3).
fn begin_restart(inner: &mut Inner, id: &str) -> (CancellationToken, RestartTracker) {
    inner.stop_capture_watch();
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
    if let Some(live) = active.live {
        live.end(LiveEnd::Discard);
    }
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
        let input = |name: &str| audio_cpal::Input {
            id: format!("id-{name}"),
            name: name.to_string(),
        };
        Inputs {
            default: default.map(input),
            all: all.iter().map(|name| input(name)).collect(),
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

    const PARAKEET: &str = "parakeet-unified-en-0.6b-q80";
    const WHISPER: &str = "whisper-large-v3-turbo-q5km";

    fn selected(model: &str, live_transcript: bool) -> Settings {
        Settings {
            asr_model: model.into(),
            live_transcript,
            ..Settings::default()
        }
    }

    #[test]
    fn a_streaming_model_with_the_switch_on_records_live_mode() {
        assert_eq!(
            recording_model(&selected(PARAKEET, true)),
            RecordingModel {
                asr_model: PARAKEET.into(),
                live_transcript: true,
            }
        );
    }

    #[test]
    fn the_switch_off_or_a_model_that_does_not_stream_records_no_live_mode() {
        assert!(!recording_model(&selected(PARAKEET, false)).live_transcript);
        let whisper = recording_model(&selected(WHISPER, true));
        assert_eq!(whisper.asr_model, WHISPER);
        assert!(!whisper.live_transcript);
    }

    #[test]
    fn a_model_that_is_not_in_the_catalog_records_no_live_mode() {
        assert!(!recording_model(&selected("no-such-model", true)).live_transcript);
    }

    #[test]
    fn a_resume_keeps_the_sessions_model_when_it_is_downloaded() {
        let settings = selected(WHISPER, true);
        assert_eq!(model_at_resume(Some(PARAKEET), &settings, |_| true), None);
    }

    #[test]
    fn a_resume_records_the_selected_model_when_the_sessions_model_is_gone() {
        let settings = selected(PARAKEET, true);
        assert_eq!(
            model_at_resume(Some(WHISPER), &settings, |id| id != WHISPER),
            Some(RecordingModel {
                asr_model: PARAKEET.into(),
                live_transcript: true,
            })
        );
    }

    fn usb() -> MicrophoneChoice {
        MicrophoneChoice {
            id: "id-USB mic".into(),
            label: "USB mic".into(),
        }
    }

    fn headset() -> MicrophoneChoice {
        MicrophoneChoice {
            id: "id-Headset".into(),
            label: "Headset".into(),
        }
    }

    fn cpal_choice(choice: &MicrophoneChoice) -> MicChoice {
        MicChoice::from(choice)
    }

    fn with_microphone(microphone: Option<MicrophoneChoice>, computer_audio: bool) -> Settings {
        Settings {
            microphone,
            computer_audio_default: computer_audio,
            ..Settings::default()
        }
    }

    #[test]
    fn a_note_starts_with_the_capture_choice_from_settings() {
        let choice = capture_choice_at_start(None, &with_microphone(Some(usb()), true));
        assert_eq!(
            choice,
            CaptureChoice {
                microphone: Some(Microphone::from(&usb())),
                computer_audio: true,
            }
        );
        let default = capture_choice_at_start(None, &Settings::default());
        assert_eq!(default, CaptureChoice::default());
    }

    #[test]
    fn a_microphone_chosen_before_the_row_exists_is_in_the_row() {
        // KTD2: held in the capture manager and written with the row.
        let settings = with_microphone(Some(usb()), false);
        let held = capture_choice_at_start(Some(Some(headset())), &settings);
        assert_eq!(held.microphone, Some(Microphone::from(&headset())));
        // "System default" held on the recording page wins over Settings.
        let default = capture_choice_at_start(Some(None), &settings);
        assert_eq!(default.microphone, None);
        // Settings does not change.
        assert_eq!(settings.microphone, Some(usb()));
    }

    #[test]
    fn a_missing_chosen_microphone_opens_the_default_with_the_fallback_flag() {
        // AE4: the note starts at once on the default.
        let mut tried = Vec::new();
        let opened = with_fallback(Some(&cpal_choice(&usb())), |target| {
            tried.push(target.cloned());
            match target {
                Some(_) => Err(audio::Error::NoInputDevice),
                None => Ok("Built-in"),
            }
        });
        assert_eq!(opened, Ok(("Built-in", true)));
        assert_eq!(tried, [Some(cpal_choice(&usb())), None]);
    }

    #[test]
    fn a_present_chosen_microphone_opens_with_no_fallback_and_no_poll_wait() {
        // After a wake or a resume, the chosen microphone opens at once when
        // it is present (KTD3).
        let opened = with_fallback(Some(&cpal_choice(&usb())), |target| match target {
            Some(c) => Ok(c.label.clone()),
            None => Ok("Built-in".to_string()),
        });
        assert_eq!(opened, Ok(("USB mic".to_string(), false)));
    }

    #[test]
    fn a_failure_other_than_a_missing_input_does_not_fall_back() {
        let mut calls = 0;
        let opened: Result<((), bool), _> = with_fallback(Some(&cpal_choice(&usb())), |_| {
            calls += 1;
            Err(audio::Error::MicStreamInitializationFailed("busy".into()))
        });
        assert!(matches!(
            opened,
            Err(audio::Error::MicStreamInitializationFailed(_))
        ));
        assert_eq!(calls, 1);
    }

    #[test]
    fn no_input_at_all_is_no_input_device() {
        let opened: Result<((), bool), _> = with_fallback(Some(&cpal_choice(&usb())), |_| {
            Err(audio::Error::NoInputDevice)
        });
        assert_eq!(opened, Err(audio::Error::NoInputDevice));
        let mut calls = 0;
        let default: Result<((), bool), _> = with_fallback(None, |_| {
            calls += 1;
            Err(audio::Error::NoInputDevice)
        });
        assert_eq!(default, Err(audio::Error::NoInputDevice));
        assert_eq!(calls, 1, "System default has nothing to fall back to");
    }

    #[test]
    fn a_chosen_microphone_lost_mid_note_falls_back_at_once() {
        // AE5: no restart backoff.
        let now = inputs(Some("Built-in"), &["Built-in"]);
        assert_eq!(
            after_loss(Some(&cpal_choice(&usb())), false, &now),
            AfterLoss::FallBackNow
        );
    }

    #[test]
    fn a_stream_error_with_the_chosen_microphone_present_restarts_on_it() {
        let now = inputs(Some("Built-in"), &["Built-in", "USB mic"]);
        assert_eq!(
            after_loss(Some(&cpal_choice(&usb())), false, &now),
            AfterLoss::Restart
        );
    }

    #[test]
    fn a_lost_fallback_input_uses_the_restart_budget() {
        // A default that keeps failing must not reopen in a loop.
        let now = inputs(Some("Built-in"), &["Built-in"]);
        assert_eq!(
            after_loss(Some(&cpal_choice(&usb())), true, &now),
            AfterLoss::Restart
        );
    }

    #[test]
    fn a_loss_with_no_input_left_takes_todays_path() {
        // The restart budget runs out, and the note is Interrupted(device).
        let none = inputs(None, &[]);
        assert_eq!(
            after_loss(Some(&cpal_choice(&usb())), false, &none),
            AfterLoss::Restart
        );
        assert_eq!(after_loss(None, false, &none), AfterLoss::Restart);
        let default = inputs(Some("Built-in"), &["Built-in"]);
        assert_eq!(after_loss(None, false, &default), AfterLoss::Restart);
    }

    #[test]
    fn the_chosen_microphone_is_found_by_id_then_by_name() {
        let now = inputs(Some("Built-in"), &["Built-in", "USB mic"]);
        assert!(chosen_present(Some(&cpal_choice(&usb())), &now));
        let renamed_id = MicChoice {
            id: "other-uid".into(),
            label: "USB mic".into(),
        };
        assert!(chosen_present(Some(&renamed_id), &now));
        assert!(!chosen_present(Some(&cpal_choice(&headset())), &now));
        assert!(!chosen_present(None, &now));
    }

    #[test]
    fn the_move_back_needs_two_polls_in_a_row() {
        let mut move_back = MoveBack::default();
        assert!(!move_back.poll(true), "one poll is not enough");
        assert!(!move_back.poll(false));
        assert!(!move_back.poll(true));
        assert!(move_back.poll(true), "two polls in a row move back");
        // After a move, the count starts again.
        assert!(!move_back.poll(true));
    }

    fn active(id: &str, fallback: bool) -> ActiveSegment {
        let (_tx, outcome) = std_mpsc::channel();
        ActiveSegment {
            session_id: id.into(),
            segment: 0,
            stop: CancellationToken::new(),
            outcome,
            thread: std::thread::spawn(|| {}),
            live: None,
            input: "Built-in".into(),
            fallback,
            opened_at_switch: false,
        }
    }

    #[test]
    fn a_change_reopens_only_while_the_note_records() {
        let mut inner = Inner::default();
        // Interrupted: nothing records.
        assert!(!change_reopens(&inner, "s1"));
        inner.active = Some(active("s1", false));
        assert!(change_reopens(&inner, "s1"));
        assert!(!change_reopens(&inner, "s2"), "another note records");
        // Restart backoff: the next segment reads the stored choice.
        inner.active = None;
        inner.recovering = Some(Pending::new("s1"));
        assert!(!change_reopens(&inner, "s1"));
    }

    #[test]
    fn stop_sleep_supersede_and_restart_cancel_the_capture_watch() {
        let mut inner = Inner::default();
        let watch = Pending::new("s1");
        let cancel = watch.cancel.clone();
        inner.capture_watch = Some(watch);
        // Stop and a new session supersede.
        inner.supersede();
        assert!(cancel.is_cancelled());
        assert!(inner.capture_watch.is_none());

        let watch = Pending::new("s1");
        let cancel = watch.cancel.clone();
        inner.capture_watch = Some(watch);
        inner.stop_capture_watch();
        assert!(cancel.is_cancelled(), "sleep stops the watch");

        let watch = Pending::new("s1");
        let cancel = watch.cancel.clone();
        inner.capture_watch = Some(watch);
        let _ = begin_restart(&mut inner, "s1");
        assert!(cancel.is_cancelled(), "a restart stops the watch");
    }

    #[test]
    fn the_capture_watch_waits_in_a_restart_and_ends_without_a_fallback() {
        let mut inner = Inner::default();
        assert_eq!(watch_step(&inner, "s1"), WatchStep::End);
        inner.active = Some(active("s1", false));
        assert_eq!(watch_step(&inner, "s1"), WatchStep::End);
        inner.active = Some(active("s1", true));
        assert_eq!(watch_step(&inner, "s1"), WatchStep::MoveBack);
        assert_eq!(watch_step(&inner, "s2"), WatchStep::End);
        inner.recovering = Some(Pending::new("s1"));
        assert_eq!(watch_step(&inner, "s1"), WatchStep::Wait);
    }

    #[test]
    fn a_segment_at_a_switch_keeps_its_short_audio() {
        // A change followed by Stop within 1 s keeps the short segment.
        let mut timeline = Timeline::new();
        timeline.open(0, at(0)).unwrap();
        let closed = close_timeline(&mut timeline, 400, at(0), "a".into(), true).unwrap();
        assert!(matches!(closed, Closed::Kept(_)));
        // A short segment with no switch is dropped, as today (R35).
        timeline.open(5_000, at(5)).unwrap();
        let closed = close_timeline(&mut timeline, 5_400, at(5), "b".into(), false).unwrap();
        assert_eq!(closed, Closed::Discarded);
    }

    #[test]
    fn a_resume_reads_the_stored_choice_not_settings() {
        // A user resume and a crash resume both read the store (KTD2).
        let mut store = Store::open_in_memory().unwrap();
        let stored = CaptureChoice {
            microphone: Some(Microphone::from(&usb())),
            computer_audio: false,
        };
        let model = recording_model(&Settings::default());
        store
            .start_session("s1", at(0), None, &model, &stored)
            .unwrap();
        let _settings = with_microphone(Some(headset()), false);
        let choice = store.capture_choice("s1").unwrap();
        assert_eq!(mic_of(&choice), Some(cpal_choice(&usb())));
    }

    #[test]
    fn a_note_from_an_older_build_resumes_on_the_system_default() {
        let mut store = Store::open_in_memory().unwrap();
        store.create_session("s1", at(0), None).unwrap();
        assert_eq!(mic_of(&store.capture_choice("s1").unwrap()), None);
    }

    #[test]
    fn a_microphone_choice_converts_both_ways() {
        let stored = Microphone::from(&usb());
        assert_eq!(MicrophoneChoice::from(&stored), usb());
        assert_eq!(
            mic_of(&CaptureChoice {
                microphone: Some(stored),
                computer_audio: false,
            }),
            Some(MicChoice {
                id: "id-USB mic".into(),
                label: "USB mic".into(),
            })
        );
    }

    #[test]
    fn a_blank_microphone_choice_is_refused() {
        assert!(check_microphone(None).is_ok());
        assert!(check_microphone(Some(&usb())).is_ok());
        let blank = MicrophoneChoice {
            id: " ".into(),
            label: "USB mic".into(),
        };
        assert!(check_microphone(Some(&blank)).is_err());
    }

    #[test]
    fn a_resume_records_the_selected_model_when_the_session_has_none() {
        let settings = selected(WHISPER, true);
        assert_eq!(
            model_at_resume(None, &settings, |_| true),
            Some(RecordingModel {
                asr_model: WHISPER.into(),
                live_transcript: false,
            })
        );
    }
}
