//! System audio capture: the audio of the apps in the note's app set
//! ([`open_apps`]), or all audio that the computer plays ([`open`]).
//!
//! - On macOS, a Core Audio process tap through `cidre`: a tap of the app
//!   set's processes, or a global tap that excludes Longhand's own process,
//!   so the alert sound is not in the note.
//! - On Windows, one process loopback client for each app in the set, or
//!   WASAPI loopback on the output device that anarlog's `EndpointFollower`
//!   picks.
//!
//! Both backends are borrowed from anarlog (see PROVENANCE.md). Each one
//! runs on its own thread, and [`open`] returns a [`CaptureStream`] of
//! [`CaptureFrame`]s whose `raw_speaker` holds `chunk_size` mono samples at
//! the rate that the caller asks for. `raw_mic` is empty. A backend error
//! ends this stream with [`Error::SpeakerStreamEnded`]. The microphone
//! stream is a different stream, so the error never ends it (U4 step 5).

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod convert;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod follow;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod rate;

pub mod apps;
#[cfg(target_os = "macos")]
mod macos;
pub mod permission;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub mod windows_app_audio;
#[cfg(target_os = "windows")]
mod windows_apps;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod windows_mix;
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod windows_tree;

use std::sync::{Arc, Mutex};

pub use audio::{CaptureFrame, CaptureStream, Error};
pub use permission::Permission;

/// Whether this platform has a system audio backend.
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// Frames queued between the backend thread and the consumer. At 20 ms per
/// frame this is about 1.3 s.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const FRAME_QUEUE: usize = 64;

/// A running system audio capture.
pub struct SystemAudioCapture {
    /// The frames. Drop the stream to stop the capture.
    pub stream: CaptureStream,
}

/// The IDs of the apps whose audio a capture records: the note's app set.
/// Clones share one set, so the capture manager can change it while the
/// stream runs (R10).
#[derive(Debug, Clone, Default)]
pub struct AppSet(Arc<Mutex<Vec<String>>>);

impl AppSet {
    pub fn new(ids: Vec<String>) -> Self {
        Self(Arc::new(Mutex::new(ids)))
    }

    /// Replace the set. The backend reads it at its next check.
    pub fn set(&self, ids: Vec<String>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = ids;
    }

    pub fn get(&self) -> Vec<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Start capture of all computer sound except Longhand's own. Frames hold
/// `chunk_size` mono samples at `sample_rate`; a `sample_rate` of 0 means
/// the backend's own rate. The permission request uses it (KTD11).
pub fn open(sample_rate: u32, chunk_size: usize) -> Result<SystemAudioCapture, Error> {
    let chunk_size = chunk_size.max(1);
    #[cfg(target_os = "macos")]
    {
        open_macos(sample_rate, chunk_size, macos::Target::Global)
    }
    #[cfg(target_os = "windows")]
    {
        let (tx, rx) = tokio::sync::mpsc::channel(FRAME_QUEUE);
        windows::open(sample_rate, chunk_size, tx)?;
        Ok(SystemAudioCapture {
            stream: CaptureStream::new(tokio_stream::wrappers::ReceiverStream::new(rx)),
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (sample_rate, chunk_size);
        Err(Error::SpeakerStreamSetupFailed)
    }
}

/// Start capture of the audio of the apps in `apps` (R6-R10). Frames are as
/// in [`open`]. While no app in the set plays, the stream sends no frames
/// and no error. Change the set with [`AppSet::set`]. On a Windows build
/// before 20348 ([`apps::supported`] is false), it returns
/// [`Error::SpeakerStreamSetupFailed`] (R17).
pub fn open_apps(
    sample_rate: u32,
    chunk_size: usize,
    apps: AppSet,
) -> Result<SystemAudioCapture, Error> {
    let chunk_size = chunk_size.max(1);
    #[cfg(target_os = "macos")]
    {
        open_macos(sample_rate, chunk_size, macos::Target::Apps(apps))
    }
    #[cfg(target_os = "windows")]
    {
        // R17: per-process loopback needs Windows build 20348 or later.
        if !apps::supported() {
            log::warn!(
                "app audio needs Windows build {} or later",
                apps::FIRST_WINDOWS_BUILD
            );
            return Err(Error::SpeakerStreamSetupFailed);
        }
        let (tx, rx) = tokio::sync::mpsc::channel(FRAME_QUEUE);
        windows_app_audio::open(sample_rate, chunk_size, apps, tx)?;
        Ok(SystemAudioCapture {
            stream: CaptureStream::new(tokio_stream::wrappers::ReceiverStream::new(rx)),
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (sample_rate, chunk_size, apps);
        Err(Error::SpeakerStreamSetupFailed)
    }
}

#[cfg(target_os = "macos")]
fn open_macos(
    sample_rate: u32,
    chunk_size: usize,
    target: macos::Target,
) -> Result<SystemAudioCapture, Error> {
    let (tx, rx) = tokio::sync::mpsc::channel(FRAME_QUEUE);
    macos::open(sample_rate, chunk_size, target, tx)?;
    Ok(SystemAudioCapture {
        stream: CaptureStream::new(tokio_stream::wrappers::ReceiverStream::new(rx)),
    })
}

/// Check that the platform can capture system audio: a backend exists and
/// an output device is present. It opens no stream and asks for no
/// permission.
pub fn probe() -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        macos::probe()
    }
    #[cfg(target_os = "windows")]
    {
        windows::probe()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err(Error::SpeakerStreamSetupFailed)
    }
}

/// One frame of system audio.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn speaker_frame(samples: &[f32]) -> CaptureFrame {
    CaptureFrame {
        raw_mic: Arc::from([] as [f32; 0]),
        raw_speaker: Arc::from(samples),
        aec_mic: None,
    }
}

/// The processes that the macOS tap excludes: Longhand's own process, found
/// by `lookup` from its PID. `lookup` returns `Ok(None)` when the audio
/// system knows no process for the PID. When the lookup fails, the tap
/// excludes nothing, and a log line records it (KTD4).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn own_process_exclusion<T>(
    pid: u32,
    lookup: impl FnOnce(u32) -> Result<Option<T>, String>,
) -> Vec<T> {
    match lookup(pid) {
        Ok(Some(process)) => vec![process],
        Ok(None) => {
            log::warn!(
                "the audio system has no process object for PID {pid}; \
                 the tap excludes no process, so Longhand's own sounds are in system audio"
            );
            Vec::new()
        }
        Err(e) => {
            log::warn!(
                "the process object lookup for PID {pid} failed: {e}; \
                 the tap excludes no process, so Longhand's own sounds are in system audio"
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    struct Capture(Mutex<Vec<String>>);

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{} {}", record.level(), record.args()));
        }
        fn flush(&self) {}
    }

    fn logs() -> &'static Capture {
        static LOGGER: OnceLock<&'static Capture> = OnceLock::new();
        LOGGER.get_or_init(|| {
            let logger: &'static Capture = Box::leak(Box::new(Capture(Mutex::new(Vec::new()))));
            log::set_logger(logger).expect("no other logger in this test binary");
            log::set_max_level(log::LevelFilter::Info);
            logger
        })
    }

    fn lines_about(pid: u32) -> Vec<String> {
        let needle = format!("PID {pid}");
        logs()
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains(&needle))
            .cloned()
            .collect()
    }

    #[test]
    fn a_found_process_is_excluded() {
        logs();
        let excluded = own_process_exclusion(4_101, |pid| Ok(Some(pid * 10)));
        assert_eq!(excluded, [41_010]);
        assert!(lines_about(4_101).is_empty());
    }

    #[test]
    fn a_failed_pid_lookup_excludes_nothing_and_logs_it() {
        logs();
        let excluded: Vec<u32> = own_process_exclusion(4_102, |_| Err("'who?'".into()));
        assert!(excluded.is_empty());
        let lines = lines_about(4_102);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("WARN"), "{lines:?}");
        assert!(lines[0].contains("excludes no process"), "{lines:?}");
    }

    #[test]
    fn an_unknown_pid_excludes_nothing_and_logs_it() {
        logs();
        let excluded: Vec<u32> = own_process_exclusion(4_103, |_| Ok(None));
        assert!(excluded.is_empty());
        assert_eq!(lines_about(4_103).len(), 1);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn a_platform_without_a_backend_captures_no_system_audio() {
        assert!(!SUPPORTED);
        assert!(matches!(
            open(16_000, 320),
            Err(Error::SpeakerStreamSetupFailed)
        ));
        assert!(matches!(probe(), Err(Error::SpeakerStreamSetupFailed)));
    }
}
