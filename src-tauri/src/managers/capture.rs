//! Owns capture: the microphone, the segment being recorded, and the
//! hand-off of segment open and close to the session core (KTD3, KTD4, KTD9).
//!
//! Each segment records on a thread of its own, which is also where the
//! idle-sleep guard will live (U22): the guard must be created and dropped by
//! the thread that owns active capture. A crash leaves the open segment's
//! chunks on disk under their names; those names, not the job store, are
//! what launch recovery rebuilds the segment from (R8).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::Instant;

use audio::AudioProvider;
use audio_cpal::CpalAudio;
use chrono::{DateTime, FixedOffset, Local};
use recorder::storage::DISK_RESERVE_BYTES;
use recorder::{ChunkedWriter, SegmentArgs, SegmentEnd, SegmentOutcome};
use serde::Serialize;
use session_core::{Closed, Event, SessionState, SpeakerCount, Store, Timeline};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event as _;
use tokio_util::sync::CancellationToken;

/// Frames of 20 ms: small enough for a responsive level meter (U7), large
/// enough to keep per-frame overhead negligible.
const FRAMES_PER_SECOND: u32 = 50;

pub const RECOVERY_DIR: &str = "audio-recovery";

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
    #[error("{0}")]
    Other(String),
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
}

#[derive(Default)]
struct Inner {
    active: Option<ActiveSegment>,
    clock: Option<Clock>,
    sample_rate: Option<u32>,
}

pub struct CaptureManager {
    audio: CpalAudio,
    working_root: PathBuf,
    store: Mutex<Store>,
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
            store: Mutex::new(store),
            inner: Mutex::new(Inner::default()),
        })
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
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

    pub fn is_capturing(&self) -> bool {
        self.inner().active.is_some()
    }

    /// Start a new session recording (R1, R6c). A session already recording
    /// is closed first and then stopped and queued by the store.
    pub fn start_session(
        &self,
        app: &AppHandle,
        speaker_count: Option<SpeakerCount>,
    ) -> Result<String, CaptureError> {
        let mut inner = self.inner();
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
            log::info!("session {other} stopped by a new session");
        }
        inner.active = Some(active);
        inner.clock = Some(Clock {
            session_id: id.clone(),
            timeline,
            epoch: Instant::now(),
        });
        log::info!("session {id} recording");
        Ok(id)
    }

    /// Stop the recording session (R6b, R35). Returns its state afterwards:
    /// queued for processing, or discarded when it kept no audio.
    pub fn stop_session(&self) -> Result<(String, SessionState), CaptureError> {
        let mut inner = self.inner();
        let active = inner.active.take().ok_or(CaptureError::NotCapturing)?;
        let id = active.session_id.clone();
        self.close_segment(&mut inner, active)?;
        let retained_audio = !self.store().segments(&id)?.is_empty();
        let state = self.store().apply(
            &id,
            &Event::UserStop {
                at: now(),
                retained_audio,
            },
        )?;
        if !retained_audio {
            // Nothing survived R35; the notice is the surface's (U7).
            let _ = std::fs::remove_dir_all(self.session_dir(&id));
            log::info!("session {id} kept no audio and was discarded");
        }
        Ok((id, state))
    }

    /// Resume an interrupted session as its next segment (R6). The store
    /// decides whether the resume is allowed (window, most recent, space).
    pub fn resume_session(&self, app: &AppHandle, id: &str) -> Result<SessionState, CaptureError> {
        let mut inner = self.inner();
        if inner.active.is_some() {
            return Err(CaptureError::AlreadyCapturing);
        }
        let space_ok = recorder::storage::check(
            recorder::storage::available_space(&self.working_root),
            DISK_RESERVE_BYTES,
        )
        .is_ok();
        let resumed_at = now();
        let is_most_recent = self.store().is_most_recent(id)?;
        let state = self.store().apply(
            id,
            &Event::UserResume {
                now: resumed_at,
                is_most_recent,
                space_ok,
            },
        )?;
        if !matches!(state, SessionState::Capturing) {
            return Ok(state);
        }
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
        inner.clock = Some(Clock {
            session_id: id.to_string(),
            timeline,
            epoch: epoch.unwrap_or_else(|| {
                let now = Instant::now();
                now.checked_sub(std::time::Duration::from_millis(offset))
                    .unwrap_or(now)
            }),
        });
        log::info!("session {id} resumed as segment {index} at {offset} ms");
        Ok(state)
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
                let result = runtime.block_on(recorder::run_segment(stream, writer, thread_stop));
                let reason = match &result.end {
                    SegmentEnd::Stopped => None,
                    SegmentEnd::SourceLost(e) => Some(format!("microphone: {e}")),
                    SegmentEnd::Storage(e) => Some(e.to_string()),
                };
                if let Some(reason) = reason {
                    log::warn!("segment {segment} of {thread_session} ended: {reason}");
                    let _ = CaptureEnded {
                        session_id: thread_session,
                        reason,
                    }
                    .emit(&thread_app);
                }
                let _ = tx.send(result);
            })
            .map_err(|e| CaptureError::Other(e.to_string()))?;
        Ok(ActiveSegment {
            session_id: session_id.to_string(),
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

    #[test]
    fn segment_audio_files_number_from_one() {
        assert_eq!(segment_audio_file(0), "audio/segment-01.opus");
        assert_eq!(segment_audio_file(11), "audio/segment-12.opus");
    }
}
