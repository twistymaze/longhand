//! System audio capture: all audio that the computer plays (R7, KTD4).
//!
//! - On macOS, a global Core Audio process tap through `cidre`. The tap
//!   excludes Longhand's own process, so the alert sound is not in the note.
//! - On Windows, WASAPI loopback on the output device that anarlog's
//!   `EndpointFollower` picks.
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
pub mod silence;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
mod windows_apps;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub use audio::{CaptureFrame, CaptureStream, Error};
pub use permission::Permission;
pub use silence::{SilenceRun, SilentCheck};

/// Whether this platform has a system audio backend.
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// Frames queued between the backend thread and the consumer. At 20 ms per
/// frame this is about 1.3 s.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
const FRAME_QUEUE: usize = 64;

/// What the backend knows about the capture that the frames do not show.
/// The capture manager reads it at each tick (U6), and the recording page
/// shows its warnings (U7).
#[derive(Debug, Clone, Default)]
pub struct SystemAudioStatus {
    other_output: Arc<AtomicBool>,
}

impl SystemAudioStatus {
    /// True when the followed output has been silent for 30 s while another
    /// output plays sound (Windows only, U4 step 6). The call can then play
    /// on a device that Longhand does not record. Always false on macOS,
    /// where the tap records every output.
    pub fn other_output_playing(&self) -> bool {
        self.other_output.load(Ordering::Relaxed)
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn set_other_output(&self, value: bool) {
        let before = self.other_output.swap(value, Ordering::Relaxed);
        if before != value {
            if value {
                log::warn!(
                    "the followed output is silent while another output plays; \
                     system audio can miss the call"
                );
            } else {
                log::info!("the other-output flag cleared");
            }
        }
    }
}

/// A running system audio capture.
pub struct SystemAudioCapture {
    /// The frames. Drop the stream to stop the capture.
    pub stream: CaptureStream,
    pub status: SystemAudioStatus,
}

/// Start system audio capture. Frames hold `chunk_size` mono samples at
/// `sample_rate`; a `sample_rate` of 0 means the backend's own rate.
pub fn open(sample_rate: u32, chunk_size: usize) -> Result<SystemAudioCapture, Error> {
    let chunk_size = chunk_size.max(1);
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::mpsc::channel(FRAME_QUEUE);
        macos::open(sample_rate, chunk_size, tx)?;
        Ok(SystemAudioCapture {
            stream: CaptureStream::new(tokio_stream::wrappers::ReceiverStream::new(rx)),
            status: SystemAudioStatus::default(),
        })
    }
    #[cfg(target_os = "windows")]
    {
        let (tx, rx) = tokio::sync::mpsc::channel(FRAME_QUEUE);
        let status = SystemAudioStatus::default();
        windows::open(sample_rate, chunk_size, tx, status.clone())?;
        Ok(SystemAudioCapture {
            stream: CaptureStream::new(tokio_stream::wrappers::ReceiverStream::new(rx)),
            status,
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (sample_rate, chunk_size);
        Err(Error::SpeakerStreamSetupFailed)
    }
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

/// Whether the default output device plays audio for some process now
/// (`kAudioDevicePropertyDeviceIsRunningSomewhere`, U5 step 6). `None`
/// when the device cannot be read, and on the platforms that do not need
/// the check: only an unknown macOS permission needs it.
pub fn output_running() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        macos::output_running()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
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

/// Apps that play the audio of other apps again: audio mixers and routing
/// tools. A global tap records each app's own output, so the audio that a
/// mixer plays again is a second copy, often late, and a mixer can also
/// play the microphone to a virtual device. The macOS tap excludes these
/// apps. Bundle IDs that start with an entry of [`MIXER_BUNDLE_PREFIXES`]
/// match too.
pub const MIXER_BUNDLE_IDS: &[&str] = &[
    // Elgato Wave Link 3 (verified on a Mac with Wave Link 3) and earlier.
    "com.elgato.WaveLink3",
    "com.elgato.WaveLink",
    // OBS Studio plays monitored sources to an output.
    "com.obsproject.obs-studio",
];

/// Bundle ID prefixes of mixer vendors: Rogue Amoeba (Loopback,
/// SoundSource, Audio Hijack, and their helpers).
pub const MIXER_BUNDLE_PREFIXES: &[&str] = &["com.rogueamoeba."];

/// True when the app with this bundle ID plays the audio of other apps
/// again. The test ignores case.
pub fn is_mixer(bundle_id: &str) -> bool {
    let id = bundle_id.to_ascii_lowercase();
    MIXER_BUNDLE_IDS.iter().any(|m| m.eq_ignore_ascii_case(&id))
        || MIXER_BUNDLE_PREFIXES
            .iter()
            .any(|p| id.starts_with(&p.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    #[test]
    fn mixer_apps_match_by_bundle_id() {
        assert!(is_mixer("com.elgato.WaveLink3"));
        assert!(is_mixer("COM.ELGATO.WAVELINK3"));
        assert!(is_mixer("com.rogueamoeba.Loopback"));
        assert!(is_mixer("com.rogueamoeba.soundsource"));
        assert!(!is_mixer("com.microsoft.teams2"));
        assert!(!is_mixer("us.zoom.xos"));
        assert!(!is_mixer("com.elgato.WaveLink3Helper.other"));
        assert!(!is_mixer(""));
    }

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

    #[test]
    fn the_status_starts_clear_and_is_shared_between_clones() {
        let status = SystemAudioStatus::default();
        let reader = status.clone();
        assert!(!reader.other_output_playing());
        status.set_other_output(true);
        assert!(reader.other_output_playing());
        status.set_other_output(false);
        assert!(!reader.other_output_playing());
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
