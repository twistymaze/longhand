//! The app's [`AudioProvider`]: the microphone from `audio-cpal` and app
//! audio from `audio-system` (KTD4). `audio-cpal` stays microphone-only, so
//! the two backends meet here and neither crate depends on the other.

use std::sync::mpsc as std_mpsc;

use audio::{AudioProvider, CaptureConfig, CaptureStream, Error};
use audio_cpal::CpalAudio;
use audio_system::AppSet;

#[derive(Default)]
pub struct AppAudio {
    mic: CpalAudio,
}

impl AppAudio {
    pub fn new() -> Self {
        Self::default()
    }

    /// The microphone backend, for the calls that only it has:
    /// `prepare`, `forget` and `inputs`.
    pub fn mic(&self) -> &CpalAudio {
        &self.mic
    }

    /// The audio of the apps in `apps` (R6). Change the set with
    /// [`AppSet::set`] while the stream runs.
    pub fn open_apps(
        &self,
        sample_rate: u32,
        chunk_size: usize,
        apps: AppSet,
    ) -> Result<CaptureStream, Error> {
        audio_system::open_apps(sample_rate, chunk_size, apps).map(|capture| capture.stream)
    }
}

impl AudioProvider for AppAudio {
    /// The microphone only. The capture manager opens app audio with
    /// [`AppAudio::open_apps`].
    fn open_capture(&self, config: CaptureConfig) -> Result<CaptureStream, Error> {
        self.mic
            .open_mic_capture(config.mic_device, config.sample_rate, config.chunk_size)
    }

    /// Longhand records no capture of all computer audio (R18): the app
    /// audio of a note comes from [`AppAudio::open_apps`].
    fn open_speaker_capture(
        &self,
        _sample_rate: u32,
        _chunk_size: usize,
    ) -> Result<CaptureStream, Error> {
        Err(Error::SpeakerStreamSetupFailed)
    }

    fn open_mic_capture(
        &self,
        device: Option<String>,
        sample_rate: u32,
        chunk_size: usize,
    ) -> Result<CaptureStream, Error> {
        self.mic.open_mic_capture(device, sample_rate, chunk_size)
    }

    fn default_device_name(&self) -> String {
        self.mic.default_device_name()
    }

    fn list_mic_devices(&self) -> Vec<String> {
        self.mic.list_mic_devices()
    }

    /// Cues are played elsewhere (rodio, U7); nothing listens on this sender.
    fn play_silence(&self) -> std_mpsc::Sender<()> {
        std_mpsc::channel().0
    }

    fn play_bytes(&self, _bytes: &'static [u8]) -> std_mpsc::Sender<()> {
        std_mpsc::channel().0
    }

    fn probe_mic(&self, device: Option<String>) -> Result<(), Error> {
        self.mic.probe_mic(device)
    }

    fn probe_speaker(&self) -> Result<(), Error> {
        audio_system::probe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_capture_of_all_computer_audio_opens() {
        // R18: the parked whole-computer capture is gone.
        let audio = AppAudio::new();
        assert_eq!(audio.probe_speaker(), audio_system::probe());
        assert!(matches!(
            audio.open_speaker_capture(16_000, 320),
            Err(Error::SpeakerStreamSetupFailed)
        ));
    }
}
