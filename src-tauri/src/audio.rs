//! The app's [`AudioProvider`]: the microphone from `audio-cpal` and system
//! audio from `audio-system` (KTD4). `audio-cpal` stays microphone-only, so
//! the two backends meet here and neither crate depends on the other.

use std::sync::mpsc as std_mpsc;

use audio::{AudioProvider, CaptureConfig, CaptureStream, Error};
use audio_cpal::CpalAudio;
use audio_system::SystemAudioCapture;

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

    /// System audio with the status that the frames do not show, such as
    /// the Windows other-output flag (U4 step 6).
    pub fn open_system_audio(
        &self,
        sample_rate: u32,
        chunk_size: usize,
    ) -> Result<SystemAudioCapture, Error> {
        audio_system::open(sample_rate, chunk_size)
    }
}

impl AudioProvider for AppAudio {
    /// The microphone only. U6 pairs it with system audio.
    fn open_capture(&self, config: CaptureConfig) -> Result<CaptureStream, Error> {
        self.mic
            .open_mic_capture(config.mic_device, config.sample_rate, config.chunk_size)
    }

    fn open_speaker_capture(
        &self,
        sample_rate: u32,
        chunk_size: usize,
    ) -> Result<CaptureStream, Error> {
        self.open_system_audio(sample_rate, chunk_size)
            .map(|capture| capture.stream)
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

    /// Moved from `audio-cpal`, which no longer has a speaker half: the
    /// speaker half of the provider is `audio-system`.
    #[test]
    fn the_speaker_is_captured_only_through_audio_system() {
        let audio = AppAudio::new();
        assert_eq!(audio.probe_speaker(), audio_system::probe());
        if !audio_system::SUPPORTED {
            assert!(matches!(
                audio.open_speaker_capture(16_000, 320),
                Err(Error::SpeakerStreamSetupFailed)
            ));
        }
    }
}
